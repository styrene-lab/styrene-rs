//! Controlled integration across daemon authorization, adapter, domain and IPC.
use rns_core::{hash::AddressHash, identity::PrivateIdentity};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use styrene_ipc::{traits::Daemon, types::*};
use styrened::{
    app_context::AppContext,
    daemon_facade::DaemonFacade,
    services::{PageService, PolicyService},
    storage::messages::MessagesStore,
    transport::mock_transport::{MockCall, MockTransport},
};
const HOST: &str = "0123456789abcdef0123456789abcdef";

fn fixture() -> (tempfile::TempDir, Arc<AppContext>, Arc<dyn Daemon>, Arc<MockTransport>) {
    let dir = tempfile::tempdir().expect("fixture root");
    let pages = dir.path().join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(
        pages.join("index.mu"),
        b">Local fixture\nHello `[Next`next.mu]\n`<12!|password`secret>",
    )
    .unwrap();
    std::fs::write(pages.join("next.mu"), b">Next\nSecond page").unwrap();
    let transport = Arc::new(MockTransport::new_default());
    let ctx = Arc::new(AppContext::with_policy_and_pages(
        transport.clone(),
        "ab".repeat(16),
        Arc::new(Mutex::new(MessagesStore::in_memory().unwrap())),
        Arc::new(styrene_services::node_store::NodeStore::in_memory().unwrap()),
        PageService::with_storage_dirs(pages, dir.path().join("files")),
        PolicyService::new(styrene_rbac::RbacPolicy::new(styrene_rbac::Role::Admin)),
    ));
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx.clone(), "caller".into()));
    (dir, ctx, daemon, transport)
}

fn queue_content(transport: &MockTransport, source: &[u8], reused: bool) {
    let peer = PrivateIdentity::new_from_name("nomadnet-integration-peer");
    transport.queue_resolve(Some(*peer.as_identity()));
    transport.set_path(
        AddressHash::new(hex::decode(HOST).unwrap().try_into().unwrap()),
        1,
        AddressHash::new([3; 16]),
    );
    if reused {
        transport.queue_reused_link(AddressHash::new([0x22; 16]));
    } else {
        transport.queue_open_link(Ok(AddressHash::new([0x22; 16])));
    }
    let mut started = RequestObservationInfo::default();
    started.request_id = "11".repeat(16);
    started.link_id = "22".repeat(16);
    started.request_size = 24;
    started.state = RequestState::Pending;
    transport.queue_request(Ok(started.clone()));
    let mut encoded = Vec::new();
    rmpv::encode::write_value(&mut encoded, &rmpv::Value::Binary(source.to_vec())).unwrap();
    let mut completed = started;
    completed.state = RequestState::Succeeded;
    completed.response_transfer = RequestResponseTransfer::Packet;
    completed.response_size = Some(encoded.len() as u64);
    completed.response = Some(encoded);
    completed.received_bytes = source.len() as u64;
    completed.total_bytes = source.len() as u64;
    completed.progress = 1.0;
    completed.observation.source = ObservationSource::TransportRequestState;
    completed.observation.connection_generation = Some(7);
    completed.observation.ipc_connection_generation = Some(8);
    completed.observation.interface_generation = Some(9);
    transport.queue_request_receipt(completed);
}

#[tokio::test]
async fn local_navigation_history_and_owner_isolation_cross_the_facade() {
    let (_dir, _ctx, daemon, transport) = fixture();
    let first = daemon.browse_page_for_owner(41, "local", "/page/index.mu", Some(1)).await.unwrap();
    assert_eq!(first.outcome, PageBrowseOutcome::Succeeded);
    assert_eq!(first.title.as_deref(), Some("Local fixture"));
    assert_eq!(first.fields[0].kind, PageFormFieldKind::Password);
    assert_eq!(first.fields[0].value, None);
    let mut nav = PageNavigationRequest::default();
    nav.session_id = Some(first.navigation.session_id.clone());
    nav.target = Some("next.mu".into());
    assert!(daemon.navigate_page_for_owner(42, nav.clone()).await.is_err());
    let second = daemon.navigate_page_for_owner(41, nav.clone()).await.unwrap();
    assert_eq!(second.title.as_deref(), Some("Next"));
    assert!(second.navigation.can_back);
    nav.action = PageNavigationAction::Back;
    let back = daemon.navigate_page_for_owner(41, nav).await.unwrap();
    assert_eq!(back.cache.status, PageCacheStatus::Hit);
    assert_eq!(back.source_bytes, first.source_bytes);
    assert!(back.stages.iter().all(|s| matches!(s.state, PageBrowseStageState::Skipped { .. })));
    assert!(daemon.close_page_session_for_owner(42, &first.navigation.session_id).await.is_err());
    daemon.close_page_session_for_owner(41, &first.navigation.session_id).await.unwrap();
    assert!(transport.calls().is_empty(), "local browsing must not call transport");
}

