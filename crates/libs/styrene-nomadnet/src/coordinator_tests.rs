use std::sync::Mutex;

use rns_core::identity::PrivateIdentity;

use super::*;
use crate::{PageFormField, PageFormFieldKind, PageFormSubmission};

const HOST: &str = "0123456789abcdef0123456789abcdef";

struct ScriptedBackend {
    identity: Option<Identity>,
    local_identity: Mutex<Option<Arc<PrivateIdentity>>>,
    outcome: Mutex<VecDeque<NativeRequestOutcome>>,
    calls: Mutex<Vec<&'static str>>,
    path_delay: Duration,
    identity_delay: Duration,
    link_delay: Duration,
    request_delay: Duration,
    path_permits: Option<Arc<tokio::sync::Semaphore>>,
    identified_as: Mutex<Vec<AddressHash>>,
    requested_data: Mutex<Vec<Vec<u8>>>,
    next_link: AtomicU64,
    link_created: bool,
    close_failures: std::sync::atomic::AtomicUsize,
}

impl ScriptedBackend {
    fn success(transfer: RequestResponseTransfer, source: &[u8]) -> Self {
        let mut started = RequestObservationInfo::default();
        started.request_id = "11".repeat(16);
        started.link_id = "22".repeat(16);
        started.request_size = 24;
        started.state = RequestState::Pending;

        let mut encoded = Vec::new();
        rmpv::encode::write_value(&mut encoded, &rmpv::Value::Binary(source.to_vec()))
            .expect("encode response");
        let mut completed = started.clone();
        completed.state = RequestState::Succeeded;
        completed.response_transfer = transfer;
        completed.response = Some(encoded);
        completed.response_size = Some(source.len().try_into().unwrap_or(u64::MAX));
        completed.received_bytes = source.len().try_into().unwrap_or(u64::MAX);
        completed.total_bytes = completed.received_bytes;
        completed.progress = 1.0;
        completed.rtt_ms = Some(19);
        if transfer == RequestResponseTransfer::Resource {
            completed.resource_hash = Some("33".repeat(32));
        }

        Self {
            identity: Some(*PrivateIdentity::new_from_name("native-browser-peer").as_identity()),
            outcome: Mutex::new(VecDeque::from([NativeRequestOutcome { started, completed }])),
            calls: Mutex::new(Vec::new()),
            local_identity: Mutex::new(None),
            path_delay: Duration::ZERO,
            identity_delay: Duration::ZERO,
            link_delay: Duration::ZERO,
            request_delay: Duration::ZERO,
            path_permits: None,
            identified_as: Mutex::new(Vec::new()),
            requested_data: Mutex::new(Vec::new()),
            next_link: AtomicU64::new(1),
            link_created: true,
            close_failures: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn identity_failure() -> Self {
        Self {
            identity: None,
            outcome: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            local_identity: Mutex::new(None),
            path_delay: Duration::ZERO,
            identity_delay: Duration::ZERO,
            link_delay: Duration::ZERO,
            request_delay: Duration::ZERO,
            path_permits: None,
            identified_as: Mutex::new(Vec::new()),
            requested_data: Mutex::new(Vec::new()),
            next_link: AtomicU64::new(1),
            link_created: true,
            close_failures: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl BrowseBackend for ScriptedBackend {
    fn identification_enabled(&self) -> bool {
        self.local_identity.lock().unwrap().is_some()
    }
    async fn discover_path(
        &self,
        _destination: AddressHash,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<(), BrowseError> {
        self.calls.lock().unwrap().push("path");
        if let Some(permits) = &self.path_permits {
            let permit = permits.acquire().await.map_err(|_| BrowseError::Cancelled)?;
            permit.forget();
        }
        tokio::select! {
            () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
            () = tokio::time::sleep(self.path_delay) => {}
        }
        remaining(deadline)?;
        Ok(())
    }

    async fn resolve_identity(
        &self,
        _destination: AddressHash,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<Identity, BrowseError> {
        self.calls.lock().unwrap().push("identity");
        tokio::select! {
            () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
            () = tokio::time::sleep(self.identity_delay) => {}
        }
        remaining(deadline)?;
        self.identity.ok_or_else(|| BrowseError::Transport("identity unavailable".into()))
    }

    async fn open_link(
        &self,
        _destination: DestinationDesc,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<BrowserLink, BrowseError> {
        self.calls.lock().unwrap().push("link");
        tokio::select! {
            () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
            () = tokio::time::sleep(self.link_delay) => {}
        }
        remaining(deadline)?;
        Ok(BrowserLink {
            id: format!("{:032x}", self.next_link.fetch_add(1, Ordering::Relaxed)),
            created: self.link_created,
        })
    }

    async fn identify_link(
        &self,
        _link_id: &str,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<(), BrowseError> {
        self.calls.lock().unwrap().push("identify");
        if cancellation.is_cancelled() {
            return Err(BrowseError::Cancelled);
        }
        let identity = self.local_identity.lock().unwrap().clone().expect("selected identity");
        self.identified_as.lock().unwrap().push(*identity.address_hash());
        remaining(deadline)?;
        Ok(())
    }

    async fn request(&self, request: NativeRequest) -> Result<NativeRequestOutcome, BrowseError> {
        let NativeRequest { correlation_id, data, cancellation, progress, deadline, .. } = request;
        self.calls.lock().unwrap().push("request");
        self.requested_data.lock().unwrap().push(data);
        tokio::select! {
            () = cancellation.cancelled() => {
                return Err(BrowseError::Transport("request cancelled".into()));
            }
            () = tokio::time::sleep(self.request_delay) => {}
        }
        remaining(deadline)?;
        let mut outcome =
            self.outcome.lock().unwrap().pop_front().ok_or(BrowseError::MissingReceipt)?;
        outcome.started.observation.correlation_id = Some(correlation_id.clone());
        outcome.completed.observation.correlation_id = Some(correlation_id);
        if let Some(progress) = progress {
            let mut receiving = outcome.started.clone();
            receiving.state = RequestState::Receiving;
            receiving.received_bytes = outcome.completed.received_bytes / 2;
            receiving.total_bytes = outcome.completed.total_bytes;
            receiving.progress = 0.5;
            receiving.response_transfer = outcome.completed.response_transfer;
            progress(receiving);
        }
        Ok(outcome)
    }

    async fn close_link(&self, _link_id: &str) -> Result<(), BrowseError> {
        self.calls.lock().unwrap().push("close");
        if self
            .close_failures
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                if remaining > 0 { Some(remaining - 1) } else { None }
            })
            .is_ok()
        {
            return Err(BrowseError::Transport("scripted close failure".into()));
        }
        Ok(())
    }
}

fn coordinator(backend: Arc<ScriptedBackend>) -> NativeNomadNetBrowseCoordinator {
    let discovery: Arc<dyn Discovery> = Arc::new(|_: &str| Some(true));
    NativeNomadNetBrowseCoordinator::with_backend(backend, discovery)
}

fn identified_coordinator(backend: Arc<ScriptedBackend>) -> NativeNomadNetBrowseCoordinator {
    *backend.local_identity.lock().unwrap() =
        Some(Arc::new(PrivateIdentity::new_from_name("selected-local-reader")));
    coordinator(backend)
}

#[tokio::test]
async fn deterministic_success_uses_one_correlation_and_preserves_source_projection() {
    let source = b">Index\nHello `[next`next.mu`]\n`<name`value>`";
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, source));
    let coordinator = coordinator(backend.clone());

    let result = coordinator.browse_remote(HOST, "/page/index.mu", Duration::from_secs(1)).await;

    assert_eq!(
        backend.calls.lock().unwrap().as_slice(),
        ["path", "identity", "link", "request", "close"]
    );
    assert_eq!(result.source_bytes, source);
    assert_eq!(result.source_checksum, hex::encode(Sha256::digest(source)));
    assert_eq!(result.title.as_deref(), Some("Index"));
    assert_eq!(result.links, ["next.mu"]);
    assert_eq!(result.request.native_path, "/page/index.mu");
    assert_eq!(result.cache.status, PageCacheStatus::NotUsed);
    assert_eq!(result.cache.stored_at, None);
    assert_eq!(result.fields.len(), 1);
    assert_eq!(result.transfer.kind, PageTransferKind::Packet);
    assert_eq!(result.outcome, PageBrowseOutcome::Succeeded);
    assert!(result.failure.is_none());
    assert!(result.started_unix_ms.is_some());
    assert!(result.completed_unix_ms.is_some());
    assert!(result.elapsed_ms.is_some());
    assert_eq!(result.observation.correlation_id.as_deref(), Some(result.correlation_id.as_str()));
    assert!(result.stages.iter().all(|stage| stage.correlation_id == result.correlation_id));
    assert!(result.stages.iter().all(|stage| {
        stage.observation.correlation_id.as_deref() == Some(result.correlation_id.as_str())
            && stage.observation.observed_at.is_some()
    }));
    assert_eq!(result.stages[0].evidence_source, Some(ObservationSource::TransportPathTable));
    assert_eq!(result.stages[4].request_id, Some("11".repeat(16)));
    assert!(result.stages.iter().enumerate().all(|(index, stage)| {
        index == 3 && matches!(stage.state, PageBrowseStageState::Skipped { .. })
            || index != 3 && stage.state == PageBrowseStageState::Succeeded
    }));
}

#[tokio::test]
async fn identity_failure_blocks_all_later_stages_without_fabricated_success() {
    let backend = Arc::new(ScriptedBackend::identity_failure());
    let coordinator = coordinator(backend.clone());

    let result = coordinator.browse_remote(HOST, "/page/index.mu", Duration::from_secs(1)).await;

    assert_eq!(backend.calls.lock().unwrap().as_slice(), ["path", "identity"]);
    assert_eq!(result.stages[0].state, PageBrowseStageState::Succeeded);
    assert!(matches!(result.stages[1].state, PageBrowseStageState::Failed { .. }));
    assert!(
        result
            .stages
            .iter()
            .skip(2)
            .all(|stage| { matches!(stage.state, PageBrowseStageState::Skipped { .. }) })
    );
    assert!(result.source_bytes.is_empty());
    assert_eq!(result.outcome, PageBrowseOutcome::Failed);
    assert_eq!(
        result.failure.as_ref().map(|failure| failure.stage),
        Some(PageBrowseStageKind::IdentityResolution)
    );
}

#[tokio::test]
async fn resource_receipt_drives_verified_resource_transfer_metadata() {
    let source = vec![b'x'; 4096];
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Resource, &source));
    let coordinator = coordinator(backend);

    let result = coordinator.browse_remote(HOST, "/page/large.mu", Duration::from_secs(1)).await;

    assert_eq!(result.transfer.kind, PageTransferKind::Resource);
    assert_eq!(result.transfer.resource_hash, Some("33".repeat(32)));
    assert!(result.transfer.verified);
    assert_eq!(result.transfer.received_bytes, 4096);
    assert_eq!(result.source_bytes, source);
    assert_eq!(result.request.request_id, Some("11".repeat(16)));
}

#[tokio::test]
async fn file_download_rejects_unannounced_host_before_transport() {
    let backend =
        Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b"not reached"));
    let coordinator =
        NativeNomadNetBrowseCoordinator::with_backend(backend.clone(), Arc::new(|_: &str| None));
    let cancellation = tokio_util::sync::CancellationToken::new();

    let error = coordinator
        .download_native(
            1,
            HOST,
            "/file/manual.bin",
            "file-correlation",
            tokio::time::Instant::now() + Duration::from_secs(1),
            cancellation,
        )
        .await
        .expect_err("unannounced host must fail closed");

    assert!(error.to_string().contains("no native NomadNet announce"));
    assert!(backend.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn selected_identity_authenticates_link_and_correlation_reaches_request_observations() {
    let backend = Arc::new(ScriptedBackend::success(
        RequestResponseTransfer::Packet,
        b">Allowed\nauthenticated",
    ));
    let coordinator = identified_coordinator(backend.clone());

    let result = coordinator.browse_remote(HOST, "/page/allowed.mu", Duration::from_secs(1)).await;

    assert_eq!(
        backend.calls.lock().unwrap().as_slice(),
        ["path", "identity", "link", "identify", "request", "close"]
    );
    assert_eq!(result.stages[3].state, PageBrowseStageState::Succeeded);
    assert_eq!(
        backend.identified_as.lock().unwrap().as_slice(),
        [*PrivateIdentity::new_from_name("selected-local-reader").address_hash()]
    );
    assert!(result.transfer.verified);
}

#[tokio::test]
async fn identified_denial_is_terminal_and_blocks_parse_and_render() {
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::None, b""));
    {
        let mut outcome = backend.outcome.lock().unwrap();
        let request = outcome.front_mut().expect("scripted request");
        request.completed.state = RequestState::TimedOut;
        request.completed.response = None;
    }
    let coordinator = identified_coordinator(backend);

    let result = coordinator.browse_remote(HOST, "/page/private.mu", Duration::from_secs(1)).await;

    assert_eq!(result.stages[3].state, PageBrowseStageState::Succeeded);
    assert!(matches!(result.stages[5].state, PageBrowseStageState::Failed { .. }));
    assert!(
        result
            .stages
            .iter()
            .skip(6)
            .all(|stage| matches!(stage.state, PageBrowseStageState::Skipped { .. }))
    );
    assert_eq!(result.outcome, PageBrowseOutcome::TimedOut);
    assert_eq!(
        result.failure.as_ref().map(|failure| failure.code.as_str()),
        Some("request_timed_out")
    );
}

#[tokio::test]
async fn one_absolute_deadline_is_not_restarted_between_stages() {
    let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b">late");
    scripted.path_delay = Duration::from_millis(20);
    let backend = Arc::new(scripted);
    let coordinator = coordinator(backend.clone());

