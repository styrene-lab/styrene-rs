//! Cross-repository destination convergence corpus.
//!
//! The frontend implements discovered, manual, pasted, and scanned destination
//! ingress. This corpus proves the backend half of task 3.6: every path must
//! reach one conversation operation, that operation must create exactly one
//! durable conversation shell, and invalid candidates must create no state
//! even when the frontend forwards them for backend validation.

use std::collections::HashSet;

use serde::Deserialize;
use styrene_ipc::traits::DaemonMessaging;
use styrene_ipc::types::MessagingDisposition;
use styrened::mobile::{IdentityBackend, MobileConfig, MobileNode};

const CORPUS: &str =
    include_str!("../../../../tests/fixtures/mobile-destination-convergence-v1/corpus.json");
const REVISION_PAIR: &str =
    include_str!("../../../../tests/fixtures/mobile-destination-convergence-v1/revision-pair.json");
const INGRESS_PATHS: [&str; 4] = ["discovered", "manual", "pasted", "scanned"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    #[serde(rename = "corpus")]
    identity: String,
    description: String,
    authority: Authority,
    canonical_peer_hash: String,
    ingress_paths: Vec<String>,
    ui_normalization: std::collections::BTreeMap<String, String>,
    converging_candidates: Vec<ConvergingCandidate>,
    rejected_candidates: Vec<RejectedCandidate>,
    invariants: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Authority {
    repository: String,
    operation: String,
    validation: String,
    openspec_change: String,
    tasks: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConvergingCandidate {
    id: String,
    ingress: String,
    raw: String,
    submitted: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RejectedCandidate {
    id: String,
    ingress: String,
    raw: String,
    ui_dispatch: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevisionPair {
    schema_version: u32,
    backend_revision: String,
    ui_revision: String,
    fixture_sha256: String,
    evidence_class: String,
    verification: Vec<String>,
}

fn corpus() -> Corpus {
    let corpus: Corpus =
        serde_json::from_str(CORPUS).expect("convergence corpus must be strict JSON");
    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.identity, "styrene-mobile-destination-convergence-v1");
    corpus
}

fn config(root: &std::path::Path) -> MobileConfig {
    MobileConfig {
        config_dir: root.join("config"),
        data_dir: root.join("data"),
        hub_address: None,
        hub_delivery_hash: None,
        display_name: None,
        identity_backend: IdentityBackend::PlaintextFile,
        interfaces: Vec::new(),
        enable_rnode_channel: false,
    }
}

fn is_canonical_peer_hash(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[test]
fn corpus_covers_every_ingress_path_with_one_canonical_target() {
    let corpus = corpus();

    assert!(!corpus.description.is_empty());
    assert_eq!(corpus.authority.repository, "https://github.com/styrene-lab/styrene-rs.git");
    assert_eq!(
        corpus.authority.operation,
        "styrene_ipc::traits::DaemonMessaging::start_conversation"
    );
    assert!(corpus.authority.validation.contains("32 hexadecimal"));
    assert_eq!(
        corpus.authority.openspec_change,
        "openspec/changes/complete-mobile-product-workflows"
    );
    assert_eq!(corpus.authority.tasks, ["3.6", "9.7"]);
    assert!(is_canonical_peer_hash(&corpus.canonical_peer_hash));
    assert_eq!(corpus.ingress_paths, INGRESS_PATHS);
    assert_eq!(corpus.invariants.len(), 5);
    for path in INGRESS_PATHS {
        assert!(corpus.ui_normalization.contains_key(path), "missing normalization for {path}");
    }

    let converging_paths: HashSet<&str> =
        corpus.converging_candidates.iter().map(|candidate| candidate.ingress.as_str()).collect();
    assert_eq!(converging_paths, INGRESS_PATHS.into_iter().collect::<HashSet<_>>());
    let rejected_paths: HashSet<&str> =
        corpus.rejected_candidates.iter().map(|candidate| candidate.ingress.as_str()).collect();
    assert!(rejected_paths.contains("manual"));
    assert!(rejected_paths.contains("pasted"));
    assert!(rejected_paths.contains("scanned"));

    let mut ids = HashSet::new();
    for candidate in &corpus.converging_candidates {
        assert!(ids.insert(candidate.id.as_str()), "duplicate candidate {}", candidate.id);
        assert_eq!(candidate.raw.trim(), candidate.submitted, "{}", candidate.id);
        assert_eq!(
            candidate.submitted.to_ascii_lowercase(),
            corpus.canonical_peer_hash,
            "{} must converge on the canonical destination",
            candidate.id
        );
    }
    for candidate in &corpus.rejected_candidates {
        assert!(ids.insert(candidate.id.as_str()), "duplicate candidate {}", candidate.id);
        assert!(
            matches!(
                candidate.ui_dispatch.as_str(),
                "forwarded"
                    | "blocked_empty"
                    | "blocked_incomplete"
                    | "blocked_oversized"
                    | "trimmed_before_dispatch"
            ),
            "{} has an unknown frontend dispatch {}",
            candidate.id,
            candidate.ui_dispatch
        );
        if candidate.ui_dispatch == "trimmed_before_dispatch" {
            assert_ne!(candidate.raw, candidate.raw.trim(), "{}", candidate.id);
            assert_eq!(
                candidate.raw.trim().to_ascii_lowercase(),
                corpus.canonical_peer_hash,
                "{} must prove the backend rejects the untrimmed form",
                candidate.id
            );
        } else {
            assert_ne!(
                candidate.raw.trim().to_ascii_lowercase(),
                corpus.canonical_peer_hash,
                "{} would converge after frontend normalization",
                candidate.id
            );
        }
    }
    assert!(
        corpus.rejected_candidates.iter().any(|candidate| candidate.ui_dispatch == "forwarded"),
        "the corpus must exercise backend validation of frontend-forwarded candidates"
    );
}

#[tokio::test]
async fn every_ingress_path_converges_on_one_durable_conversation() {
    let corpus = corpus();
    let root = tempfile::tempdir().unwrap();
    let mobile_config = config(root.path());
    let node = MobileNode::boot(mobile_config.clone()).await.unwrap();

    assert!(node.conversation_page(16, None).await.unwrap().conversations.is_empty());
    assert!(node.list_contacts().await.unwrap().is_empty());

    for (index, candidate) in corpus.converging_candidates.iter().enumerate() {
        let outcome =
            DaemonMessaging::start_conversation(node.facade.as_ref(), &candidate.submitted)
                .await
                .unwrap_or_else(|error| panic!("{} was rejected: {error}", candidate.id));
        let expected = if index == 0 {
            MessagingDisposition::Created
        } else {
            MessagingDisposition::Unchanged
        };
        assert_eq!(outcome.disposition, expected, "{}", candidate.id);
        assert_eq!(outcome.affected_count, u64::from(index == 0), "{}", candidate.id);
        assert_eq!(outcome.target_id, corpus.canonical_peer_hash, "{}", candidate.id);
        let conversation = outcome.conversation.expect("conversation shell is returned");
        assert_eq!(conversation.peer_hash, corpus.canonical_peer_hash, "{}", candidate.id);
        assert_eq!(conversation.message_count, 0, "{}", candidate.id);
        assert_eq!(conversation.last_message_content, None, "{}", candidate.id);
        assert!(outcome.contact.is_none(), "{} synthesized a contact", candidate.id);
        assert!(outcome.message.is_none(), "{} synthesized a message", candidate.id);

        let page = node.conversation_page(16, None).await.unwrap();
        assert_eq!(page.conversations.len(), 1, "{} created a second shell", candidate.id);
        assert_eq!(page.conversations[0].peer_hash, corpus.canonical_peer_hash);
    }
    assert!(node.list_contacts().await.unwrap().is_empty());

    for candidate in &corpus.rejected_candidates {
        let error = DaemonMessaging::start_conversation(node.facade.as_ref(), &candidate.raw)
            .await
            .err()
            .unwrap_or_else(|| panic!("{} was accepted by the backend", candidate.id));
        let rendered = error.to_string();
        assert!(
            !rendered.contains(candidate.raw.trim()) || candidate.raw.trim().is_empty(),
            "{} echoed the rejected candidate into the error",
            candidate.id
        );
        let page = node.conversation_page(16, None).await.unwrap();
        assert_eq!(page.conversations.len(), 1, "{} created a conversation shell", candidate.id);
        assert_eq!(page.conversations[0].peer_hash, corpus.canonical_peer_hash);
        assert!(
            node.list_contacts().await.unwrap().is_empty(),
            "{} created a contact",
            candidate.id
        );
    }
    node.shutdown().await.unwrap();

    let reopened = MobileNode::boot(mobile_config).await.unwrap();
    let page = reopened.conversation_page(16, None).await.unwrap();
    assert_eq!(page.conversations.len(), 1);
    assert_eq!(page.conversations[0].peer_hash, corpus.canonical_peer_hash);
    assert_eq!(page.conversations[0].message_count, 0);
    assert!(reopened.list_contacts().await.unwrap().is_empty());
    reopened.shutdown().await.unwrap();
}

#[test]
fn convergence_declares_the_verified_cross_repository_revision_pair() {
    let pair: RevisionPair =
        serde_json::from_str(REVISION_PAIR).expect("revision pair must be strict JSON");

    assert_eq!(pair.schema_version, 1);
    assert_eq!(pair.backend_revision.len(), 40);
    assert!(pair.backend_revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(pair.ui_revision.len(), 40);
    assert!(pair.ui_revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(pair.evidence_class, "backend_and_session_component");
    assert!(pair.verification.len() >= 3);
    assert!(pair.verification.iter().all(|command| command.starts_with("cargo test ")));

    let digest = {
        use sha2::Digest as _;
        hex::encode(sha2::Sha256::digest(CORPUS.as_bytes()))
    };
    assert_eq!(pair.fixture_sha256, digest, "revision pair must pin the committed corpus bytes");
}
