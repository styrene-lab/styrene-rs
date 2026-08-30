use std::time::Duration;

use base64::Engine as _;
use rns_core::identity::PrivateIdentity;
use sha2::{Digest as _, Sha256};
use styrene_ipc::traits::{DaemonEvents, DaemonIdentity, DaemonMessaging, DaemonStatus};
use styrene_ipc::types::{
    CapabilityFailureCode, ConversationInvalidationReason, DaemonEvent,
    IdentityCustodyAuthentication, IdentityCustodyAvailability, IdentityCustodyBackend,
    IdentityCustodyDowngrade, IdentityCustodyProtection, InterfaceFailureCode,
    MOBILE_DIAGNOSTIC_MAX_BYTES, MOBILE_DIAGNOSTIC_MAX_EVENTS, MessagingDisposition,
    MobileDiagnosticSeverity, MobileDiagnosticSource, MobileDiagnosticStage,
};
use styrened::mobile::{
    IdentityBackend, LEGACY_HUB_POLL_DEADLINE, LEGACY_HUB_POLL_MAX_BYTES,
    LEGACY_HUB_POLL_MAX_ITEMS, MobileBootFailureCode, MobileBootStage, MobileConfig,
    MobileConnectionPhase, MobileInterfaceConfig, MobileNode, MobileRuntimeState,
    POLL_PREVIEW_MAX_BYTES, POLL_PREVIEW_MAX_CHARS, PollAcknowledgementOutcome, PollBatchFailure,
    PollLocalOutcome, legacy_poll_preview,
};
use styrened::startup_contract::CapabilityFailureKind;
use styrened::storage::messages::{
    AttemptRouteObservationRecord, MessageRecord, MessagesStore, OutboundAttemptRecord,
    OutboundRouteRecord, StorageRecoveryOutcome,
};

fn config(root: &std::path::Path, backend: IdentityBackend) -> MobileConfig {
    MobileConfig {
        config_dir: root.join("config"),
        data_dir: root.join("data"),
        hub_address: None,
        hub_delivery_hash: None,
        display_name: None,
        identity_backend: backend,
        interfaces: Vec::new(),
        enable_rnode_channel: false,
    }
}

async fn shutdown(node: MobileNode) {
    node.shutdown().await.expect("mobile shutdown");
}

fn legacy_poll_wire(content: &str) -> Vec<u8> {
    let signer = PrivateIdentity::new_from_name("mobile-p0-legacy-poll-sender");
    styrened::lxmf_bridge::build_wire_message([0x41; 16], [0; 16], "", content, None, &signer)
        .expect("build legacy poll fixture")
}

#[tokio::test]
async fn legacy_poll_acknowledges_only_durable_results() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();
    let wire = legacy_poll_wire("durable message");

    let batch = node.process_legacy_hub_batch(vec![
        ("accepted".into(), wire.clone()),
        ("durable-duplicate".into(), wire),
        ("decode-rejected".into(), b"not an LXMF message".to_vec()),
    ]);

    assert_eq!(batch.acknowledgement_ids(), ["accepted", "durable-duplicate"]);
    let result = batch.complete(Ok(()));
    assert_eq!(result.message_count, 1);
    assert!(matches!(result.items[0].local, PollLocalOutcome::Accepted { .. }));
    assert!(matches!(result.items[1].local, PollLocalOutcome::DurableDuplicate { .. }));
    assert!(matches!(result.items[2].local, PollLocalOutcome::DecodeRejected { .. }));
    assert_eq!(result.items[0].acknowledgement, PollAcknowledgementOutcome::Acknowledged);
    assert_eq!(result.items[1].acknowledgement, PollAcknowledgementOutcome::Acknowledged);
    assert_eq!(result.items[2].acknowledgement, PollAcknowledgementOutcome::NotEligible);
    shutdown(node).await;

    let storage_root = tempfile::tempdir().unwrap();
    let storage_node =
        MobileNode::boot(config(storage_root.path(), IdentityBackend::PlaintextFile))
            .await
            .unwrap();
    let store = storage_node.app_context.store().clone();
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = store.lock().expect("lock store before poisoning");
        panic!("deterministic legacy poll storage failure");
    }));
    assert!(poisoned.is_err());
    let storage_batch = storage_node.process_legacy_hub_batch(vec![(
        "storage-failed".into(),
        legacy_poll_wire("must remain on hub"),
    )]);
    assert!(storage_batch.acknowledgement_ids().is_empty());
    let storage_result = storage_batch.complete(Ok(()));
    assert!(matches!(storage_result.items[0].local, PollLocalOutcome::StorageFailed { .. }));
    assert_eq!(storage_result.items[0].acknowledgement, PollAcknowledgementOutcome::NotEligible);
}