    let result = coordinator.browse_remote(HOST, "/page/late.mu", Duration::from_millis(5)).await;

    assert_eq!(backend.calls.lock().unwrap().as_slice(), ["path"]);
    assert!(matches!(result.stages[0].state, PageBrowseStageState::Failed { .. }));
    assert!(
        result
            .stages
            .iter()
            .skip(1)
            .all(|stage| { matches!(stage.state, PageBrowseStageState::Skipped { .. }) })
    );
    assert_eq!(result.outcome, PageBrowseOutcome::TimedOut);
}

#[tokio::test]
async fn oversized_source_is_rejected_before_parse() {
    let source = vec![b'x'; MAX_PAGE_SOURCE_SIZE + 1];
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Resource, &source));
    let coordinator = coordinator(backend);

    let result = coordinator.browse_remote(HOST, "/page/oversize.mu", Duration::from_secs(1)).await;

    assert!(matches!(
        &result.stages[5].state,
        PageBrowseStageState::Failed { code, .. } if code == "response_too_large"
    ));
    assert!(result.source_bytes.is_empty());
}

#[tokio::test]
async fn daemon_history_cache_and_reload_are_deterministic() {
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b"unused"));
    let coordinator = coordinator(backend);
    let loads = std::sync::atomic::AtomicUsize::new(0);
    let mut navigate = PageNavigationRequest::default();
    navigate.target = Some("/page/index.mu".into());
    let first = coordinator
        .navigate(navigate, HOST, |path| {
            loads.fetch_add(1, Ordering::Relaxed);
            format!(">{path}").into_bytes()
        })
        .await
        .expect("first page");
    let session_id = first.navigation.session_id.clone();

    let mut next = PageNavigationRequest::default();
    next.session_id = Some(session_id.clone());
    next.target = Some("docs.mu".into());
    let second = coordinator
        .navigate(next, HOST, |path| {
            loads.fetch_add(1, Ordering::Relaxed);
            format!(">{path}").into_bytes()
        })
        .await
        .expect("second page");
    assert_eq!(second.navigation.history_len, 2);

    let mut back = PageNavigationRequest::default();
    back.session_id = Some(session_id.clone());
    back.action = PageNavigationAction::Back;
    let previous = coordinator
        .navigate(back, HOST, |_| panic!("back must use the daemon cache"))
        .await
        .expect("back");
    assert_eq!(previous.cache.status, PageCacheStatus::Hit);
    assert_ne!(previous.correlation_id, first.correlation_id);
    assert_eq!(
        previous.cache.origin_correlation_id.as_deref(),
        Some(first.correlation_id.as_str())
    );
    assert_eq!(previous.transfer.kind, PageTransferKind::Cache);
    assert_eq!(previous.outcome, PageBrowseOutcome::Succeeded);
    assert!(previous.stages.iter().all(|stage| {
        matches!(stage.state, PageBrowseStageState::Skipped { .. })
            && stage.observation.correlation_id.as_deref() == Some(previous.correlation_id.as_str())
    }));
    assert_eq!(previous.navigation.history_len, 2);
    assert!(previous.navigation.can_forward);

    let mut reload = PageNavigationRequest::default();
    reload.session_id = Some(session_id.clone());
    reload.action = PageNavigationAction::Reload;
    let reloaded = coordinator
        .navigate(reload, HOST, |path| {
            loads.fetch_add(1, Ordering::Relaxed);
            format!(">reloaded {path}").into_bytes()
        })
        .await
        .expect("reload");
    assert_eq!(reloaded.cache.status, PageCacheStatus::Bypassed);
    assert_eq!(reloaded.navigation.history_len, 2);
    assert_eq!(loads.load(Ordering::Relaxed), 3);

    let closed = coordinator.close_session(&session_id).await.expect("close session");
    assert_eq!(closed.history_len, 2);
    assert!(!closed.connection_open);
}

