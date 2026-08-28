use rns_core::destination::{DestinationName, NAME_HASH_LENGTH, SingleOutputDestination};
use rns_core::identity::PrivateIdentity;
use rns_core::packet::PacketDataBuffer;
use rns_core::transport::core_transport::AnnounceEvent;
use std::sync::{Arc, Mutex};
use styrene_ipc::types::{DaemonEvent, DiscoveredCapability};
use styrened::services::{DiscoveryService, EventService};
use styrened::storage::messages::MessagesStore;
use styrened::storage::standard_propagation::StandardPropagationPolicy;
use styrened::transport::mock_transport::MockTransport;
use styrened::workers::announce::{spawn_announce_worker, spawn_announce_worker_with_milestones};

#[tokio::test]
async fn authentic_nomadnet_announce_event_projects_native_page_host_capability() {
    let transport = Arc::new(MockTransport::new_default());
    let discovery = Arc::new(DiscoveryService::new());
    let events = Arc::new(EventService::new());
    let mut event_rx = events.subscribe_devices();
    let worker = spawn_announce_worker(transport.clone(), discovery.clone(), events);

    let identity = PrivateIdentity::new_from_name("native-nomadnet-host");
    let name = DestinationName::new("nomadnetwork", "node");
    let destination = SingleOutputDestination::new(*identity.as_identity(), name);
    let destination_hash = hex::encode(destination.desc.address_hash.as_slice());
    let mut name_hash = [0u8; NAME_HASH_LENGTH];
    name_hash.copy_from_slice(&name.hash.as_slice()[..NAME_HASH_LENGTH]);
    transport.inject_announce(AnnounceEvent {
        destination: Arc::new(tokio::sync::Mutex::new(destination)),
        app_data: PacketDataBuffer::new_from_slice(b"Native NomadNet host"),
        ratchet: None,
        name_hash,
        hops: 1,
        interface: b"test-interface".to_vec(),
    });

    tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .expect("device event timeout")
        .expect("device event");
    let device = discovery.device(&destination_hash).expect("discovered native page host");
    assert_eq!(device.discovered_capabilities, vec![DiscoveredCapability::NativeNomadNetHost]);
    worker.abort();
}

#[tokio::test]
async fn valid_propagation_announce_projects_typed_name_and_inactive_state() {
    let transport = Arc::new(MockTransport::new_default());
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    let discovery = Arc::new(DiscoveryService::with_store(store.clone()));
    let events = Arc::new(EventService::new());
    let mut event_rx = events.subscribe_devices();
    let worker = spawn_announce_worker(transport.clone(), discovery.clone(), events);
    let identity = PrivateIdentity::new_from_name("standard-propagation-host");
    let name = DestinationName::new("lxmf", "propagation");
    let destination = SingleOutputDestination::new(*identity.as_identity(), name);
    let identity_hash: [u8; 16] = identity.address_hash().as_slice().try_into().unwrap();
    let propagation_destination: [u8; 16] =
        destination.desc.address_hash.as_slice().try_into().unwrap();
    let destination_hash = hex::encode(destination.desc.address_hash.as_slice());
    let mut name_hash = [0u8; NAME_HASH_LENGTH];
    name_hash.copy_from_slice(name.as_name_hash_slice());
    let app_data = lxmf::propagation_announce::StandardPropagationAnnounce::inactive(
        1_700_000_000,
        Some("Propagation Peer"),
    )
    .unwrap()
    .encode()
    .unwrap();
    transport.inject_announce(AnnounceEvent {
        destination: Arc::new(tokio::sync::Mutex::new(destination)),
        app_data: PacketDataBuffer::new_from_slice(&app_data),
        ratchet: None,
        name_hash,
        hops: 1,
        interface: b"test-interface".to_vec(),
    });

    tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .expect("device event timeout")
        .expect("device event");
    let device = discovery.device(&destination_hash).expect("propagation host");
    assert_eq!(device.name, "Propagation Peer");
    assert_eq!(device.standard_lxmf_propagation_active, Some(false));
    assert_eq!(
        device.discovered_capabilities,
        vec![DiscoveredCapability::StandardLxmfPropagationHost]
    );
    let changed = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .expect("standard propagation event timeout")
        .expect("standard propagation event");
    assert!(matches!(changed, DaemonEvent::StandardPropagationChanged { .. }));
    let observation = store
        .lock()
        .unwrap()
        .standard_propagation_observation(
            1_700_000_001,
            StandardPropagationPolicy {
                queue_max_count: 4096,
                queue_max_bytes: 16 * 1024 * 1024,
                expiry_secs: 30 * 24 * 60 * 60,
            },
        )
        .unwrap();
    assert_eq!(observation.peers.len(), 1);
    assert_eq!(observation.peers[0].identity_hash, identity_hash);
    assert_eq!(observation.peers[0].propagation_destination, Some(propagation_destination));
    assert!(!observation.peers[0].enabled);
    worker.abort();
}

#[tokio::test]
async fn invalid_propagation_app_data_fails_closed() {
    let transport = Arc::new(MockTransport::new_default());
    let discovery = Arc::new(DiscoveryService::new());
    let events = Arc::new(EventService::new());
    let (milestone_tx, mut milestone_rx) = tokio::sync::mpsc::unbounded_channel();
    let worker = spawn_announce_worker_with_milestones(
        transport.clone(),
        discovery.clone(),
        events,
        milestone_tx,
    );
    let identity = PrivateIdentity::new_from_name("invalid-propagation-host");
    let name = DestinationName::new("lxmf", "propagation");
    let destination = SingleOutputDestination::new(*identity.as_identity(), name);
    let destination_hash = hex::encode(destination.desc.address_hash.as_slice());
    let mut name_hash = [0u8; NAME_HASH_LENGTH];
    name_hash.copy_from_slice(name.as_name_hash_slice());
    transport.inject_announce(AnnounceEvent {
        destination: Arc::new(tokio::sync::Mutex::new(destination)),
        app_data: PacketDataBuffer::new_from_slice(b"not propagation metadata"),
        ratchet: None,
        name_hash,
        hops: 1,
        interface: b"test-interface".to_vec(),
    });
    let milestone = tokio::time::timeout(std::time::Duration::from_secs(1), milestone_rx.recv())
        .await
        .expect("announce processing milestone timeout")
        .expect("announce processing milestone");
    assert_eq!(milestone.destination_hash, destination_hash);
    assert!(!milestone.accepted);
    assert!(discovery.device(&destination_hash).is_none());
    worker.abort();
}