#[tokio::test]
async fn legacy_poll_reports_partial_acknowledgement_failure() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();
    let batch = node.process_legacy_hub_batch(vec![
        ("durable".into(), legacy_poll_wire("retain on hub")),
        ("rejected".into(), vec![0xff, 0x00]),
    ]);

    assert_eq!(batch.acknowledgement_ids(), ["durable"]);
    let result = batch.complete(Err("scripted hub delete failure".into()));
    assert_eq!(result.message_count, 1);
    assert_eq!(
        result.items[0].acknowledgement,
        PollAcknowledgementOutcome::Failed { error: "scripted hub delete failure".into() }
    );
    assert_eq!(result.items[1].acknowledgement, PollAcknowledgementOutcome::NotEligible);
    shutdown(node).await;
}

#[tokio::test]
async fn unicode_previews_are_bounded_and_panic_free() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();
    let contents = [
        String::new(),
        "a".repeat(POLL_PREVIEW_MAX_CHARS),
        "界".repeat(POLL_PREVIEW_MAX_CHARS + 1),
        "e\u{301}".repeat(POLL_PREVIEW_MAX_CHARS + 1),
        format!("{}界", "a".repeat(POLL_PREVIEW_MAX_BYTES - 1)),
    ];
    let messages = contents
        .iter()
        .enumerate()
        .map(|(index, content)| (format!("preview-{index}"), legacy_poll_wire(content)))
        .collect();

    let result = node.process_legacy_hub_batch(messages).complete(Ok(()));

    assert_eq!(result.message_count, contents.len());
    assert!(result.messages.iter().all(|message| {
        message.content_preview.len() <= POLL_PREVIEW_MAX_BYTES
            && message.content_preview.chars().count() <= POLL_PREVIEW_MAX_CHARS
            && message.content_preview.is_char_boundary(message.content_preview.len())
    }));
    assert_eq!(result.messages[0].content_preview, "");
    assert_eq!(result.messages[1].content_preview, contents[1]);
    assert_eq!(result.messages[4].content_preview, "a".repeat(POLL_PREVIEW_MAX_BYTES - 1));

    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    for case in 0..4096 {
        let length = case % 321;
        let mut value = String::new();
        for _ in 0..length {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let scalar = match state % 8 {
                0 => 0x20 + (state as u32 % 0x5f),
                1 => 0x300 + (state as u32 % 0x70),
                2 => 0x400 + (state as u32 % 0x100),
                3 => 0x590 + (state as u32 % 0x100),
                4 => 0x900 + (state as u32 % 0x80),
                5 => 0x4e00 + (state as u32 % 0x500),
                6 => 0x1f300 + (state as u32 % 0x300),
                _ => 0x10000 + (state as u32 % 0x8000),
            };
            value.push(char::from_u32(scalar).expect("generated valid Unicode scalar"));
        }
        let preview = std::panic::catch_unwind(|| legacy_poll_preview(&value))
            .expect("valid Unicode preview must not panic");
        assert!(value.starts_with(&preview));
        assert!(preview.len() <= POLL_PREVIEW_MAX_BYTES);
        assert!(preview.chars().count() <= POLL_PREVIEW_MAX_CHARS);
        assert!(value.is_char_boundary(preview.len()));
    }
    shutdown(node).await;
}

#[tokio::test]
async fn legacy_poll_enforces_corpus_bounds_and_shared_deadline() {
    assert_eq!(LEGACY_HUB_POLL_DEADLINE, Duration::from_secs(30));
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();

    let item_limited = node.process_legacy_hub_batch(
        (0..=LEGACY_HUB_POLL_MAX_ITEMS)
            .map(|index| (format!("item-{index}"), Vec::new()))
            .collect(),
    );
    assert!(item_limited.acknowledgement_ids().is_empty());
    let item_result = item_limited.complete(Ok(()));
    assert!(item_result.items.is_empty());
    assert!(item_result.items.len() <= LEGACY_HUB_POLL_MAX_ITEMS);
    assert_eq!(
        item_result.batch_failure,
        Some(PollBatchFailure::ItemLimitExceeded {
            limit: LEGACY_HUB_POLL_MAX_ITEMS,
            observed: LEGACY_HUB_POLL_MAX_ITEMS + 1,
        })
    );

    let byte_limited = node.process_legacy_hub_batch(vec![(
        "oversized".into(),
        vec![0; LEGACY_HUB_POLL_MAX_BYTES + 1],
    )]);
    assert!(byte_limited.acknowledgement_ids().is_empty());
    let byte_result = byte_limited.complete(Ok(()));
    assert!(byte_result.items.is_empty());
    assert!(byte_result.items.len() <= LEGACY_HUB_POLL_MAX_ITEMS);
    assert!(matches!(
        byte_result.batch_failure,
        Some(PollBatchFailure::ByteLimitExceeded {
            limit: LEGACY_HUB_POLL_MAX_BYTES,
            observed,
        }) if observed > LEGACY_HUB_POLL_MAX_BYTES
    ));
    shutdown(node).await;
}