#[test]
fn form_projection_redacts_passwords_and_submission_is_native_messagepack() {
    let mut page = initial_result(HOST, "/page/form.mu", "form", PageCacheStatus::NotUsed);
    finish_projection(
        &mut page,
        b"`<name`Ada> `<12!|password`secret> `[Submit`next.mu`name|password]".to_vec(),
    );
    assert_eq!(page.fields.len(), 2);
    assert_eq!(page.fields[0].value.as_deref(), Some("Ada"));
    assert_eq!(page.fields[1].kind, PageFormFieldKind::Password);
    assert_eq!(page.fields[1].value, None);
    assert_eq!(page.link_targets[0].submitted_fields, ["name", "password"]);

    let mut submission = PageFormSubmission::default();
    submission.values.insert("name".into(), vec!["Grace".into()]);
    submission.values.insert("password".into(), vec!["swordfish".into()]);
    submission.values.insert("opts".into(), vec!["blue".into(), "red".into()]);
    assert!(!format!("{submission:?}").contains("swordfish"));
    let mut checkbox_red = PageFormField::default();
    checkbox_red.name = "opts".into();
    checkbox_red.kind = PageFormFieldKind::Checkbox;
    checkbox_red.value = Some("red".into());
    let mut checkbox_blue = checkbox_red.clone();
    checkbox_blue.value = Some("blue".into());
    let encoded = encode_submission(
        Some(&submission),
        &[page.fields[0].clone(), page.fields[1].clone(), checkbox_red, checkbox_blue],
        &["mode=safe".into(), "*".into()],
    )
    .expect("native map");
    assert_eq!(
        hex::encode(&encoded),
        "84a87661725f6d6f6465a473616665aa6669656c645f6e616d65a54772616365ae6669656c645f70617373776f7264a973776f726466697368aa6669656c645f6f707473a87265642c626c7565"
    );
    let decoded = rmpv::decode::read_value(&mut std::io::Cursor::new(encoded)).unwrap();
    assert!(matches!(decoded, rmpv::Value::Map(values) if values.len() == 4));
}

