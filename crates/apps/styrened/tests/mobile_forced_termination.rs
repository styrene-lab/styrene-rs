use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rns_core::destination::{DestinationName, SingleOutputDestination};
use rns_core::identity::PrivateIdentity;
use styrene_ipc::traits::DaemonIdentity;
use styrened::mobile::{IdentityBackend, MobileConfig, MobileNode};
use styrened::storage::messages::{
    MOBILE_STORAGE_SCHEMA_VERSION, MessageRecord, MessagesStore, OutboundAttemptRecord,
    OutboundRouteRecord, StorageCommitKind, StorageOpenOutcome, StorageRecoveryOutcome,
};

const PEER: &str = "00112233445566778899aabbccddeeff";
const MESSAGE_ID: &str = "committed-mobile-message";
const CHILD_MODE: &str = "STYRENE_MOBILE_TERMINATION_CHILD";
const CHILD_ROOT: &str = "STYRENE_MOBILE_TERMINATION_ROOT";
const READY_DEADLINE: Duration = Duration::from_secs(15);
const TERMINATION_DEADLINE: Duration = Duration::from_secs(5);

fn config(root: &Path) -> MobileConfig {
    MobileConfig {
        config_dir: root.join("config"),
        data_dir: root.join("data"),
        hub_address: None,
        hub_delivery_hash: None,
        display_name: Some("Termination Test Node".into()),
        identity_backend: IdentityBackend::PlaintextFile,
        interfaces: Vec::new(),
        enable_rnode_channel: false,
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

struct OwnedChild(Option<Child>);

impl OwnedChild {
    fn terminate(&mut self) {
        let child = self.0.as_mut().expect("runner-owned child is present");
        child.kill().expect("terminate runner-owned child");
        let deadline = Instant::now() + TERMINATION_DEADLINE;
        loop {
            if child.try_wait().expect("wait for runner-owned child").is_some() {
                self.0 = None;
                return;
            }
            assert!(Instant::now() < deadline, "runner-owned child missed termination deadline");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_child(test_name: &str, mode: &str, root: &Path) -> OwnedChild {
    let child = Command::new(std::env::current_exe().expect("current integration test binary"))
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env(CHILD_MODE, mode)
        .env(CHILD_ROOT, root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn isolated mobile child");
    OwnedChild(Some(child))
}

fn wait_for_ready(child: &mut OwnedChild, marker: &Path) -> String {
    let deadline = Instant::now() + READY_DEADLINE;
    loop {
        if let Ok(value) = std::fs::read_to_string(marker) {
            return value;
        }
        if let Some(status) = child.0.as_mut().unwrap().try_wait().expect("inspect child") {
            panic!("mobile child exited before readiness: {status}");
        }
        assert!(Instant::now() < deadline, "mobile child missed readiness deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn child_root() -> PathBuf {
    PathBuf::from(std::env::var_os(CHILD_ROOT).expect("child storage root"))
}

fn publish_ready(root: &Path, value: &str) {
    let pending = root.join("ready.pending");
    std::fs::write(&pending, value).expect("write readiness marker");
    std::fs::rename(pending, root.join("ready")).expect("publish readiness marker atomically");
}

async fn select_propagation_destination(node: &MobileNode) -> String {
    let now = rns_core::transport::time::now_epoch_secs_i64();
    let identity = PrivateIdentity::new_from_name("forced termination propagation peer");
    let destination = SingleOutputDestination::new(
        *identity.as_identity(),
        DestinationName::new("lxmf", "propagation"),
    )
    .desc
    .address_hash;
    let mut identity_hash = [0; 16];
    identity_hash.copy_from_slice(identity.address_hash().as_slice());
    let mut destination_hash = [0; 16];
    destination_hash.copy_from_slice(destination.as_slice());
    let mut metadata = lxmf::propagation_announce::StandardPropagationAnnounce::active(
        now,
        Some("Termination Propagation Peer"),
        256,
        4_000,
    )
    .unwrap();
    metadata.stamp_cost = 0;
    metadata.stamp_cost_flexibility = 0;
    metadata.peering_cost = 0;
    node.app_context
        .discovery()
        .accept_standard_propagation_announce(
            hex::encode(destination_hash),
            identity_hash,
            destination_hash,
            now,
            &metadata,
        )
        .expect("persist propagation candidate");
    let destination = hex::encode(destination_hash);
    let selected = node
        .select_propagation_destination(&destination)
        .await
        .expect("persist propagation selection");
    assert!(selected.ready);
    destination
}

async fn committed_child(root: &Path) {
    let node = MobileNode::boot(config(root)).await.expect("boot committed-state child");
    let identity =
        DaemonIdentity::query_identity(node.facade.as_ref()).await.expect("query child identity");
    node.start_conversation(PEER).await.expect("commit conversation shell");
    node.set_contact(PEER, "Committed Contact").await.expect("commit contact");
    node.set_draft(PEER, "committed draft").await.expect("commit draft");
    let propagation_destination = select_propagation_destination(&node).await;
    node.app_context
        .store()
        .lock()
        .unwrap()
        .insert_message(&MessageRecord {
            id: MESSAGE_ID.into(),
            source: PEER.into(),
            destination: identity.identity_hash.clone(),
            title: String::new(),
            content: "committed content".into(),
            timestamp: 1,
            direction: "in".into(),
            fields: None,
            receipt_status: None,
            read: false,
        })
        .expect("commit message");
    publish_ready(root, &format!("{}\n{propagation_destination}", identity.identity_hash));
    tokio::time::sleep(Duration::from_secs(120)).await;
}

async fn interrupted_child(root: &Path) {
    let node = MobileNode::boot(config(root)).await.expect("boot interrupted-work child");
    node.start_conversation(PEER).await.expect("commit pre-interruption shell");
    let signer = PrivateIdentity::new_from_name("forced termination outbound signer");
    let mut source = [0; 16];
    source.copy_from_slice(signer.address_hash().as_slice());
    let destination: [u8; 16] = hex::decode(PEER).unwrap().try_into().unwrap();
    let canonical_wire = styrened::lxmf_bridge::build_wire_message(
        source,
        destination,
        "",
        "persisted sending attempt",
        None,
        &signer,
    )
    .expect("build canonical outbound wire");
    let message_id = lxmf::inbound_decode::outbound_message_id_hex(&canonical_wire)
        .expect("canonical outbound message id");
    let now_ms = rns_core::transport::time::now_epoch_secs_i64() * 1_000;
    let message = MessageRecord {
        id: message_id.clone(),
        source: hex::encode(source),
        destination: PEER.into(),
        title: String::new(),
        content: "persisted sending attempt".into(),
        timestamp: now_ms / 1_000,
        direction: "out".into(),
        fields: None,
        receipt_status: Some("sending: direct".into()),
        read: true,
    };
    let route = OutboundRouteRecord {
        message_id: message_id.clone(),
        requested_method: "direct".into(),
        actual_method: "direct".into(),
        representation: "packet".into(),
        fallback_reason: None,
        correlation_id: "forced-termination-attempt".into(),
        retry_of: None,
        deadline_unix_ms: now_ms + 60_000,
        state: "queued".into(),
        attempt_count: 0,
    };
    let attempt = OutboundAttemptRecord {
        message_id: message_id.clone(),
        attempt_number: 1,
        started_unix_ms: now_ms,
        deadline_unix_ms: route.deadline_unix_ms,
        state: "sending".into(),
        route_observation: None,
    };
    {
        let store = node.app_context.store().lock().unwrap();
        store
            .insert_outbound_message_with_canonical_wire(
                &message,
                &route,
                None,
                &[],
                0,
                None,
                Some(&canonical_wire),
            )
            .expect("commit canonical outbound message");
        assert!(store.begin_outbound_attempt(&attempt).expect("commit sending attempt"));
    }
    publish_ready(root, &message_id);
    tokio::time::sleep(Duration::from_secs(120)).await;
    drop(node);
}

fn prepare_pre_lifecycle_schema(root: &Path) {
    std::fs::create_dir_all(root.join("data")).unwrap();
    let path = root.join("data/messages.db");
    {
        let mut store = MessagesStore::open(&path).expect("create current schema fixture");
        store.mark_clean_shutdown().expect("close schema fixture cleanly");
    }
    let connection = rusqlite::Connection::open(path).unwrap();
    connection
        .execute_batch(
            "DROP TABLE mobile_storage_lifecycle;
             DELETE FROM schema_migrations
             WHERE id IN (
                 '2026-08-29-mobile-storage-lifecycle-v17',
                 '2026-08-30-mobile-storage-session-v18'
             );",
        )
        .expect("downgrade fixture to pre-lifecycle schema");
}

fn lifecycle_is_clean(path: &Path) -> bool {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT clean_shutdown != 0 FROM mobile_storage_lifecycle WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn concurrent_handles_join_the_local_session_without_recovery() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    let path = root.path().join("data/messages.db");
    let first = MessagesStore::open(&path).unwrap();
    let second = MessagesStore::open(&path).unwrap();

    assert_eq!(first.storage_status().recovery, StorageRecoveryOutcome::NewStore);
    assert_eq!(second.storage_status().recovery, StorageRecoveryOutcome::LocalSessionJoined);
    assert!(!lifecycle_is_clean(&path));

    drop(second);
    drop(first);
}

#[test]
fn only_the_last_orderly_owner_marks_the_session_clean() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    let path = root.path().join("data/messages.db");
    let mut first = MessagesStore::open(&path).unwrap();
    let mut second = MessagesStore::open(&path).unwrap();

    first.mark_clean_shutdown().unwrap();
    assert!(!lifecycle_is_clean(&path));
    drop(first);
    assert!(!lifecycle_is_clean(&path));
    second.mark_clean_shutdown().unwrap();
    assert!(lifecycle_is_clean(&path));
    drop(second);

    let reopened = MessagesStore::open(&path).unwrap();
    assert_eq!(reopened.storage_status().recovery, StorageRecoveryOutcome::CleanShutdown);
}

#[test]
fn an_unorderly_owner_cannot_be_masked_by_an_orderly_handle() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    let path = root.path().join("data/messages.db");
    let mut orderly = MessagesStore::open(&path).unwrap();
    let unorderly = MessagesStore::open(&path).unwrap();

    orderly.mark_clean_shutdown().unwrap();
    drop(unorderly);
    orderly.mark_clean_shutdown().unwrap();
    assert!(!lifecycle_is_clean(&path));
    drop(orderly);

    let reopened = MessagesStore::open(&path).unwrap();
    assert_eq!(
        reopened.storage_status().recovery,
        StorageRecoveryOutcome::InterruptedProcessRecovered
    );
}

#[test]
fn committed_mobile_state_survives_process_termination() {
    if std::env::var(CHILD_MODE).as_deref() == Ok("committed") {
        runtime().block_on(committed_child(&child_root()));
        return;
    }

    let root = tempfile::tempdir().unwrap();
    prepare_pre_lifecycle_schema(root.path());
    let mut child = spawn_child(
        "committed_mobile_state_survives_process_termination",
        "committed",
        root.path(),
    );
    let ready = wait_for_ready(&mut child, &root.path().join("ready"));
    let mut ready = ready.lines();
    let identity_hash = ready.next().expect("committed identity marker");
    let propagation_destination = ready.next().expect("propagation selection marker");
    child.terminate();

    runtime().block_on(async {
        let node = MobileNode::boot(config(root.path())).await.expect("boot replacement node");
        let status = node.storage_status().expect("storage status");
        assert_eq!(status.schema_version, MOBILE_STORAGE_SCHEMA_VERSION);
        assert_eq!(status.open, StorageOpenOutcome::Opened);
        assert_eq!(status.recovery, StorageRecoveryOutcome::InterruptedProcessRecovered);
        assert_eq!(status.last_commit.unwrap().kind, StorageCommitKind::SessionOpened);
        assert_eq!(status.degraded, None);
        assert_eq!(
            DaemonIdentity::query_identity(node.facade.as_ref()).await.unwrap().identity_hash,
            identity_hash
        );
        assert_eq!(node.get_messages(PEER, 16).await.unwrap().len(), 1);
        assert_eq!(node.list_contacts().await.unwrap().len(), 1);
        assert_eq!(node.draft(PEER).await.unwrap().unwrap().content, "committed draft");
        let page = node.conversation_page(16, None).await.unwrap();
        assert_eq!(page.conversations.len(), 1);
        assert_eq!(page.conversations[0].message_count, 1);
        assert_eq!(page.conversations[0].unread_count, 1);
        let propagation = node.propagation_snapshot().await.unwrap();
        assert!(propagation.ready);
        assert_eq!(propagation.selected_destination.as_deref(), Some(propagation_destination));
        node.shutdown().await.unwrap();
        drop(node);

        let clean = MobileNode::boot(config(root.path())).await.expect("boot after clean shutdown");
        assert_eq!(clean.storage_status().unwrap().recovery, StorageRecoveryOutcome::CleanShutdown);
        assert_eq!(clean.get_messages(PEER, 16).await.unwrap().len(), 1);
        clean.shutdown().await.unwrap();
    });
}

#[test]
fn interrupted_work_has_explicit_recovery_outcome() {
    if std::env::var(CHILD_MODE).as_deref() == Ok("interrupted") {
        runtime().block_on(interrupted_child(&child_root()));
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let mut child =
        spawn_child("interrupted_work_has_explicit_recovery_outcome", "interrupted", root.path());
    let message_id = wait_for_ready(&mut child, &root.path().join("ready"));
    child.terminate();

    runtime().block_on(async {
        let node = MobileNode::boot(config(root.path())).await.expect("boot replacement node");
        let status = node.storage_status().expect("storage status");
        assert_eq!(status.recovery, StorageRecoveryOutcome::InterruptedProcessRecovered);
        assert_eq!(status.last_commit.unwrap().kind, StorageCommitKind::SessionOpened);
        assert_eq!(status.degraded, None);
        let message = node.message(&message_id).await.unwrap().expect("recovered outbound message");
        assert_eq!(message.id, message_id);
        assert_eq!(message.status, "queued: recovered");
        assert_eq!(message.attempts.len(), 1);
        assert_eq!(message.attempts[0].number, 1);
        assert_eq!(message.attempts[0].state, "interrupted");
        assert!(!matches!(message.status.as_str(), "sent" | "delivered" | "completed"));
        {
            let store = node.app_context.store().lock().unwrap();
            assert_eq!(
                store
                    .list_messages(16, None)
                    .unwrap()
                    .iter()
                    .filter(|item| item.id == message_id)
                    .count(),
                1
            );
            assert!(store.canonical_outbound_wire(&message_id).unwrap().is_some());
            let route = store.outbound_route(&message_id).unwrap().unwrap();
            assert_eq!(route.state, "queued");
            assert_eq!(route.attempt_count, 1);
        }
        let page = node.conversation_page(16, None).await.unwrap();
        assert_eq!(page.conversations.len(), 1);
        assert_eq!(page.conversations[0].message_count, 1);
        node.shutdown().await.unwrap();
    });
}