#[tokio::test]
async fn mobile_diagnostics_are_bounded_and_chronological() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();

    for correlation in 0..MOBILE_DIAGNOSTIC_MAX_EVENTS + 137 {
        node.record_diagnostic(
            MobileDiagnosticSource::Messaging,
            MobileDiagnosticStage::Outbound,
            MobileDiagnosticSeverity::Info,
            Some(&correlation.to_be_bytes()),
        );
    }

    let snapshot = node.facade.mobile_diagnostics().await.unwrap();
    assert!(snapshot.event_count <= MOBILE_DIAGNOSTIC_MAX_EVENTS);
    assert!(snapshot.retained_bytes <= MOBILE_DIAGNOSTIC_MAX_BYTES);
    assert!(snapshot.truncated);
    assert!(snapshot.dropped_events >= 137);
    assert_eq!(snapshot.event_count as usize, snapshot.events.len());
    assert_eq!(snapshot.first_sequence, snapshot.events.first().map(|event| event.sequence));
    assert_eq!(snapshot.last_sequence, snapshot.events.last().map(|event| event.sequence));
    assert!(snapshot.events.windows(2).all(|events| events[0].sequence < events[1].sequence));
    assert!(snapshot.events.iter().all(|event| event.generation == 1));

    let export = node.facade.export_mobile_diagnostics().await.unwrap();
    assert!(export.bytes.len() <= MOBILE_DIAGNOSTIC_MAX_BYTES as usize);
    assert_eq!(export.event_count, snapshot.event_count);
    assert_eq!(export.first_sequence, snapshot.first_sequence);
    assert_eq!(export.last_sequence, snapshot.last_sequence);
    shutdown(node).await;
}

#[tokio::test]
async fn diagnostic_export_is_deterministic_and_payload_free() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();
    let forbidden = [
        "P0_MESSAGE_CONTENT_8d1f9c2a",
        "P0_MESSAGE_TITLE_29a2d18b",
        "P0_CANONICAL_WIRE_f7c64e10",
        "P0_ATTACHMENT_BYTES_741cca9e",
        "P0_IDENTITY_PRIVATE_KEY_b083a3d2",
        "P0_CREDENTIAL_1c45dd86",
        "P0_TOKEN_963bf080",
        "P0_PASSPHRASE_2ba57d3e",
        "/private/mobile/p0-path-36ac9f52",
    ];
    for value in forbidden {
        node.record_diagnostic(
            MobileDiagnosticSource::Platform,
            MobileDiagnosticStage::Persistence,
            MobileDiagnosticSeverity::Warning,
            Some(value.as_bytes()),
        );
    }
    let low_entropy = b"0";
    node.record_diagnostic(
        MobileDiagnosticSource::Messaging,
        MobileDiagnosticStage::Outbound,
        MobileDiagnosticSeverity::Info,
        Some(low_entropy),
    );
    node.record_diagnostic(
        MobileDiagnosticSource::Messaging,
        MobileDiagnosticStage::Outbound,
        MobileDiagnosticSeverity::Info,
        Some(low_entropy),
    );

    let first = node.facade.export_mobile_diagnostics().await.unwrap();
    let second = node.facade.export_mobile_diagnostics().await.unwrap();
    assert_eq!(first.bytes, second.bytes);
    assert_eq!(first.digest_sha256, second.digest_sha256);
    assert_eq!(first.digest_sha256, hex::encode(Sha256::digest(&first.bytes)));
    assert_eq!(first.byte_count as usize, first.bytes.len());
    assert!(first.bytes.len() <= MOBILE_DIAGNOSTIC_MAX_BYTES as usize);

    let text = String::from_utf8(first.bytes.clone()).unwrap();
    let snapshot_text = serde_json::to_string(&node.facade.mobile_diagnostics().await.unwrap())
        .expect("serialize diagnostic snapshot");
    for value in forbidden {
        let encodings = [
            value.to_string(),
            hex::encode(value.as_bytes()),
            base64::engine::general_purpose::STANDARD.encode(value),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value),
        ];
        for encoded in encodings {
            assert!(!text.contains(&encoded), "diagnostic export leaked forbidden encoding");
            assert!(
                !snapshot_text.contains(&encoded),
                "diagnostic snapshot leaked forbidden encoding"
            );
        }
    }
    let json: serde_json::Value = serde_json::from_slice(&first.bytes).unwrap();
    let events = json["events"].as_array().unwrap();
    let correlations =
        events.iter().filter_map(|event| event["safe_correlation"].as_str()).collect::<Vec<_>>();
    let keyed = &correlations[correlations.len() - 2..];
    assert_eq!(keyed[0], keyed[1], "same runtime correlation must remain stable");
    assert!(keyed[0].starts_with("hmac-sha256:"));
    assert_ne!(
        keyed[0],
        format!("sha256:{}", hex::encode(Sha256::digest(low_entropy))),
        "low-entropy correlation must not be offline-verifiable plain SHA-256"
    );
    assert!(!text.contains(&hex::encode(Sha256::digest(low_entropy))));
    assert!(events.iter().all(|event| {
        event.as_object().is_some_and(|fields| {
            fields.keys().all(|field| {
                matches!(
                    field.as_str(),
                    "sequence"
                        | "unix_time_ms"
                        | "source"
                        | "stage"
                        | "severity"
                        | "generation"
                        | "safe_correlation"
                )
            })
        })
    }));

    let other_root = tempfile::tempdir().unwrap();
    let other =
        MobileNode::boot(config(other_root.path(), IdentityBackend::PlaintextFile)).await.unwrap();
    other.record_diagnostic(
        MobileDiagnosticSource::Messaging,
        MobileDiagnosticStage::Outbound,
        MobileDiagnosticSeverity::Info,
        Some(low_entropy),
    );
    let other_snapshot = other.diagnostic_snapshot();
    let other_correlation =
        other_snapshot.events.last().and_then(|event| event.safe_correlation.as_deref()).unwrap();
    assert_ne!(keyed[0], other_correlation, "correlation keys must be runtime-local");
    shutdown(other).await;
    shutdown(node).await;
}