#[test]
fn explicit_submission_without_link_directive_sends_named_fields() {
    let mut submission = PageFormSubmission::default();
    submission.values.insert("name".into(), vec!["rust".into()]);
    submission.values.insert("opts".into(), vec!["red".into(), "blue".into()]);
    let encoded = encode_submission(Some(&submission), &[], &[]).expect("native map");
    let decoded = rmpv::decode::read_value(&mut std::io::Cursor::new(encoded)).unwrap();
    let rmpv::Value::Map(values) = decoded else { panic!("map") };
    assert_eq!(
        values,
        vec![
            (rmpv::Value::from("field_name"), rmpv::Value::from("rust")),
            (rmpv::Value::from("field_opts"), rmpv::Value::from("red,blue")),
        ]
    );
    assert_eq!(encode_submission(None, &[], &[]).expect("nil"), vec![0xc0]);
}

#[tokio::test]
async fn submitted_fields_reach_native_request_and_close_does_not_navigate() {
    let source = b"`<name`Ada> `<12!|password`> `[Submit`next.mu`mode=safe|name|password]";
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, source));
    let duplicate = backend.outcome.lock().unwrap().front().cloned().unwrap();
    backend.outcome.lock().unwrap().push_back(duplicate);
    let coordinator = coordinator(backend.clone());
    let mut initial = PageNavigationRequest::default();
    initial.target = Some(format!("{HOST}:/page/form.mu"));
    let first = coordinator.navigate(initial, HOST, |_| Vec::new()).await.expect("form page");
    let mut submission = PageFormSubmission::default();
    submission.values.insert("name".into(), vec!["Ada".into()]);
    submission.values.insert("password".into(), vec!["secret".into()]);
    let mut request = PageNavigationRequest::default();
    request.session_id = Some(first.navigation.session_id.clone());
    request.target = Some("next.mu".into());
    request.submission = Some(submission);
    let page = coordinator.navigate(request, HOST, |_| Vec::new()).await.expect("submit");
    let encoded = backend.requested_data.lock().unwrap()[1].clone();
    assert!(matches!(
        rmpv::decode::read_value(&mut std::io::Cursor::new(encoded)).unwrap(),
        rmpv::Value::Map(values) if values.len() == 3
    ));
    let calls_before_close = backend.calls.lock().unwrap().len();
    let closed = coordinator.close_session(&page.navigation.session_id).await.expect("close");
    assert_eq!(closed.history_len, 2);
    assert_eq!(&backend.calls.lock().unwrap()[calls_before_close..], ["close"]);
}

#[tokio::test]
async fn resource_request_reports_deterministic_progress_before_completion() {
    let backend = ScriptedBackend::success(RequestResponseTransfer::Resource, b"resource");
    let observations = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&observations);
    backend
        .request(NativeRequest {
            link_id: "22".repeat(16),
            path: "/file/resource.bin".into(),
            correlation_id: "download-progress".into(),
            data: vec![0xc0],
            max_response_size: MAX_ENCODED_FILE_RESPONSE_SIZE,
            cancellation: tokio_util::sync::CancellationToken::new(),
            progress: Some(Arc::new(move |receipt| captured.lock().unwrap().push(receipt))),
            deadline: tokio::time::Instant::now() + Duration::from_secs(1),
        })
        .await
        .expect("resource request");
    let observations = observations.lock().unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].state, RequestState::Receiving);
    assert_eq!(observations[0].progress, 0.5);
}

#[tokio::test]
async fn file_download_verifies_integrity_and_requires_explicit_save() {
    let bytes = b"verified file bytes";
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Resource, bytes));
    let coordinator = Arc::new(coordinator(backend.clone()));
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("saved.bin");
    let mut request = FileDownloadRequest::default();
    request.target = format!("{HOST}:/file/archive.bin");
    request.expected_sha256 = Some(hex::encode(Sha256::digest(bytes)));
    let started = coordinator.start_download(request).await.expect("start download");
    let completed = loop {
        let current = coordinator.download(&started.download_id).await.unwrap();
        if current.state.is_terminal() {
            break current;
        }
        tokio::task::yield_now().await;
    };
    assert_eq!(completed.state, FileDownloadState::Completed);
    assert!(completed.integrity_verified);
    assert!(!destination.exists());
    assert!(
        coordinator
            .save_download(&started.download_id, Path::new("saved.bin"))
            .await
            .unwrap_err()
            .contains("absolute path")
    );
    let existing = root.path().join("existing.bin");
    std::fs::write(&existing, b"operator data").unwrap();
    assert!(coordinator.save_download(&started.download_id, &existing).await.is_err());
    assert_eq!(std::fs::read(&existing).unwrap(), b"operator data");
    let missing_parent = root.path().join("missing").join("saved.bin");
    assert!(coordinator.save_download(&started.download_id, &missing_parent).await.is_err());
    assert!(!missing_parent.exists());
    let saved =
        coordinator.save_download(&started.download_id, &destination).await.expect("explicit save");
    assert_eq!(saved.state, FileDownloadState::Saved);
    assert_eq!(std::fs::read(destination).unwrap(), bytes);
    assert_eq!(
        backend.calls.lock().unwrap().iter().filter(|call| **call == "close").count(),
        1,
        "download link leaked or closed more than once"
    );
}

#[tokio::test]
async fn file_download_cancellation_returns_completed_cancelled_state() {
    let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Resource, b"late");
    scripted.request_delay = Duration::from_secs(60);
    let coordinator = Arc::new(coordinator(Arc::new(scripted)));
    let mut request = FileDownloadRequest::default();
    request.target = format!("{HOST}:/file/late.bin");
    let started = coordinator.start_download(request).await.expect("start download");
    tokio::task::yield_now().await;
    let cancelled = coordinator.cancel_download(&started.download_id).await.expect("cancel");
    assert_eq!(cancelled.state, FileDownloadState::Cancelled);
    assert!(cancelled.error.is_some());
}

