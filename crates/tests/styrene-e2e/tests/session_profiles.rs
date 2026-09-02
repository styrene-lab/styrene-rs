//! Live and Embedded sessions expose one daemon contract.
//!
//! An Embedded session starts a `styrened` runtime in-process and reaches it
//! over a private socket. A Live session opened against that same endpoint
//! must return the same typed records, and closing the Embedded session must
//! take the endpoint away rather than leave a runtime behind.

use styrene_session::{EmbeddedConfig, Session, SessionError, SessionProfile};

fn comparable_capabilities(
    status: &styrene_ipc::types::DaemonStatusInfo,
) -> Option<(u16, Vec<String>, Vec<String>)> {
    status.active_capabilities.as_ref().map(|capabilities| {
        (
            capabilities.version,
            capabilities.runtime.clone(),
            capabilities.authorized_operations.clone(),
        )
    })
}

#[tokio::test]
async fn live_and_embedded_sessions_return_equivalent_records() {
    let data = tempfile::tempdir().expect("data dir");
    let mut embedded = Session::embedded(EmbeddedConfig {
        db: Some(data.path().join("messages.db")),
        config: None,
        identity: None,
        ephemeral: true,
    })
    .await
    .expect("embedded runtime starts");
    assert_eq!(embedded.profile(), SessionProfile::Embedded);
    let endpoint = embedded.metadata().endpoint.clone();
    assert!(endpoint.exists(), "embedded session owns a private socket");

    let mut live = Session::live(&endpoint).await.expect("live session over the embedded endpoint");
    assert_eq!(live.profile(), SessionProfile::Live);
    assert!(live.generation() > embedded.generation());
    // Two IPC connections to one daemon carry distinct daemon connection generations.
    assert_ne!(live.metadata().daemon_generation, embedded.metadata().daemon_generation);

    let embedded_identity = embedded.client().identity().await.expect("embedded identity");
    let live_identity = live.client().identity().await.expect("live identity");
    assert_eq!(embedded_identity, live_identity);

    let embedded_status = embedded.client().status().await.expect("embedded status");
    let live_status = live.client().status().await.expect("live status");
    assert_eq!(embedded_status.daemon_version, live_status.daemon_version);
    assert_eq!(embedded_status.rns_initialized, live_status.rns_initialized);
    assert_eq!(comparable_capabilities(&embedded_status), comparable_capabilities(&live_status));
    assert!(comparable_capabilities(&live_status).is_some(), "daemon advertises capabilities");

    let embedded_devices = embedded.client().devices(false).await.expect("embedded devices");
    let live_devices = live.client().devices(false).await.expect("live devices");
    assert_eq!(embedded_devices, live_devices);

    // Closing the Embedded session shuts down its runtime and endpoint. The
    // Live session reports a typed failure; nothing restarts on its behalf.
    embedded.close().await;
    embedded.close().await;
    assert!(!endpoint.exists(), "embedded shutdown removes its socket");
    assert!(live.client().status().await.is_err());
    assert!(matches!(
        Session::live(&endpoint).await,
        Err(SessionError::Connect { profile: SessionProfile::Live, .. })
    ));
    live.close().await;
}