#[tokio::test]
async fn production_mobile_operations_record_allowlisted_outcomes() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();

    assert!(node.poll_hub().await.is_err());
    assert!(node.sync_propagation_once(Duration::from_millis(10)).await.is_err());
    node.shutdown().await.unwrap();

    let snapshot = node.diagnostic_snapshot();
    assert!(snapshot.events.iter().any(|event| {
        event.source == MobileDiagnosticSource::Runtime
            && event.stage == MobileDiagnosticStage::Boot
            && event.severity == MobileDiagnosticSeverity::Info
    }));
    assert!(snapshot.events.iter().any(|event| {
        event.source == MobileDiagnosticSource::Messaging
            && event.stage == MobileDiagnosticStage::Inbound
            && event.severity == MobileDiagnosticSeverity::Error
    }));
    assert!(snapshot.events.iter().any(|event| {
        event.source == MobileDiagnosticSource::Messaging
            && event.stage == MobileDiagnosticStage::Synchronization
            && event.severity == MobileDiagnosticSeverity::Error
    }));
    assert!(snapshot.events.iter().any(|event| {
        event.source == MobileDiagnosticSource::Runtime
            && event.stage == MobileDiagnosticStage::Lifecycle
            && event.severity == MobileDiagnosticSeverity::Info
    }));
    assert!(snapshot.events.iter().all(|event| event.safe_correlation.is_none()));
}

#[tokio::test]
async fn production_custody_backends_fail_closed() {
    #[cfg(not(all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios"))))]
    {
        let root = tempfile::tempdir().unwrap();
        let result = MobileNode::boot(config(root.path(), IdentityBackend::Keychain)).await;
        assert!(result.is_err(), "unsupported Keychain backend must fail");
        assert!(!root.path().join("config/identity").exists());
    }

    #[cfg(not(target_os = "android"))]
    {
        let root = tempfile::tempdir().unwrap();
        let result = MobileNode::boot(config(root.path(), IdentityBackend::AndroidKeystore)).await;
        assert!(result.is_err(), "unsupported Android Keystore backend must fail");
        assert!(!root.path().join("config/identity").exists());
    }

    let root = tempfile::tempdir().unwrap();
    let result = MobileNode::boot(config(root.path(), IdentityBackend::EncryptedFile)).await;
    let error = match result {
        Ok(node) => {
            shutdown(node).await;
            panic!("encrypted-file custody accepted missing host key material");
        }
        Err(error) => error,
    };
    assert!(error.to_string().contains("nonempty host key material"));
    assert!(!root.path().join("config/identity").exists());

    #[cfg(not(feature = "mobile-identity"))]
    {
        let root = tempfile::tempdir().unwrap();
        let result = MobileNode::boot_with_encrypted_file_key(
            config(root.path(), IdentityBackend::EncryptedFile),
            b"host-owned-test-key",
        )
        .await;
        assert!(result.is_err(), "unsupported encrypted-file backend must fail");
        assert!(!root.path().join("config/identity").exists());
    }
}

#[tokio::test]
async fn plaintext_backend_requires_explicit_selection() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile))
        .await
        .expect("explicit development backend");
    assert!(root.path().join("config/identity").exists());
    shutdown(node).await;
}

#[tokio::test]
async fn custody_projection_is_authoritative_and_secret_free() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();
    let identity = node.facade.query_identity().await.unwrap();
    let custody = identity.custody.expect("mobile custody projection");

    assert_eq!(custody.requested_backend, IdentityCustodyBackend::PlaintextFile);
    assert_eq!(custody.active_backend, Some(IdentityCustodyBackend::PlaintextFile));
    assert_eq!(custody.protection, Some(IdentityCustodyProtection::DevelopmentPlaintext));
    assert_eq!(custody.authentication, IdentityCustodyAuthentication::None);
    assert_eq!(custody.availability, IdentityCustodyAvailability::Available);
    assert_eq!(custody.downgrade, IdentityCustodyDowngrade::None);
    let json = serde_json::to_string(&custody).unwrap();
    for forbidden in ["private", "passphrase", "credential", "key_material", "export"] {
        assert!(!json.contains(forbidden), "custody projection leaked {forbidden}");
    }
    shutdown(node).await;
}