#[tokio::test]
async fn session_capacity_evicts_terminal_then_lru_and_never_active_work() {
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b"unused"));
    let bounded = coordinator(backend.clone());
    {
        let mut sessions = bounded.sessions.lock().unwrap();
        for index in 0..MAX_SESSIONS {
            let mut session = BrowseSession::new(0, index as u64 + 1);
            session.active = index == 0;
            session.terminal = index == 1;
            session.link = Some(BrowserLink { id: format!("{index:032x}"), created: true });
            sessions.insert(format!("session-{index}"), session);
        }
    }
    let mut request = PageNavigationRequest::default();
    request.target = Some("/page/new.mu".into());
    bounded
        .navigate(request, HOST, |_| b">new".to_vec())
        .await
        .expect("terminal session is evicted");
    {
        let sessions = bounded.sessions.lock().unwrap();
        assert!(sessions.contains_key("session-0"), "active work was evicted");
        assert!(!sessions.contains_key("session-1"), "terminal session was retained");
    }
    assert!(backend.calls.lock().unwrap().contains(&"close"));

    let coordinator =
        coordinator(Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b"unused")));
    {
        let mut sessions = coordinator.sessions.lock().unwrap();
        for index in 0..MAX_SESSIONS {
            let mut session = BrowseSession::new(0, index as u64);
            session.active = true;
            sessions.insert(format!("active-{index}"), session);
        }
    }
    let mut request = PageNavigationRequest::default();
    request.target = Some("/page/rejected.mu".into());
    assert_eq!(
        coordinator.navigate(request, HOST, |_| Vec::new()).await.unwrap_err(),
        "page session capacity is full"
    );
    assert_eq!(coordinator.sessions.lock().unwrap().len(), MAX_SESSIONS);
}

#[tokio::test]
async fn full_capacity_invalid_submission_does_not_evict_sessions_or_links() {
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">new"));
    let coordinator = coordinator(backend.clone());
    {
        let mut sessions = coordinator.sessions.lock().unwrap();
        for index in 0..MAX_SESSIONS {
            let mut session = BrowseSession::new(0, index as u64);
            session.terminal = true;
            session.link = Some(BrowserLink { id: format!("{index:032x}"), created: true });
            sessions.insert(format!("preserved-{index}"), session);
        }
    }
    let before = coordinator
        .sessions
        .lock()
        .unwrap()
        .iter()
        .map(|(id, session)| (id.clone(), session.link.clone()))
        .collect::<HashMap<_, _>>();
    let mut submission = PageFormSubmission::default();
    submission.values.insert(String::new(), vec!["invalid".into()]);
    let mut request = PageNavigationRequest::default();
    request.session_id = Some("replacement".into());
    request.target = Some(format!("{HOST}:/page/new.mu"));
    request.submission = Some(submission);

    assert_eq!(
        coordinator.navigate(request, HOST, |_| Vec::new()).await.unwrap_err(),
        "submitted field state exceeds its bound"
    );
    let after = coordinator
        .sessions
        .lock()
        .unwrap()
        .iter()
        .map(|(id, session)| (id.clone(), session.link.clone()))
        .collect::<HashMap<_, _>>();
    assert_eq!(after, before);
    assert!(!backend.calls.lock().unwrap().contains(&"close"));
}

#[tokio::test]
async fn concurrent_session_reservations_are_atomic_and_bounded() {
    let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b">race");
    let outcome = scripted.outcome.lock().unwrap().front().cloned().unwrap();
    scripted.outcome.lock().unwrap().extend(std::iter::repeat_n(outcome, MAX_SESSIONS - 1));
    let permits = Arc::new(tokio::sync::Semaphore::new(0));
    scripted.path_permits = Some(Arc::clone(&permits));
    let backend = Arc::new(scripted);
    let coordinator = Arc::new(coordinator(backend.clone()));
    let callers = MAX_SESSIONS * 2;
    let start = Arc::new(tokio::sync::Barrier::new(callers + 1));
    let mut tasks = Vec::new();
    for index in 0..callers {
        let coordinator = Arc::clone(&coordinator);
        let start = Arc::clone(&start);
        tasks.push(tokio::spawn(async move {
            start.wait().await;
            let mut request = PageNavigationRequest::default();
            request.session_id = Some(format!("race-{index}"));
            request.target = Some(format!("{HOST}:/page/race.mu"));
            coordinator.navigate(request, HOST, |_| Vec::new()).await
        }));
    }
    start.wait().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let calls =
                backend.calls.lock().unwrap().iter().filter(|call| **call == "path").count();
            let rejected = tasks.iter().filter(|task| task.is_finished()).count();
            if calls == MAX_SESSIONS && rejected == MAX_SESSIONS {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("all capacity contenders reached a deterministic state");
    {
        let sessions = coordinator.sessions.lock().unwrap();
        assert_eq!(sessions.len(), MAX_SESSIONS);
        assert!(sessions.values().all(|session| session.active));
    }
    permits.add_permits(MAX_SESSIONS);
    let mut succeeded = 0;
    let mut rejected = 0;
    for task in tasks {
        match task.await.expect("navigation task") {
            Ok(_) => succeeded += 1,
            Err(error) if error == "page session capacity is full" => rejected += 1,
            Err(error) => panic!("unexpected navigation error: {error}"),
        }
    }
    assert_eq!(succeeded, MAX_SESSIONS);
    assert_eq!(rejected, MAX_SESSIONS);
    assert!(coordinator.sessions.lock().unwrap().len() <= MAX_SESSIONS);
}

#[tokio::test]
async fn concurrent_navigation_of_one_session_is_rejected() {
    let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b">first");
    let permits = Arc::new(tokio::sync::Semaphore::new(0));
    scripted.path_permits = Some(Arc::clone(&permits));
    let backend = Arc::new(scripted);
    let coordinator = Arc::new(coordinator(backend.clone()));
    let first_coordinator = Arc::clone(&coordinator);
    let first = tokio::spawn(async move {
        let mut request = PageNavigationRequest::default();
        request.session_id = Some("shared".into());
        request.target = Some(format!("{HOST}:/page/index.mu"));
        first_coordinator.navigate(request, HOST, |_| Vec::new()).await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !backend.calls.lock().unwrap().contains(&"path") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first navigation acquired the session");
    let mut concurrent = PageNavigationRequest::default();
    concurrent.session_id = Some("shared".into());
    concurrent.target = Some(format!("{HOST}:/page/other.mu"));
    assert_eq!(
        coordinator.navigate(concurrent, HOST, |_| Vec::new()).await.unwrap_err(),
        "page session already has active work"
    );
    permits.add_permits(1);
    first.await.expect("first task").expect("first navigation");
}

#[tokio::test]
async fn close_removes_session_and_superseded_links_are_retired() {
    let source = b"`[Next`next.mu]";
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, source));
    let duplicate = backend.outcome.lock().unwrap().front().cloned().unwrap();
    backend.outcome.lock().unwrap().push_back(duplicate);
    let coordinator = coordinator(backend.clone());
    let mut first = PageNavigationRequest::default();
    first.target = Some(format!("{HOST}:/page/index.mu"));
    let first = coordinator.navigate(first, HOST, |_| Vec::new()).await.unwrap();
    let mut next = PageNavigationRequest::default();
    next.session_id = Some(first.navigation.session_id.clone());
    next.target = Some("next.mu".into());
    let next = coordinator.navigate(next, HOST, |_| Vec::new()).await.unwrap();
    coordinator.close_session(&next.navigation.session_id).await.unwrap();
    assert!(!coordinator.sessions.lock().unwrap().contains_key(&next.navigation.session_id));
    assert_eq!(
        backend.calls.lock().unwrap().iter().filter(|call| **call == "close").count(),
        2,
        "superseded and current links must each be retired exactly once"
    );
}