#[tokio::test]
async fn remote_receipts_shared_links_and_downloads_cross_the_adapter() {
    let (dir, ctx, daemon, transport) = fixture();
    ctx.discovery()
        .accept_announce_with_type(
            HOST.into(),
            1,
            b"Fixture",
            Some(styrened::services::discovery::NATIVE_NOMADNET_HOST_DEVICE_TYPE),
        )
        .unwrap();
    ctx.set_signer(Arc::new(PrivateIdentity::new_from_name("reader")));
    queue_content(&transport, b">Remote\nHello", false);
    let first = daemon.browse_page_for_owner(10, HOST, "/page/index.mu", Some(1)).await.unwrap();
    assert_eq!(first.outcome, PageBrowseOutcome::Succeeded, "{:?}", first.failure);
    assert_eq!(first.request.link_id.as_deref(), Some("22222222222222222222222222222222"));
    assert_eq!(first.transfer.kind, PageTransferKind::Packet);
    assert!(first.transfer.verified);
    let transfer_stage =
        first.stages.iter().find(|s| s.kind == PageBrowseStageKind::Transfer).unwrap();
    assert_eq!(transfer_stage.observation.connection_generation, Some(7));
    assert_eq!(transfer_stage.observation.ipc_connection_generation, Some(8));
    assert_eq!(transfer_stage.observation.interface_generation, Some(9));
    assert_eq!(first.source_bytes, b">Remote\nHello");
    assert!(transport.calls().iter().any(|c| matches!(c, MockCall::IdentifyLink { .. })));
    queue_content(&transport, b">Second", true);
    let second = daemon.browse_page_for_owner(10, HOST, "/page/second.mu", Some(1)).await.unwrap();
    assert_eq!(second.outcome, PageBrowseOutcome::Succeeded);
    daemon.close_page_session_for_owner(10, &first.navigation.session_id).await.unwrap();
    assert!(
        !transport.calls().iter().any(|c| matches!(c, MockCall::CloseLink { .. })),
        "second session still retains shared link"
    );
    queue_content(&transport, b"file bytes", true);
    let mut request = FileDownloadRequest::default();
    request.session_id = Some(second.navigation.session_id.clone());
    request.target = "/file/sample.bin".into();
    request.timeout_secs = Some(1);
    let download = daemon.start_file_download_for_owner(10, request).await.unwrap();
    let finished = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let state = daemon.file_download_for_owner(10, &download.download_id).await.unwrap();
            if state.state.is_terminal() {
                break state;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(finished.state, FileDownloadState::Completed, "{:?}", finished.error);
    assert!(finished.integrity_verified);
    assert!(daemon.file_download_for_owner(11, &download.download_id).await.is_err());
    let path = dir.path().join("saved.bin");
    let saved = daemon
        .save_file_download_for_owner(10, &download.download_id, path.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(saved.state, FileDownloadState::Saved);
    assert_eq!(std::fs::read(path).unwrap(), b"file bytes");
    transport.queue_close(Ok(()));
    daemon.close_page_session_for_owner(10, &second.navigation.session_id).await.unwrap();
    assert_eq!(
        transport.calls().iter().filter(|c| matches!(c, MockCall::CloseLink { .. })).count(),
        1
    );
}

#[cfg(feature = "ipc-server")]
#[tokio::test]
async fn real_unix_ipc_uses_extracted_coordinator() {
    let (_dir, _ctx, daemon, _transport) = fixture();
    let socket_dir = tempfile::Builder::new().prefix("nn-").tempdir_in("/tmp").unwrap();
    let socket = socket_dir.path().join("ipc.sock");
    let mut server = styrene_ipc_server::IpcServer::new(
        daemon,
        styrene_ipc_server::IpcServerConfig { socket_path: socket.clone(), event_capacity: 16 },
    );
    server.start().await.unwrap();
    let (client, _) = styrene_ipc_client::Client::connect_unix(
        &socket,
        styrene_ipc_client::ConnectionGeneration(1),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    let page = client.browse_page("local", "/page/index.mu", Some(1)).await.unwrap();
    assert_eq!(page.outcome, PageBrowseOutcome::Succeeded);
    assert_eq!(page.fields[0].value, None);
    assert_eq!(page.title.as_deref(), Some("Local fixture"));
    client.close_page(&page.navigation.session_id).await.unwrap();
    server.stop().await;
}

#[test]
fn legacy_ipc_and_domain_address_contracts_agree() {
    for input in [
        "/",
        "/page/index.mu",
        "0123456789ABCDEF0123456789ABCDEF:/page/start.mu",
        "/page/../bad.mu",
        "bad:/page/index.mu",
        "/file/x",
        "/page/two words.mu",
    ] {
        let old = styrene_ipc::PageAddress::parse(input)
            .map(|a| a.to_string())
            .map_err(|e| e.to_string());
        let new = styrene_nomadnet::PageAddress::parse(input)
            .map(|a| a.to_string())
            .map_err(|e| e.to_string());
        assert_eq!(old, new, "{input}");
    }
}

#[tokio::test]
async fn denied_browse_stays_outside_the_domain_and_transport() {
    let (_dir, ctx, _daemon, transport) = fixture();
    let caller = "de".repeat(16);
    ctx.policy()
        .grant(styrene_rbac::RosterEntry::new(&caller, styrene_rbac::Role::Blocked), ctx.store())
        .unwrap();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, caller));
    assert!(matches!(
        daemon.browse_page(HOST, "/page/index.mu", Some(1)).await,
        Err(styrene_ipc::IpcError::Denied { .. })
    ));
    assert!(transport.calls().is_empty());
}