#[cfg(feature = "mobile-identity")]
#[tokio::test]
async fn encrypted_file_create_restore_has_authoritative_custody_projection() {
    let root = tempfile::tempdir().unwrap();
    let mobile_config = config(root.path(), IdentityBackend::EncryptedFile);
    let first = MobileNode::boot_with_encrypted_file_key(
        mobile_config.clone(),
        b"host-owned-feature-matrix-key",
    )
    .await
    .unwrap();
    let first_identity = first.facade.query_identity().await.unwrap();
    let custody = first_identity.custody.expect("encrypted custody projection");

    assert_eq!(custody.requested_backend, IdentityCustodyBackend::EncryptedFile);
    assert_eq!(custody.active_backend, Some(IdentityCustodyBackend::EncryptedFile));
    assert_eq!(custody.protection, Some(IdentityCustodyProtection::EncryptedAtRest));
    assert_eq!(custody.authentication, IdentityCustodyAuthentication::HostKeyMaterial);
    assert_eq!(custody.availability, IdentityCustodyAvailability::Available);
    assert_eq!(custody.downgrade, IdentityCustodyDowngrade::None);
    assert!(custody.failure.is_none());
    let encoded = serde_json::to_string(&custody).unwrap();
    for forbidden in
        ["host-owned-feature-matrix-key", "private_key", "passphrase", "credential", "export"]
    {
        assert!(!encoded.contains(forbidden), "encrypted custody projection leaked {forbidden}");
    }
    assert_ne!(std::fs::read(root.path().join("config/identity")).unwrap().len(), 64);
    shutdown(first).await;

    let second =
        MobileNode::boot_with_encrypted_file_key(mobile_config, b"host-owned-feature-matrix-key")
            .await
            .unwrap();
    let restored = second.facade.query_identity().await.unwrap();
    assert_eq!(restored.identity_hash, first_identity.identity_hash);
    assert_eq!(restored.custody, Some(custody));
    shutdown(second).await;
}

#[tokio::test]
async fn public_identity_edits_survive_restart() {
    let root = tempfile::tempdir().unwrap();
    let mut mobile_config = config(root.path(), IdentityBackend::PlaintextFile);
    mobile_config.display_name = Some("Initial Node".into());
    let first = MobileNode::boot(mobile_config.clone()).await.unwrap();
    let original_hash = first.facade.query_identity().await.unwrap().identity_hash;
    assert!(
        first.facade.set_identity(Some("  Field Node  "), Some("radio"), Some("FN")).await.unwrap()
    );
    shutdown(first).await;

    let second = MobileNode::boot(mobile_config).await.unwrap();
    let restored = second.facade.query_identity().await.unwrap();
    assert_eq!(restored.display_name, "Field Node");
    assert_eq!(restored.icon.as_deref(), Some("radio"));
    assert_eq!(restored.short_name.as_deref(), Some("FN"));
    assert_eq!(restored.identity_hash, original_hash);
    shutdown(second).await;
}

#[tokio::test]
async fn invalid_identity_edit_is_atomic() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();
    node.facade.set_identity(Some("Stable Node"), None, None).await.unwrap();
    let path = root.path().join("config/identity-public.json");
    let before = std::fs::read(&path).unwrap();

    assert!(node.facade.set_identity(Some("bad\nname"), Some("changed"), None).await.is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
    let identity = node.facade.query_identity().await.unwrap();
    assert_eq!(identity.display_name, "Stable Node");
    assert_eq!(identity.icon, None);
    shutdown(node).await;
}

#[tokio::test]
async fn offline_runtime_is_ready_but_not_connected() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();

    let snapshot = node.session_snapshot().await;
    assert_eq!(snapshot.runtime, MobileRuntimeState::Ready);
    assert_eq!(snapshot.phase, MobileConnectionPhase::Offline);
    assert!(!node.is_connected());
    assert!(snapshot.failure.is_none());

    shutdown(node).await;
}

#[tokio::test]
async fn shutdown_is_distinct_from_offline_ready() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();
    assert_eq!(node.session_snapshot().await.runtime, MobileRuntimeState::Ready);

    node.shutdown().await.unwrap();
    let stopped = node.session_snapshot().await;
    assert_eq!(stopped.runtime, MobileRuntimeState::Stopped);
    assert_eq!(stopped.phase, MobileConnectionPhase::Stopped);
}

async fn occupied_server_config(root: &std::path::Path) -> (tokio::net::TcpListener, MobileConfig) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut mobile_config = config(root, IdentityBackend::PlaintextFile);
    mobile_config.interfaces.push(MobileInterfaceConfig::TcpServer {
        bind_address: listener.local_addr().unwrap().to_string(),
    });
    (listener, mobile_config)
}