#[tokio::test]
async fn failed_close_preserves_the_session_for_retry() {
    let backend =
        Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">still open"));
    let coordinator = coordinator(backend.clone());
    let mut request = PageNavigationRequest::default();
    request.target = Some(format!("{HOST}:/page/index.mu"));
    let page = coordinator.navigate(request, HOST, |_| Vec::new()).await.unwrap();
    let link_id = coordinator
        .sessions
        .lock()
        .unwrap()
        .get(&page.navigation.session_id)
        .and_then(|session| session.link.as_ref())
        .map(|link| link.id.clone())
        .unwrap();
    backend.close_failures.store(MAX_LINK_CLEANUP_ATTEMPTS.into(), Ordering::Relaxed);

    assert!(coordinator.close_session(&page.navigation.session_id).await.is_err());
    {
        let sessions = coordinator.sessions.lock().unwrap();
        let session = sessions.get(&page.navigation.session_id).expect("session retained");
        assert!(!session.active);
        assert_eq!(session.current.as_ref().unwrap().navigation.address, page.navigation.address);
    }
    assert_eq!(
        coordinator.cleanup.status(&link_id).await,
        Some(LinkCleanupStatus::TerminalError {
            attempts: MAX_LINK_CLEANUP_ATTEMPTS,
            error: "scripted close failure".into(),
        })
    );
    assert_eq!(
        backend.calls.lock().unwrap().iter().filter(|call| **call == "close").count(),
        usize::from(MAX_LINK_CLEANUP_ATTEMPTS)
    );
}

#[tokio::test]
async fn owner_cleanup_retains_terminal_link_ownership_until_recovery() {
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">owned"));
    let coordinator = coordinator(backend.clone());
    let mut request = PageNavigationRequest::default();
    request.target = Some(format!("{HOST}:/page/owned.mu"));
    let page = coordinator.navigate_for_owner(42, request, HOST, |_| Vec::new()).await.unwrap();
    let link_id = coordinator
        .sessions
        .lock()
        .unwrap()
        .get(&page.navigation.session_id)
        .and_then(|session| session.link.as_ref())
        .map(|link| link.id.clone())
        .unwrap();
    backend.close_failures.store(MAX_LINK_CLEANUP_ATTEMPTS.into(), Ordering::Relaxed);

    let terminal = coordinator.cleanup_owner(42).await.unwrap_err();

    assert!(terminal.contains("cleanup failed after 3 attempts"));
    assert!(!coordinator.sessions.lock().unwrap().contains_key(&page.navigation.session_id));
    assert!(
        coordinator
            .owner_cleanup
            .lock()
            .unwrap()
            .get(&42)
            .is_some_and(|links| links.contains_key(&link_id))
    );
    assert!(matches!(
        coordinator.cleanup.status(&link_id).await,
        Some(LinkCleanupStatus::TerminalError { attempts: 3, .. })
    ));

    coordinator.cleanup_owner(42).await.expect("retained owner cleanup recovers");

    assert!(coordinator.owner_cleanup.lock().unwrap().get(&42).is_none_or(HashMap::is_empty));
    assert_eq!(
        coordinator.cleanup.status(&link_id).await,
        Some(LinkCleanupStatus::Completed { attempts: 1 })
    );
}

#[tokio::test]
async fn reused_browser_link_is_never_closed() {
    let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b">reused");
    scripted.link_created = false;
    let backend = Arc::new(scripted);
    let coordinator = coordinator(backend.clone());

    coordinator.browse_remote(HOST, "/page/reused.mu", Duration::from_secs(1)).await;

    assert!(!backend.calls.lock().unwrap().contains(&"close"));
}

#[tokio::test]
async fn committed_navigation_supervises_transient_old_link_cleanup() {
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">first"));
    let duplicate = backend.outcome.lock().unwrap().front().cloned().unwrap();
    backend.outcome.lock().unwrap().push_back(duplicate);
    let coordinator = coordinator(backend.clone());
    let mut first = PageNavigationRequest::default();
    first.target = Some(format!("{HOST}:/page/first.mu"));
    let first = coordinator.navigate(first, HOST, |_| Vec::new()).await.unwrap();
    backend.close_failures.store(1, Ordering::Relaxed);
    let mut second = PageNavigationRequest::default();
    second.session_id = Some(first.navigation.session_id.clone());
    second.target = Some(format!("{HOST}:/page/second.mu"));

    let second = coordinator.navigate(second, HOST, |_| Vec::new()).await.unwrap();

    assert_eq!(second.navigation.address, format!("{HOST}:/page/second.mu"));
    let cleaned_link = format!("{:032x}", 1);
    assert_eq!(
        coordinator.cleanup.status(&cleaned_link).await,
        Some(LinkCleanupStatus::Completed { attempts: 2 })
    );
    {
        let sessions = coordinator.sessions.lock().unwrap();
        assert_eq!(
            sessions[&first.navigation.session_id].current.as_ref().unwrap().navigation.address,
            second.navigation.address
        );
    }
}

#[tokio::test]
async fn failed_eviction_close_rolls_back_session_ownership() {
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">new"));
    let coordinator = coordinator(backend.clone());
    {
        let mut sessions = coordinator.sessions.lock().unwrap();
        for index in 0..MAX_SESSIONS {
            let mut session = BrowseSession::new(0, index as u64);
            session.terminal = true;
            session.link = Some(BrowserLink { id: format!("{index:032x}"), created: true });
            sessions.insert(format!("old-{index}"), session);
        }
    }
    backend.close_failures.store(MAX_LINK_CLEANUP_ATTEMPTS.into(), Ordering::Relaxed);
    let mut request = PageNavigationRequest::default();
    request.session_id = Some("replacement".into());
    request.target = Some(format!("{HOST}:/page/new.mu"));

    assert!(coordinator.navigate(request, HOST, |_| Vec::new()).await.is_err());

    let sessions = coordinator.sessions.lock().unwrap();
    assert_eq!(sessions.len(), MAX_SESSIONS);
    assert!(!sessions.contains_key("replacement"));
    assert_eq!(sessions.values().filter(|session| session.link.is_some()).count(), MAX_SESSIONS);
}

