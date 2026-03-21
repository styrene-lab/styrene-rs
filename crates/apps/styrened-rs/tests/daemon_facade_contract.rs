//! Daemon facade contract tests — verifying the DaemonFacade can be used
//! as `Arc<dyn Daemon>` by Unix socket IPC consumers.
//!
//! Package J — dependent unlock validation.
//!
//! These tests prove:
//! 1. DaemonFacade implements the full Daemon composite trait
//! 2. It can be held behind Arc<dyn Daemon> (the IPC handler's view)
//! 3. Auth enforcement works through the trait object
//! 4. Real and stubbed methods are accessible through the trait
//! 5. Multiple facades can coexist (different callers, same AppContext)

use reticulum_daemon::app_context::AppContext;
use reticulum_daemon::daemon_facade::DaemonFacade;
use reticulum_daemon::storage::messages::MessagesStore;
use reticulum_daemon::transport::mesh_transport::MeshTransport;
use reticulum_daemon::transport::null_transport::NullTransport;
use std::sync::{Arc, Mutex};
use styrene_ipc::error::IpcError;
use styrene_ipc::traits::Daemon;
use styrene_ipc::types::SendChatRequest;

fn make_ctx() -> Arc<AppContext> {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    Arc::new(AppContext::new(transport, "daemon-identity".into(), store))
}

#[test]
fn facade_usable_as_arc_dyn_daemon() {
    let ctx = make_ctx();
    let facade = DaemonFacade::new(ctx, "local".into());
    let daemon: Arc<dyn Daemon> = Arc::new(facade);
    // The IPC handler would hold this Arc<dyn Daemon>
    let _ = daemon;
}

#[tokio::test]
async fn daemon_trait_object_query_status() {
    let ctx = make_ctx();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "caller".into()));
    let status = daemon.query_status().await.unwrap();
    assert!(!status.rns_initialized);
    assert_eq!(status.device_count, 0);
}

#[tokio::test]
async fn daemon_trait_object_query_identity() {
    let ctx = make_ctx();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "caller".into()));
    let identity = daemon.query_identity().await.unwrap();
    assert_eq!(identity.identity_hash, "daemon-identity");
}

#[tokio::test]
async fn daemon_trait_object_announce() {
    let ctx = make_ctx();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "caller".into()));
    assert!(daemon.announce().await.unwrap());
}

#[tokio::test]
async fn daemon_trait_object_auto_reply_roundtrip() {
    let ctx = make_ctx();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "caller".into()));

    // Set
    daemon
        .set_auto_reply("all", Some("Away"), Some(120))
        .await
        .unwrap();

    // Get
    let config = daemon.query_auto_reply().await.unwrap();
    assert_eq!(config.mode, "all");
    assert_eq!(config.message, Some("Away".into()));
    assert_eq!(config.cooldown_secs, Some(120));
}

#[tokio::test]
async fn daemon_trait_object_blocked_caller() {
    let ctx = make_ctx();
    ctx.auth().block("evil");
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "evil".into()));

    let result = daemon.query_status().await;
    assert!(matches!(result, Err(IpcError::Unavailable { .. })));
}

#[tokio::test]
async fn multiple_facades_same_context() {
    let ctx = make_ctx();

    // Two facades with different caller identities
    let admin_facade: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx.clone(), "admin".into()));
    let peer_facade: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx.clone(), "peer".into()));

    // Set admin as Operator
    ctx.auth()
        .set_role("admin", reticulum_daemon::services::Role::Operator);

    // Both can query status
    assert!(admin_facade.query_status().await.is_ok());
    assert!(peer_facade.query_status().await.is_ok());

    // Only admin can exec (but it's not implemented, so we get NotImplemented not Unavailable)
    let admin_exec = admin_facade.exec("dest", "ls", vec![], None).await;
    assert!(matches!(admin_exec, Err(IpcError::NotImplemented { .. })));

    let peer_exec = peer_facade.exec("dest", "ls", vec![], None).await;
    assert!(matches!(peer_exec, Err(IpcError::Unavailable { .. })));
}

#[tokio::test]
async fn daemon_trait_object_query_devices() {
    let ctx = make_ctx();

    // Discover a device
    ctx.discovery()
        .accept_announce_with_details("node1".into(), 1000, Some("Hub".into()), None, None)
        .unwrap();

    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "caller".into()));
    let devices = daemon.query_devices(false).await.unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "Hub");
}

#[tokio::test]
async fn daemon_trait_object_not_implemented_methods() {
    let ctx = make_ctx();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "caller".into()));

    // These should all return NotImplemented, not panic
    assert!(matches!(
        daemon.list_tunnels().await,
        Err(IpcError::NotImplemented { .. })
    ));
    // send_chat returns Internal error (no transport) rather than NotImplemented
    assert!(matches!(
        daemon.send_chat(SendChatRequest::default()).await,
        Err(IpcError::Internal { .. })
    ));
    assert!(matches!(
        daemon.query_path_info("abc").await,
        Err(IpcError::NotImplemented { .. })
    ));

    // These should now work (not NotImplemented)
    let _results = daemon.search_messages("test", None, 10).await.expect("search works");
    let _convos = daemon.query_conversations(false).await.expect("conversations work");
    let _contacts = daemon.query_contacts().await.expect("contacts work");
}