#[tokio::test]
async fn partial_boot_failure_is_typed_and_retryable() {
    let root = tempfile::tempdir().unwrap();
    let (_listener, mobile_config) = occupied_server_config(root.path()).await;
    let error = match MobileNode::boot(mobile_config).await {
        Ok(node) => {
            shutdown(node).await;
            panic!("occupied listener unexpectedly booted");
        }
        Err(error) => error,
    };

    assert_eq!(error.stage, MobileBootStage::Transport);
    assert_eq!(error.code, MobileBootFailureCode::TransportUnavailable);
    assert!(error.retryable);
    assert_eq!(error.message, "transport initialization failed");
    assert!(error.message.len() <= 256);
    assert!(!error.to_string().contains(root.path().to_string_lossy().as_ref()));
    assert!(!error.message.contains(root.path().to_string_lossy().as_ref()));
}

#[tokio::test]
async fn retry_after_boot_failure_uses_clean_state() {
    let root = tempfile::tempdir().unwrap();
    let (listener, mobile_config) = occupied_server_config(root.path()).await;
    let first = match MobileNode::boot(mobile_config.clone()).await {
        Ok(node) => {
            shutdown(node).await;
            panic!("occupied listener unexpectedly booted");
        }
        Err(error) => error,
    };
    assert_eq!(first.stage, MobileBootStage::Transport);
    assert_eq!(first.code, MobileBootFailureCode::TransportUnavailable);
    assert!(first.retryable);
    assert_eq!(first.message, "transport initialization failed");
    assert!(!first.to_string().contains(root.path().to_string_lossy().as_ref()));
    drop(listener);

    let node = MobileNode::boot(mobile_config).await.expect("retry after failed partial boot");
    assert_eq!(node.tcp_listen_addresses().len(), 1);
    assert_eq!(node.storage_status().unwrap().recovery, StorageRecoveryOutcome::CleanShutdown);
    assert_eq!(node.app_context.pages().pages_dir(), root.path().join("config/pages"));
    shutdown(node).await;
}

#[tokio::test]
async fn interface_observations_are_generation_scoped() {
    let root = tempfile::tempdir().unwrap();
    let mut mobile_config = config(root.path(), IdentityBackend::PlaintextFile);
    mobile_config
        .interfaces
        .push(MobileInterfaceConfig::TcpServer { bind_address: "127.0.0.1:0".into() });
    let node = MobileNode::boot(mobile_config).await.unwrap();
    let session_generation = node.session_snapshot().await.generation;
    let interfaces = node.facade.list_interfaces().await.unwrap();

    assert!(!interfaces.is_empty());
    assert!(interfaces.iter().all(|interface| {
        interface.observation.connection_generation == Some(session_generation)
    }));
    shutdown(node).await;
}

#[tokio::test]
async fn interface_failures_are_typed() {
    let root = tempfile::tempdir().unwrap();
    let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = unused.local_addr().unwrap();
    drop(unused);
    let mut mobile_config = config(root.path(), IdentityBackend::PlaintextFile);
    mobile_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: address.to_string() });
    let node = MobileNode::boot(mobile_config).await.unwrap();

    let failure = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(failure) = node
                .facade
                .list_interfaces()
                .await
                .unwrap()
                .into_iter()
                .find_map(|interface| interface.failure)
            {
                break failure;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("interface did not expose retry failure");
    assert_eq!(failure.code, InterfaceFailureCode::Retrying);
    assert!(failure.retryable);
    shutdown(node).await;
}

#[tokio::test]
async fn capability_snapshot_uses_current_generation() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();
    let generation = node.session_snapshot().await.generation;
    let active = node.active_capabilities(node.app_context.identity().identity_hash());
    let status = node.facade.query_status().await.unwrap();

    assert_eq!(active.generation(), Some(generation));
    assert_eq!(status.active_capabilities.unwrap().generation, Some(generation));
    shutdown(node).await;
}

#[tokio::test]
async fn capability_disabled_reasons_are_typed() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();
    let active = node.active_capabilities(node.app_context.identity().identity_hash());
    assert!(!active.failures().is_empty());
    assert!(active.failures().iter().all(|failure| {
        matches!(
            failure.kind(),
            CapabilityFailureKind::Unavailable
                | CapabilityFailureKind::Unauthorized
                | CapabilityFailureKind::Degraded
                | CapabilityFailureKind::Unverified
        )
    }));

    let projected = node.facade.query_status().await.unwrap().active_capabilities.unwrap();
    assert!(!projected.failures.is_empty());
    assert!(projected.failures.iter().all(|failure| {
        failure.code != CapabilityFailureCode::Unknown && failure.id.len() <= 128
    }));
    shutdown(node).await;
}

const PEER_A: &str = "00112233445566778899aabbccddeeff";
const PEER_B: &str = "ffeeddccbbaa99887766554433221100";

