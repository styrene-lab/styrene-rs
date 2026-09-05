use std::time::Duration;
use styrene_ipc::traits::DaemonStatus;
use styrene_ipc::types::*;
use styrened::daemon::{self, DaemonConfig2};
use styrened::services::{config::ConfigService, interfaces::InterfaceService};
use styrened::transport::null_transport::NullTransport;

fn mutation(
    inventory: &InterfaceInventory,
    action: &str,
    settings: InterfaceSettings,
) -> InterfaceMutation {
    let mut request = InterfaceMutation::default();
    request.expected_revision = inventory.revision.clone();
    request.action = action.into();
    request.settings = settings;
    request
}
fn listener() -> InterfaceSettings {
    let mut s = InterfaceSettings::default();
    s.name = "Test listener".into();
    s.kind = "tcp_server".into();
    s.host = "127.0.0.1".into();
    s.enabled = true;
    s
}
#[tokio::test]
async fn tcp_crud_closes_children_and_survives_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.toml");
    std::fs::write(
        &path,
        "interfaces_managed = true\ninterfaces = []\ncustom_site = 'preserve me'\n",
    )
    .unwrap();
    let start = || DaemonConfig2 {
        db: None,
        config: Some(path.clone()),
        identity: None,
        socket: Some(root.path().join("control.sock")),
        ephemeral: true,
    };
    let handle = daemon::start(start()).await.unwrap();
    handle.app_context.config().load(&path).unwrap();
    let d = &handle.daemon_facade;
    let initial = d.interface_inventory().await.unwrap();
    assert!(initial.entries.is_empty());
    let created = d.mutate_interface(mutation(&initial, "create", listener())).await.unwrap();
    let id = created.entries[0].settings.id.clone();
    assert!(
        d.mutate_interface(mutation(&initial, "create", listener())).await.is_err(),
        "stale creates must not duplicate interfaces"
    );
    let ready = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = d.interface_inventory().await.unwrap();
            if view.entries[0].state == "listening" {
                break view;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let endpoint = ready.entries[0].local_endpoint.clone().unwrap();
    let mut socket = tokio::net::TcpStream::connect(&endpoint).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if handle
                .app_context
                .transport()
                .interface_snapshots()
                .await
                .iter()
                .any(|s| s.parent.is_some())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut settings = ready.entries[0].settings.clone();
    settings.enabled = false;
    settings.name = "Renamed listener".into();
    let disabled = d.mutate_interface(mutation(&ready, "update", settings)).await.unwrap();
    assert_eq!(disabled.entries[0].state, "disabled");
    use tokio::io::AsyncReadExt;
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(std::fs::read_to_string(&path).unwrap().contains("preserve me"));
    handle.shutdown().await;
    let handle = daemon::start(start()).await.unwrap();
    handle.app_context.config().load(&path).unwrap();
    let restored = handle.daemon_facade.interface_inventory().await.unwrap();
    assert_eq!(restored.entries[0].settings.id, id);
    assert_eq!(restored.entries[0].state, "disabled");
    assert!(
        handle.app_context.transport().interface_snapshots().await.is_empty(),
        "restart must not recreate the CLI default listener"
    );
    let deleted = handle
        .daemon_facade
        .mutate_interface(mutation(&restored, "delete", restored.entries[0].settings.clone()))
        .await
        .unwrap();
    assert!(deleted.entries.is_empty());
    assert!(styrened::config::DaemonConfig::from_path(&path).unwrap().interfaces.is_empty());
    handle.shutdown().await;
}
#[tokio::test]
async fn failed_persistence_and_invalid_inputs_do_not_change_inventory() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.toml");
    std::fs::write(&path, "interfaces=[]\n").unwrap();
    let config = ConfigService::with_path(&path).unwrap();
    let service = InterfaceService::default();
    let transport = NullTransport::new();
    let initial = service.list(&config, &transport).await;
    let mut invalid = listener();
    invalid.host = "https://example.com".into();
    assert!(
        service.mutate(&config, &transport, mutation(&initial, "create", invalid)).await.is_err()
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        service
            .mutate(&config, &transport, mutation(&initial, "create", listener()))
            .await
            .is_err()
    );
    assert_eq!(service.list(&config, &transport).await, initial);
}

#[tokio::test]
async fn tcp_client_connect_disable_reenable_and_authorization() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.toml");
    std::fs::write(&path, "interfaces_managed=true\ninterfaces=[]\n").unwrap();
    let handle = daemon::start(DaemonConfig2 {
        db: None,
        config: Some(path.clone()),
        identity: None,
        socket: Some(root.path().join("control.sock")),
        ephemeral: true,
    })
    .await
    .unwrap();
    handle.app_context.config().load(&path).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut settings = InterfaceSettings::default();
    settings.kind = "tcp_client".into();
    settings.name = "Loopback peer".into();
    settings.host = "127.0.0.1".into();
    settings.port = listener.local_addr().unwrap().port();
    settings.enabled = true;
    let initial = handle.daemon_facade.interface_inventory().await.unwrap();
    let denied =
        styrened::daemon_facade::DaemonFacade::new(handle.app_context.clone(), "aa".repeat(16));
    assert!(denied.mutate_interface(mutation(&initial, "create", settings.clone())).await.is_err());
    let inventory = handle
        .daemon_facade
        .mutate_interface(mutation(&initial, "create", settings))
        .await
        .unwrap();
    let (mut peer, _) =
        tokio::time::timeout(Duration::from_secs(5), listener.accept()).await.unwrap().unwrap();
    let mut settings = inventory.entries[0].settings.clone();
    settings.enabled = false;
    let disabled = handle
        .daemon_facade
        .mutate_interface(mutation(&inventory, "update", settings.clone()))
        .await
        .unwrap();
    use tokio::io::AsyncReadExt;
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), peer.read(&mut byte)).await.unwrap().unwrap(),
        0
    );
    settings.enabled = true;
    let enabled = handle
        .daemon_facade
        .mutate_interface(mutation(&disabled, "update", settings))
        .await
        .unwrap();
    let (_peer, _) =
        tokio::time::timeout(Duration::from_secs(5), listener.accept()).await.unwrap().unwrap();
    assert_eq!(enabled.entries[0].settings.id, inventory.entries[0].settings.id);
    handle.shutdown().await;
}
