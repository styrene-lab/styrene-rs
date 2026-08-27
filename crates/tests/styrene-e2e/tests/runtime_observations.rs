use std::time::Duration;

use styrene_e2e::node::TestNodeBuilder;
use styrene_ipc::traits::DaemonStatus;
use styrene_ipc::types::ObservationSource;
use styrened::daemon_facade::DaemonFacade;
use styrened::services::status::InterfaceRecord;

fn configured_listener() -> InterfaceRecord {
    InterfaceRecord {
        kind: "tcp_server".into(),
        enabled: true,
        host: Some("127.0.0.1".into()),
        port: Some(0),
        name: Some("listener".into()),
    }
}

#[tokio::test]
async fn started_runtime_interface_is_visible_with_its_identity() {
    let node = TestNodeBuilder::new("runtime-interface").tcp_server("127.0.0.1:0").build().await;
    let runtime = node.app_context.transport().interface_stats().await;
    assert!(!runtime.is_empty(), "test transport did not start an interface");
    let facade = DaemonFacade::new(node.app_context.clone(), node.identity_hash.clone());

    let listed = facade.list_interfaces().await.unwrap();

    assert_eq!(listed.len(), runtime.len());
    for hash in runtime.keys() {
        let expected = hex::encode(hash.as_slice());
        assert!(
            listed.iter().any(|interface| interface.hash == expected),
            "runtime interface {expected} missing from daemon observations"
        );
    }
    assert!(listed.iter().all(|interface| {
        interface.observation.source == ObservationSource::RuntimeInterfaceRegistry
            && interface.observation.age_secs == Some(0)
            && !interface.observation.stale
    }));
}

#[tokio::test]
async fn listener_reports_actual_ephemeral_endpoint() {
    let node = TestNodeBuilder::new("runtime-endpoint").tcp_server("127.0.0.1:0").build().await;
    let actual = node.listen_addr.expect("bound listener endpoint");
    node.app_context.status().replace_interfaces(vec![configured_listener()]);
    let facade = DaemonFacade::new(node.app_context.clone(), node.identity_hash.clone());

    let listed = facade.list_interfaces().await.unwrap();
    let listener = listed.iter().find(|interface| interface.kind == "tcp_server").unwrap();

    assert_eq!(listener.host.as_deref(), Some(actual.ip().to_string().as_str()));
    assert_eq!(listener.port, Some(actual.port()));
    assert!(!listener.hash.is_empty());
}

#[tokio::test]
async fn counters_remain_attached_to_runtime_interface_identities() {
    let server = TestNodeBuilder::new("counter-server").tcp_server("127.0.0.1:0").build().await;
    let client = TestNodeBuilder::new("counter-client")
        .tcp_client(server.listen_addr.expect("server endpoint"))
        .build()
        .await;
    client.announce().await;
    let mut runtime = server.app_context.transport().interface_stats().await;
    for _ in 0..40 {
        if runtime.values().any(|stats| stats.rx_bytes > 0) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        runtime = server.app_context.transport().interface_stats().await;
    }
    assert!(!runtime.is_empty());
    assert!(runtime.values().any(|stats| stats.rx_bytes > 0), "announce traffic not observed");
    server.app_context.status().replace_interfaces(
        (0..runtime.len())
            .map(|index| InterfaceRecord {
                name: Some(format!("configured-{index}")),
                ..configured_listener()
            })
            .collect(),
    );
    let facade = DaemonFacade::new(server.app_context.clone(), server.identity_hash.clone());

    let mut expected: Vec<_> = runtime
        .iter()
        .map(|(hash, stats)| (hex::encode(hash.as_slice()), stats.tx_bytes, stats.rx_bytes))
        .collect();
    let mut actual: Vec<_> = facade
        .list_interfaces()
        .await
        .unwrap()
        .into_iter()
        .map(|interface| (interface.hash, interface.tx_bytes, interface.rx_bytes))
        .collect();
    expected.sort();
    actual.sort();
    assert_eq!(actual, expected, "runtime counters were detached or evenly divided");
}

#[tokio::test]
async fn listener_reports_connected_peer_count() {
    let server = TestNodeBuilder::new("peer-server").tcp_server("127.0.0.1:0").build().await;
    let _client = TestNodeBuilder::new("peer-client")
        .tcp_client(server.listen_addr.expect("server endpoint"))
        .build()
        .await;
    server.app_context.status().replace_interfaces(vec![configured_listener()]);
    let facade = DaemonFacade::new(server.app_context.clone(), server.identity_hash.clone());

    let mut listed = facade.list_interfaces().await.unwrap();
    for _ in 0..40 {
        if listed.iter().any(|interface| interface.peers_connected == 1) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        listed = facade.list_interfaces().await.unwrap();
    }
    let listener = listed.iter().find(|interface| interface.kind == "tcp_server").unwrap();
    assert_eq!(listener.peers_connected, 1);
}