fn insert_test_outbound(store: &MessagesStore, id: &str, requested_method: &str) {
    let message = MessageRecord {
        id: id.into(),
        source: "11".repeat(16),
        destination: PEER_A.into(),
        title: String::new(),
        content: "route evidence".into(),
        timestamp: 1,
        direction: "out".into(),
        fields: None,
        receipt_status: Some("queued".into()),
        read: true,
    };
    let route = OutboundRouteRecord {
        message_id: id.into(),
        requested_method: requested_method.into(),
        actual_method: requested_method.into(),
        representation: "packet".into(),
        fallback_reason: None,
        correlation_id: id.into(),
        retry_of: None,
        deadline_unix_ms: i64::MAX,
        state: "queued".into(),
        attempt_count: 0,
    };
    store.insert_outbound_message(&message, &route).unwrap();
}

#[test]
fn message_attempt_retains_observed_tcp_route() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("messages.db");
    let id = "aa".repeat(32);
    {
        let store = MessagesStore::open(&path).unwrap();
        insert_test_outbound(&store, &id, "direct");
        let attempt = OutboundAttemptRecord {
            message_id: id.clone(),
            attempt_number: 1,
            started_unix_ms: 10,
            deadline_unix_ms: 100,
            state: "sending".into(),
            route_observation: None,
        };
        let observation = AttemptRouteObservationRecord {
            message_id: id.clone(),
            attempt_number: 1,
            outcome: "observed".into(),
            connection_generation: Some(7),
            observed_at: Some(9),
            next_hop: Some("22".repeat(16)),
            hops: Some(2),
            stale: false,
            interface_id: Some("33".repeat(16)),
            interface_kind: Some("tcp_client".into()),
            interface_generation: Some(7),
            bearer: None,
        };
        assert!(store.begin_outbound_attempt_with_route(&attempt, Some(&observation)).unwrap());
    }

    let reopened = MessagesStore::open(&path).unwrap();
    let attempts = reopened.outbound_attempts(&id).unwrap();
    let observation = attempts[0].route_observation.as_ref().unwrap();
    assert_eq!(observation.outcome, "observed");
    assert_eq!(observation.connection_generation, Some(7));
    assert_eq!(observation.interface_kind.as_deref(), Some("tcp_client"));
    assert_eq!(observation.bearer, None);
    assert_eq!(observation.next_hop.as_deref(), Some("22222222222222222222222222222222"));
    assert_eq!(reopened.outbound_route(&id).unwrap().unwrap().actual_method, "direct");
    drop(reopened);
    let connection = rusqlite::Connection::open(&path).unwrap();
    assert!(
        connection
            .execute(
                "UPDATE outbound_attempt_route_observations SET stale = 1
                 WHERE message_id = ?1 AND attempt_number = 1",
                [&id],
            )
            .is_err(),
        "attached route evidence must be immutable"
    );
}

#[test]
fn missing_route_correlation_remains_unknown() {
    let store = MessagesStore::in_memory().unwrap();
    let id = "bb".repeat(32);
    insert_test_outbound(&store, &id, "direct");
    store
        .begin_outbound_attempt(&OutboundAttemptRecord {
            message_id: id.clone(),
            attempt_number: 1,
            started_unix_ms: 10,
            deadline_unix_ms: 100,
            state: "sending".into(),
            route_observation: None,
        })
        .unwrap();

    let attempt = &store.outbound_attempts(&id).unwrap()[0];
    let observation = attempt.route_observation.as_ref().unwrap();
    assert_eq!(observation.outcome, "unknown");
    assert_eq!(observation.connection_generation, None);
    assert_eq!(observation.interface_id, None);
    assert_eq!(observation.bearer, None);
    assert_eq!(store.outbound_route(&id).unwrap().unwrap().requested_method, "direct");
}

#[test]
fn legacy_attempt_migration_records_unknown_route_evidence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("messages.db");
    let id = "cc".repeat(32);
    {
        let store = MessagesStore::open(&path).unwrap();
        insert_test_outbound(&store, &id, "opportunistic");
        store
            .begin_outbound_attempt(&OutboundAttemptRecord {
                message_id: id.clone(),
                attempt_number: 1,
                started_unix_ms: 10,
                deadline_unix_ms: 100,
                state: "sending".into(),
                route_observation: None,
            })
            .unwrap();
    }
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER outbound_attempt_route_observations_immutable;
             DROP TABLE outbound_attempt_route_observations;
             DELETE FROM schema_migrations
             WHERE id = '2026-08-29-attempt-route-observations-v16';",
        )
        .unwrap();
    drop(connection);

    let migrated = MessagesStore::open(&path).unwrap();
    let observation =
        migrated.outbound_attempts(&id).unwrap()[0].route_observation.clone().unwrap();
    assert_eq!(observation.outcome, "unknown");
    assert_eq!(observation.bearer, None);
}

