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

use std::sync::{Arc, Mutex};
use styrene_ipc::error::IpcError;
use styrene_ipc::traits::Daemon;
use styrene_ipc::types::SendChatRequest;
use styrened::app_context::AppContext;
use styrened::daemon_facade::DaemonFacade;
use styrened::storage::messages::MessagesStore;
use styrened::transport::mesh_transport::MeshTransport;
use styrened::transport::null_transport::NullTransport;

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
    daemon.set_auto_reply("all", Some("Away"), Some(120)).await.unwrap();

    // Get
    let config = daemon.query_auto_reply().await.unwrap();
    assert_eq!(config.mode, "all");
    assert_eq!(config.message, Some("Away".into()));
    assert_eq!(config.cooldown_secs, Some(120));
}

#[tokio::test]
async fn daemon_trait_object_blocked_caller() {
    use styrene_rbac::RbacPolicy;
    use styrened::services::PolicyService;

    let mut policy = RbacPolicy::default();
    policy.block("deadbeef");

    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    let node_store = Arc::new(styrene_services::node_store::NodeStore::in_memory().unwrap());
    let ctx = Arc::new(AppContext::with_policy(
        transport,
        "daemon-identity".into(),
        store,
        node_store,
        PolicyService::new(policy),
    ));

    let daemon: Arc<dyn Daemon> =
        Arc::new(DaemonFacade::new(ctx, "deadbeef11112222333344445555aaaa".into()));

    let result = daemon.query_status().await;
    assert!(matches!(result, Err(IpcError::Unavailable { .. })));
}

#[tokio::test]
async fn multiple_facades_same_context() {
    use styrene_rbac::{RbacPolicy, RosterEntry};
    use styrened::services::PolicyService;

    let mut policy = RbacPolicy::default();
    policy.add_entry(
        RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", styrene_rbac::Role::Admin)
            .with_label("admin"),
    );

    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    let node_store = Arc::new(styrene_services::node_store::NodeStore::in_memory().unwrap());
    let ctx = Arc::new(AppContext::with_policy(
        transport,
        "daemon-identity".into(),
        store,
        node_store,
        PolicyService::new(policy),
    ));

    // Two facades with different caller identities
    let admin_facade: Arc<dyn Daemon> =
        Arc::new(DaemonFacade::new(ctx.clone(), "aaaa1111bbbb2222cccc3333dddd4444".into()));
    let peer_facade: Arc<dyn Daemon> =
        Arc::new(DaemonFacade::new(ctx.clone(), "bbbb2222cccc3333dddd4444eeee5555".into()));

    // Both can query status
    assert!(admin_facade.query_status().await.is_ok());
    assert!(peer_facade.query_status().await.is_ok());

    // Admin can exec (returns Internal in test mode — no transport)
    let admin_exec = admin_facade.exec("dest", "ls", vec![], None).await;
    assert!(matches!(admin_exec, Err(IpcError::Internal { .. })));

    // Peer cannot exec
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

    // list_tunnels returns Ok(empty) because TunnelService is wired but has no peers.
    assert!(daemon.list_tunnels().await.unwrap().is_empty());
    // send_chat returns Internal error (no transport) rather than NotImplemented
    assert!(matches!(
        daemon.send_chat(SendChatRequest::default()).await,
        Err(IpcError::Internal { .. })
    ));
    // query_path_info returns InvalidRequest for bad hash, not NotImplemented
    assert!(matches!(daemon.query_path_info("abc").await, Err(IpcError::InvalidRequest { .. })));

    // These should now work (not NotImplemented)
    let _results = daemon.search_messages("test", None, 10).await.expect("search works");
    let _convos = daemon.query_conversations(false).await.expect("conversations work");
    let _contacts = daemon.query_contacts().await.expect("contacts work");
}