#[tokio::test]
async fn submitted_response_is_not_reused_as_ordinary_url_cache() {
    let backend =
        Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">personalized"));
    let duplicate = backend.outcome.lock().unwrap().front().cloned().unwrap();
    backend.outcome.lock().unwrap().push_back(duplicate);
    let coordinator = coordinator(backend.clone());
    let mut submitted = PageNavigationRequest::default();
    submitted.target = Some(format!("{HOST}:/page/dynamic.mu"));
    submitted.submission = Some(PageFormSubmission::default());
    coordinator.navigate(submitted, HOST, |_| Vec::new()).await.unwrap();
    let mut ordinary = PageNavigationRequest::default();
    ordinary.target = Some(format!("{HOST}:/page/dynamic.mu"));

    let ordinary = coordinator.navigate(ordinary, HOST, |_| Vec::new()).await.unwrap();

    assert_eq!(ordinary.cache.status, PageCacheStatus::Miss);
    assert_eq!(backend.requested_data.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn owner_scope_blocks_known_session_and_download_ids() {
    let coordinator = Arc::new(coordinator(Arc::new(ScriptedBackend::success(
        RequestResponseTransfer::Packet,
        b"unused",
    ))));
    let mut request = PageNavigationRequest::default();
    request.target = Some("/page/local.mu".into());
    let page =
        coordinator.navigate_for_owner(7, request, HOST, |_| b">local".to_vec()).await.unwrap();
    assert!(coordinator.close_session_for_owner(8, &page.navigation.session_id).await.is_err());
    let mut download = FileDownloadRequest::default();
    download.session_id = Some(page.navigation.session_id.clone());
    download.target = "/file/data.bin".into();
    assert!(coordinator.start_download_for_owner(8, download).await.is_err());
}

#[tokio::test]
async fn saving_reservation_blocks_duplicate_save_and_eviction() {
    let coordinator = Arc::new(coordinator(Arc::new(ScriptedBackend::success(
        RequestResponseTransfer::Packet,
        b"unused",
    ))));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *coordinator.save_gate.lock().await = Some(Arc::clone(&gate));
    let mut info = FileDownloadInfo::default();
    info.download_id = "saving".into();
    info.state = FileDownloadState::Completed;
    info.integrity_verified = true;
    coordinator.downloads.lock().await.insert(
        info.download_id.clone(),
        DownloadRecord {
            owner: 0,
            info,
            bytes: Some(b"atomic bytes".to_vec()),
            saving: false,
            cancellation: tokio_util::sync::CancellationToken::new(),
            completion: tokio::sync::watch::channel::<u64>(0).0,
            last_used: 0,
        },
    );
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("saved.bin");
    let duplicate = root.path().join("duplicate.bin");
    let saving = {
        let coordinator = Arc::clone(&coordinator);
        let destination = destination.clone();
        tokio::spawn(async move { coordinator.save_download("saving", &destination).await })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if coordinator.downloads.lock().await["saving"].saving {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("save reservation");
    assert!(coordinator.save_download("saving", &duplicate).await.is_err());
    {
        let mut downloads = coordinator.downloads.lock().await;
        for index in 1..MAX_DOWNLOADS {
            let mut info = FileDownloadInfo::default();
            info.download_id = format!("active-{index}");
            downloads.insert(
                info.download_id.clone(),
                DownloadRecord {
                    owner: 0,
                    info,
                    bytes: None,
                    saving: false,
                    cancellation: tokio_util::sync::CancellationToken::new(),
                    completion: tokio::sync::watch::channel::<u64>(0).0,
                    last_used: index as u64,
                },
            );
        }
    }
    let mut request = FileDownloadRequest::default();
    request.target = format!("{HOST}:/file/full.bin");
    assert_eq!(coordinator.start_download(request).await.unwrap_err(), "download capacity is full");
    gate.add_permits(1);
    let saved = saving.await.unwrap().unwrap();
    assert_eq!(saved.state, FileDownloadState::Saved);
    assert_eq!(std::fs::read(destination).unwrap(), b"atomic bytes");
    assert!(!duplicate.exists());
}

#[tokio::test]
async fn dropped_save_waiter_does_not_abandon_handoff_or_reservation() {
    let coordinator = Arc::new(coordinator(Arc::new(ScriptedBackend::success(
        RequestResponseTransfer::Packet,
        b"unused",
    ))));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *coordinator.save_gate.lock().await = Some(Arc::clone(&gate));
    let mut info = FileDownloadInfo::default();
    info.download_id = "detached-save".into();
    info.state = FileDownloadState::Completed;
    info.integrity_verified = true;
    coordinator.downloads.lock().await.insert(
        info.download_id.clone(),
        DownloadRecord {
            owner: 0,
            info,
            bytes: Some(b"detached bytes".to_vec()),
            saving: false,
            cancellation: tokio_util::sync::CancellationToken::new(),
            completion: tokio::sync::watch::channel::<u64>(0).0,
            last_used: 0,
        },
    );
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("detached.bin");
    let waiter = {
        let coordinator = Arc::clone(&coordinator);
        let destination = destination.clone();
        tokio::spawn(async move { coordinator.save_download("detached-save", &destination).await })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        while !coordinator.downloads.lock().await["detached-save"].saving {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("save reservation");
    waiter.abort();
    gate.add_permits(1);

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if coordinator.downloads.lock().await["detached-save"].info.state
                == FileDownloadState::Saved
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("detached save completion");
    assert_eq!(std::fs::read(destination).unwrap(), b"detached bytes");
}

#[tokio::test]
async fn cancellation_interrupts_every_download_setup_stage() {
    for stage in ["path", "identity", "link"] {
        let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b"late");
        match stage {
            "path" => scripted.path_delay = Duration::from_secs(60),
            "identity" => scripted.identity_delay = Duration::from_secs(60),
            "link" => scripted.link_delay = Duration::from_secs(60),
            _ => unreachable!(),
        }
        let backend = Arc::new(scripted);
        let coordinator = Arc::new(coordinator(backend.clone()));
        let mut request = FileDownloadRequest::default();
        request.target = format!("{HOST}:/file/setup.bin");
        let started = coordinator.start_download(request).await.unwrap();
        for _ in 0..100 {
            if backend.calls.lock().unwrap().contains(&stage) {
                break;
            }
            tokio::task::yield_now().await;
        }
        let cancelled = tokio::time::timeout(
            Duration::from_millis(100),
            coordinator.cancel_download(&started.download_id),
        )
        .await
        .expect("setup cancellation did not return promptly")
        .expect("download disappeared");
        assert_eq!(cancelled.state, FileDownloadState::Cancelled, "stage {stage}");
    }
}

#[tokio::test]
async fn download_completion_before_wait_registration_is_observed() {
    let coordinator = Arc::new(coordinator(Arc::new(ScriptedBackend::success(
        RequestResponseTransfer::Packet,
        b"unused",
    ))));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *coordinator.cancel_wait_gate.lock().await = Some(Arc::clone(&gate));
    let cancellation = tokio_util::sync::CancellationToken::new();
    let observed_cancellation = cancellation.clone();
    let (completion, _) = tokio::sync::watch::channel(0_u64);
    let mut info = FileDownloadInfo::default();
    info.download_id = "interleaving".into();
    coordinator.downloads.lock().await.insert(
        info.download_id.clone(),
        DownloadRecord {
            owner: 0,
            info,
            bytes: None,
            saving: false,
            cancellation,
            completion,
            last_used: 0,
        },
    );
    let cancellation = {
        let coordinator = Arc::clone(&coordinator);
        tokio::spawn(async move { coordinator.cancel_download("interleaving").await })
    };
    tokio::time::timeout(Duration::from_secs(1), observed_cancellation.cancelled())
        .await
        .expect("cancellation reached the interleaving point");
    {
        let mut downloads = coordinator.downloads.lock().await;
        let record = downloads.get_mut("interleaving").unwrap();
        record.info.state = FileDownloadState::Cancelled;
        record.completion.send_modify(|version| *version += 1);
    }
    gate.add_permits(1);

    let cancelled = tokio::time::timeout(Duration::from_millis(100), cancellation)
        .await
        .expect("retained completion notification prevents a lost wakeup")
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.state, FileDownloadState::Cancelled);
}

#[tokio::test]
async fn download_cancellation_wait_has_a_hard_terminal_bound() {
    let coordinator = Arc::new(coordinator(Arc::new(ScriptedBackend::success(
        RequestResponseTransfer::Packet,
        b"unused",
    ))));
    let cancellation = tokio_util::sync::CancellationToken::new();
    let observed_cancellation = cancellation.clone();
    let (completion, _) = tokio::sync::watch::channel(0_u64);
    let mut info = FileDownloadInfo::default();
    info.download_id = "bounded-cancel".into();
    coordinator.downloads.lock().await.insert(
        info.download_id.clone(),
        DownloadRecord {
            owner: 0,
            info,
            bytes: None,
            saving: false,
            cancellation,
            completion,
            last_used: 0,
        },
    );
    let waiter = {
        let coordinator = Arc::clone(&coordinator);
        tokio::spawn(async move { coordinator.cancel_download("bounded-cancel").await })
    };
    observed_cancellation.cancelled().await;

    let cancelled = tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("bounded cancellation waiter")
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.state, FileDownloadState::Cancelled);
    assert_eq!(cancelled.error.as_deref(), Some("download cancellation completion timed out"));
}

#[tokio::test]
async fn download_capacity_never_evicts_active_work_and_uses_terminal_lru() {
    let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b"late");
    scripted.path_delay = Duration::from_secs(60);
    let coordinator = Arc::new(coordinator(Arc::new(scripted)));
    {
        let mut downloads = coordinator.downloads.lock().await;
        for index in 0..MAX_DOWNLOADS {
            let mut info = FileDownloadInfo::default();
            info.download_id = format!("active-{index}");
            downloads.insert(
                info.download_id.clone(),
                DownloadRecord {
                    owner: 0,
                    info,
                    bytes: None,
                    saving: false,
                    cancellation: tokio_util::sync::CancellationToken::new(),
                    completion: tokio::sync::watch::channel::<u64>(0).0,
                    last_used: index as u64,
                },
            );
        }
    }
    let mut request = FileDownloadRequest::default();
    request.target = format!("{HOST}:/file/full.bin");
    assert_eq!(
        coordinator.start_download(request.clone()).await.unwrap_err(),
        "download capacity is full"
    );
    {
        let mut downloads = coordinator.downloads.lock().await;
        downloads.get_mut("active-0").unwrap().info.state = FileDownloadState::Completed;
    }
    let started = coordinator.start_download(request).await.expect("terminal LRU eviction");
    assert!(!coordinator.downloads.lock().await.contains_key("active-0"));
    let cancelled = coordinator.cancel_download(&started.download_id).await.unwrap();
    assert_eq!(cancelled.state, FileDownloadState::Cancelled);
}

#[tokio::test]
async fn link_variables_neither_read_nor_replace_ordinary_url_cache() {
    let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">public"));
    for source in [b"`[Run`/page/next.mu`mode=private]".as_slice(), b">private".as_slice()] {
        let next = ScriptedBackend::success(RequestResponseTransfer::Packet, source);
        backend
            .outcome
            .lock()
            .unwrap()
            .push_back(next.outcome.lock().unwrap().pop_front().unwrap());
    }
    let coordinator = coordinator(backend.clone());
    let mut request = PageNavigationRequest::default();
    request.target = Some(format!("{HOST}:/page/next.mu"));
    let public = coordinator.navigate(request.clone(), HOST, |_| Vec::new()).await.unwrap();
    let mut index = PageNavigationRequest::default();
    index.target = Some(format!("{HOST}:/page/index.mu"));
    let index = coordinator.navigate(index, HOST, |_| Vec::new()).await.unwrap();
    let mut link = PageNavigationRequest::default();
    link.session_id = Some(index.navigation.session_id);
    link.target = Some("/page/next.mu".into());
    let private = coordinator.navigate(link, HOST, |_| Vec::new()).await.unwrap();
    assert_eq!(private.cache.status, PageCacheStatus::Miss);
    assert_ne!(private.source_bytes, public.source_bytes);
    let ordinary = coordinator.navigate(request, HOST, |_| Vec::new()).await.unwrap();
    assert_eq!(ordinary.cache.status, PageCacheStatus::Hit);
    assert_eq!(ordinary.source_bytes, public.source_bytes);
    assert_eq!(backend.requested_data.lock().unwrap().len(), 3);
}