#[tokio::test]
async fn canonical_peer_starts_one_durable_empty_conversation() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();

    let created = DaemonMessaging::start_conversation(node.facade.as_ref(), PEER_A).await.unwrap();
    let unchanged =
        DaemonMessaging::start_conversation(node.facade.as_ref(), PEER_A).await.unwrap();

    assert_eq!(created.disposition, MessagingDisposition::Created);
    assert_eq!(unchanged.disposition, MessagingDisposition::Unchanged);
    assert_eq!(created.conversation.as_ref().unwrap().peer_hash, PEER_A);
    assert_eq!(created.conversation, unchanged.conversation);
    let page = node.conversation_page(16, None).await.unwrap();
    assert_eq!(page.conversations.len(), 1);
    shutdown(node).await;
}

#[tokio::test]
async fn empty_conversation_survives_restart_without_preview() {
    let root = tempfile::tempdir().unwrap();
    let mobile_config = config(root.path(), IdentityBackend::PlaintextFile);
    let first = MobileNode::boot(mobile_config.clone()).await.unwrap();
    first.start_conversation(PEER_A).await.unwrap();
    shutdown(first).await;

    let second = MobileNode::boot(mobile_config).await.unwrap();
    let page = second.conversation_page(16, None).await.unwrap();
    let conversation = page.conversations.iter().find(|item| item.peer_hash == PEER_A).unwrap();
    assert_eq!(conversation.last_message_content, None);
    assert_eq!(conversation.last_message_timestamp, None);
    assert_eq!(conversation.unread_count, 0);
    assert_eq!(conversation.message_count, 0);
    let json = serde_json::to_value(conversation).unwrap();
    assert!(json.get("route").is_none());
    assert!(json.get("connectivity").is_none());
    shutdown(second).await;
}

#[tokio::test]
async fn contact_alias_resolves_in_conversation_summary() {
    let root = tempfile::tempdir().unwrap();
    let mobile_config = config(root.path(), IdentityBackend::PlaintextFile);
    let node = MobileNode::boot(mobile_config.clone()).await.unwrap();
    node.start_conversation(PEER_A).await.unwrap();
    node.app_context
        .discovery()
        .accept_announce_with_details(
            PEER_A.into(),
            1,
            Some("Canonical Name".into()),
            Some("canonical_announce".into()),
            None,
        )
        .unwrap();

    let canonical = node.conversation_page(16, None).await.unwrap();
    assert_eq!(canonical.conversations[0].peer_name.as_deref(), Some("Canonical Name"));
    node.set_contact(PEER_A, "Local Alias").await.unwrap();
    let aliased = node.conversation_page(16, None).await.unwrap();
    assert_eq!(aliased.conversations[0].peer_name.as_deref(), Some("Local Alias"));

    node.start_conversation(PEER_B).await.unwrap();
    let fallback = node
        .conversation_page(16, None)
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|item| item.peer_hash == PEER_B)
        .unwrap();
    assert_eq!(fallback.peer_name.as_deref(), Some(&PEER_B[..12]));
    shutdown(node).await;

    let reopened = MobileNode::boot(mobile_config).await.unwrap();
    let restored = reopened.conversation_page(16, None).await.unwrap();
    let restored = restored.conversations.iter().find(|item| item.peer_hash == PEER_A).unwrap();
    assert_eq!(restored.peer_name.as_deref(), Some("Local Alias"));
    shutdown(reopened).await;
}

#[tokio::test]
async fn alias_mutation_emits_scoped_invalidation() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), IdentityBackend::PlaintextFile)).await.unwrap();
    node.start_conversation(PEER_A).await.unwrap();
    node.set_contact(PEER_A, "First").await.unwrap();
    let mut events = DaemonEvents::subscribe_messages(node.facade.as_ref(), &[]).await.unwrap();

    node.set_contact(PEER_A, "Second").await.unwrap();
    let changed = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let DaemonEvent::ConversationInvalidated { invalidation } =
                events.recv().await.unwrap()
            {
                break invalidation;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(changed.peer_hash, PEER_A);
    assert_eq!(changed.reason, ConversationInvalidationReason::ContactAliasChanged);

    DaemonMessaging::set_contact(node.facade.as_ref(), PEER_B, None, Some("notes only"))
        .await
        .unwrap();
    let unrelated = tokio::time::timeout(Duration::from_millis(50), async {
        loop {
            if let DaemonEvent::ConversationInvalidated { invalidation } =
                events.recv().await.unwrap()
            {
                break invalidation;
            }
        }
    })
    .await;
    assert!(unrelated.is_err(), "notes for an unrelated peer invalidated conversations");

    node.remove_contact(PEER_A).await.unwrap();
    let removed = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let DaemonEvent::ConversationInvalidated { invalidation } =
                events.recv().await.unwrap()
            {
                break invalidation;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(removed.peer_hash, PEER_A);
    assert_eq!(removed.reason, ConversationInvalidationReason::ContactAliasRemoved);
    assert_ne!(removed.peer_hash, PEER_B);
    shutdown(node).await;
}
