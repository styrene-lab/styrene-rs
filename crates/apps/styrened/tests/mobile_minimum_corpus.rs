use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const CORPUS_ID: &str = "styrene-mobile-minimum-v1";
const REQUIRED_FIXTURES: &[&str] = &[
    "live-empty-connected",
    "tcp-reconnecting-rnode-unavailable",
    "canonical-peer-discovery",
    "direct-message-queued",
    "propagation-uploaded-not-delivered",
    "propagation-sync-complete",
    "stale-generation-rejected",
    "recoverable-session-failure",
];
const REQUIRED_ACCESSIBILITY_IDS: &[&str] = &[
    "mobile.session-state",
    "mobile.identity",
    "mobile.messages",
    "mobile.people",
    "mobile.network",
    "mobile.propagation",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    corpus: String,
    target_classes: Vec<TargetClass>,
    required_accessibility_ids: Vec<String>,
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    id: String,
    profile: Profile,
    generation: u64,
    session: Session,
    bearers: Vec<Bearer>,
    peers: Vec<Peer>,
    conversations: Vec<Conversation>,
    messages: Vec<Message>,
    propagation: Propagation,
    event: Option<GenerationEvent>,
    expected: ExpectedProjection,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    phase: SessionPhase,
    identity_hash: String,
    endpoint: Option<String>,
    failure: Option<TypedFailure>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bearer {
    kind: BearerKind,
    state: BearerState,
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Peer {
    destination_hash: String,
    aspect: String,
    display_name: Option<String>,
    observed_at: i64,
    age_secs: u64,
    source: PeerSource,
    announce_count: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Conversation {
    peer_hash: String,
    unread_count: u32,
    draft: String,
    draft_revision: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    id: String,
    peer_hash: String,
    content: String,
    requested_method: DeliveryMethod,
    actual_method: DeliveryMethod,
    persistence: PersistenceState,
    transport: TransportEvidence,
    propagation: PropagationEvidence,
    delivery: DeliveryEvidence,
    correlation_id: String,
    failure: Option<TypedFailure>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Propagation {
    selected_destination: Option<String>,
    ready: bool,
    sync_state: SyncState,
    new_messages: u32,
    failure: Option<TypedFailure>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationEvent {
    generation: u64,
    expected_applied: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedProjection {
    fixture_banner: bool,
    live_network_enabled: bool,
    peer_count: usize,
    conversation_count: usize,
    message_count: usize,
    accessibility_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TypedFailure {
    code: String,
    retryable: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum TargetClass {
    Ios,
    Android,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Profile {
    Live,
    Fixture,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum SessionPhase {
    Connected,
    Reconnecting,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
enum BearerKind {
    Tcp,
    BluetoothRnode,
    AndroidUsb,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum BearerState {
    Connected,
    Disconnected,
    Reconnecting,
    Unavailable,
    Unverified,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum DeliveryMethod {
    Direct,
    Propagated,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum PeerSource {
    CanonicalAnnounce,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum PersistenceState {
    Durable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum TransportEvidence {
    Accepted,
    None,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum PropagationEvidence {
    Uploaded,
    None,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum DeliveryEvidence {
    Pending,
    Delivered,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum SyncState {
    Idle,
    Complete,
    Failed,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn corpus_path() -> PathBuf {
    workspace_root().join("tests/fixtures/mobile-minimum-v1/states.json")
}

fn read_corpus() -> Corpus {
    let path = corpus_path();
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

#[test]
fn mobile_minimum_fixture_contract_is_strict_and_complete() {
    let corpus = read_corpus();

    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.corpus, CORPUS_ID);
    assert_eq!(corpus.target_classes, [TargetClass::Ios, TargetClass::Android]);
    assert_eq!(
        corpus.required_accessibility_ids, REQUIRED_ACCESSIBILITY_IDS,
        "accessibility identifiers are a cross-target contract"
    );
    assert_eq!(
        corpus.fixtures.iter().map(|fixture| fixture.id.as_str()).collect::<HashSet<_>>(),
        REQUIRED_FIXTURES.iter().copied().collect(),
        "fixture IDs must cover every minimum state"
    );

    for fixture in &corpus.fixtures {
        assert!(fixture.generation > 0, "{}: generation must be non-zero", fixture.id);
        assert_eq!(
            fixture.expected.peer_count,
            fixture.peers.len(),
            "{}: peer projection count",
            fixture.id
        );
        assert_eq!(
            fixture.expected.conversation_count,
            fixture.conversations.len(),
            "{}: conversation projection count",
            fixture.id
        );
        assert_eq!(
            fixture.expected.message_count,
            fixture.messages.len(),
            "{}: message projection count",
            fixture.id
        );
        assert_eq!(
            fixture.expected.accessibility_ids, REQUIRED_ACCESSIBILITY_IDS,
            "{}: accessibility contract",
            fixture.id
        );
        assert_eq!(fixture.profile == Profile::Fixture, fixture.expected.fixture_banner);
        assert_eq!(fixture.profile == Profile::Live, fixture.expected.live_network_enabled);
        assert!(!fixture.session.identity_hash.is_empty(), "{}: identity", fixture.id);
        assert!(
            fixture.session.endpoint.as_ref().is_some_and(|endpoint| !endpoint.is_empty()),
            "{}: endpoint",
            fixture.id
        );
        if fixture.session.phase == SessionPhase::Failed {
            validate_failure(fixture.session.failure.as_ref(), &fixture.id, "session");
        } else {
            assert!(
                fixture.session.failure.is_none(),
                "{}: unexpected session failure",
                fixture.id
            );
        }

        let bearer_kinds = fixture.bearers.iter().map(|bearer| bearer.kind).collect::<HashSet<_>>();
        assert_eq!(
            bearer_kinds,
            HashSet::from([BearerKind::Tcp, BearerKind::BluetoothRnode, BearerKind::AndroidUsb,]),
            "{}: all bearer states must be explicit",
            fixture.id
        );
        assert_eq!(bearer_kinds.len(), fixture.bearers.len(), "{}: duplicate bearer", fixture.id);
        for bearer in &fixture.bearers {
            if bearer.state == BearerState::Connected {
                assert!(bearer.reason.is_none(), "{}: connected bearer reason", fixture.id);
            } else {
                assert!(
                    bearer.reason.as_ref().is_some_and(|reason| !reason.is_empty()),
                    "{}: non-connected bearer reason",
                    fixture.id
                );
            }
        }

        let peer_hashes =
            fixture.peers.iter().map(|peer| peer.destination_hash.as_str()).collect::<HashSet<_>>();
        assert_eq!(peer_hashes.len(), fixture.peers.len(), "{}: duplicate peer", fixture.id);
        for peer in &fixture.peers {
            assert_eq!(peer.destination_hash.len(), 32, "{}: peer hash", fixture.id);
            assert!(!peer.aspect.is_empty(), "{}: peer aspect", fixture.id);
            assert!(peer.observed_at > 0, "{}: peer observation", fixture.id);
            assert_eq!(peer.source, PeerSource::CanonicalAnnounce);
            assert!(peer.announce_count > 0, "{}: peer announce count", fixture.id);
            let _ = (&peer.display_name, peer.age_secs);
        }
        for conversation in &fixture.conversations {
            assert!(!conversation.peer_hash.is_empty(), "{}: conversation peer", fixture.id);
            let _ = (conversation.unread_count, &conversation.draft, conversation.draft_revision);
        }
        for message in &fixture.messages {
            assert_eq!(
                message.persistence,
                PersistenceState::Durable,
                "{}: presented message must be durable",
                fixture.id
            );
            assert!(!message.id.is_empty(), "{}: message ID", fixture.id);
            assert!(!message.peer_hash.is_empty(), "{}: message peer", fixture.id);
            assert!(!message.content.is_empty(), "{}: message content", fixture.id);
            assert!(!message.correlation_id.is_empty(), "{}: correlation", fixture.id);
            assert_eq!(message.requested_method, message.actual_method);
            if message.delivery == DeliveryEvidence::Delivered {
                assert_eq!(
                    message.transport,
                    TransportEvidence::Accepted,
                    "{}: delivered transport evidence",
                    fixture.id
                );
            }
            if message.propagation == PropagationEvidence::Uploaded {
                assert_eq!(message.actual_method, DeliveryMethod::Propagated);
            }
            if message.failure.is_some() {
                validate_failure(message.failure.as_ref(), &fixture.id, "message");
            }
        }
        if fixture.propagation.ready {
            assert!(fixture.propagation.selected_destination.is_some());
        }
        if fixture.propagation.sync_state == SyncState::Failed {
            validate_failure(fixture.propagation.failure.as_ref(), &fixture.id, "propagation");
        } else {
            assert!(
                fixture.propagation.failure.is_none(),
                "{}: unexpected propagation failure",
                fixture.id
            );
        }
        if fixture.propagation.new_messages > 0 {
            assert_eq!(fixture.propagation.sync_state, SyncState::Complete);
        }
        if let Some(event) = &fixture.event {
            assert_eq!(event.expected_applied, event.generation == fixture.generation);
        }
    }
}

fn validate_failure(failure: Option<&TypedFailure>, fixture_id: &str, domain: &str) {
    let failure = failure.unwrap_or_else(|| panic!("{fixture_id}: missing {domain} failure"));
    assert!(!failure.code.is_empty(), "{fixture_id}: {domain} failure code");
    assert!(failure.retryable, "{fixture_id}: minimum corpus failures must be recoverable");
}

#[test]
fn live_empty_fixture_never_substitutes_preview_records() {
    let corpus = read_corpus();
    let fixture = corpus
        .fixtures
        .iter()
        .find(|fixture| fixture.id == "live-empty-connected")
        .expect("required live-empty fixture");

    assert_eq!(fixture.profile, Profile::Live);
    assert!(fixture.peers.is_empty());
    assert!(fixture.conversations.is_empty());
    assert!(fixture.messages.is_empty());
    assert!(!fixture.expected.fixture_banner);
    assert!(fixture.expected.live_network_enabled);
}

#[test]
fn propagation_upload_is_not_recipient_delivery() {
    let corpus = read_corpus();
    let fixture = corpus
        .fixtures
        .iter()
        .find(|fixture| fixture.id == "propagation-uploaded-not-delivered")
        .expect("required propagation upload fixture");
    let message = fixture.messages.first().expect("propagated message");

    assert_eq!(message.propagation, PropagationEvidence::Uploaded);
    assert_eq!(message.delivery, DeliveryEvidence::Pending);
    assert_eq!(message.actual_method, DeliveryMethod::Propagated);
}
