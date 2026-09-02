use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;
use styrene_ipc::types::{
    MessageDeliveryEvidenceState, MessageInfo, MessageLifecycleState,
    MessageRetryIneligibilityReason,
};

const HANDOFF: &str =
    include_str!("../../../../tests/fixtures/mobile-product-handoff-v1/message.json");
const REVISION_PAIR: &str =
    include_str!("../../../../tests/fixtures/mobile-product-handoff-v1/revision-pair.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffFixture {
    schema_version: u32,
    corpus: String,
    authority: Authority,
    authoritative_fields: Vec<String>,
    message: MessageInfo,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Authority {
    repository: String,
    source_revision: String,
    source_type: String,
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

fn validate(value: &Value) -> Result<HandoffFixture, String> {
    let fixture: HandoffFixture = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid handoff fixture: {error}"))?;
    if fixture.schema_version != 1 || fixture.corpus != "styrene-mobile-product-handoff-v1" {
        return Err("unsupported handoff fixture identity".into());
    }
    if fixture.authority.repository != "https://github.com/styrene-lab/styrene-rs.git"
        || fixture.authority.source_revision.len() != 40
        || !fixture.authority.source_revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        || fixture.authority.source_type != "styrene_ipc::types::MessageInfo"
    {
        return Err("handoff authority must identify an immutable backend contract".into());
    }
    let unique = fixture.authoritative_fields.iter().collect::<HashSet<_>>();
    if unique.len() != fixture.authoritative_fields.len() {
        return Err("authoritative field paths must be unique".into());
    }
    let message = value.get("message").ok_or("message is required")?;
    for pointer in &fixture.authoritative_fields {
        if message.pointer(pointer).is_none() {
            return Err(format!("missing authoritative field {pointer}"));
        }
    }
    let expected = &fixture.message;
    if !expected.projection_complete
        || expected.lifecycle_state != MessageLifecycleState::Failed
        || expected.retry_eligible != Some(false)
        || expected.retry_ineligibility_reason
            != Some(MessageRetryIneligibilityReason::AttemptLimitReached)
        || expected.requested_delivery_method.as_deref() != Some("propagated")
        || expected.actual_delivery_method.as_deref() != Some("direct")
        || expected.attempts.first().and_then(|attempt| attempt.bearer.as_deref()) != Some("tcp")
        || expected.propagation_correlations.first().map(|correlation| correlation.state.as_str())
            != Some("accepted")
        || expected.delivery_evidence.first().map(|evidence| evidence.state)
            != Some(MessageDeliveryEvidenceState::Completed)
    {
        return Err("authoritative message evidence was altered or synthesized".into());
    }
    Ok(fixture)
}

#[test]
fn backend_owned_handoff_deserializes_with_immutable_authority() {
    let value: Value = serde_json::from_str(HANDOFF).expect("handoff fixture must be JSON");
    let fixture = validate(&value).expect("handoff fixture must preserve backend authority");

    assert_eq!(fixture.message.id, "message-1");
    assert_eq!(fixture.message.attempts[0].route.connection_generation, Some(7));
    assert_eq!(
        fixture.message.delivery_evidence[0].correlation_id.as_deref(),
        Some("correlation-1")
    );
}

#[test]
fn handoff_declares_the_verified_cross_repository_revision_pair() {
    let pair: RevisionPair =
        serde_json::from_str(REVISION_PAIR).expect("revision pair must be strict JSON");

    assert_eq!(pair.schema_version, 1);
    assert_eq!(pair.backend_revision, "73daf4414deb826d388a4ca2cc1bb53a4bfd32d5");
    assert_eq!(pair.ui_revision, "9750cb52e5291e6cbd887ef725d0306878cad50f");
    assert_eq!(
        pair.fixture_sha256,
        "00918cb8d369d8bc1622942bfebb92994b9ae56f5fe893047e3f111b198df014"
    );
    assert_eq!(pair.evidence_class, "component_and_reducer");
    assert_eq!(pair.verification.len(), 4);
    assert!(pair.verification.iter().all(|command| command.starts_with("cargo test ")));
}

#[test]
fn handoff_mutations_reject_dropped_or_synthesized_authority() {
    let canonical: Value = serde_json::from_str(HANDOFF).expect("handoff fixture must be JSON");

    let mut dropped = canonical.clone();
    dropped["message"].as_object_mut().unwrap().remove("actual_delivery_method");
    assert!(validate(&dropped).unwrap_err().contains("missing authoritative field"));

    let mut synthesized = canonical;
    synthesized["message"]["retry_eligible"] = Value::Bool(true);
    assert!(validate(&synthesized).unwrap_err().contains("altered or synthesized"));
}
