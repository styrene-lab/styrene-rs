use styrene_a2a::{
    AcceptanceDisposition, AcceptanceReceipt, AgentEnvelope, AgentEnvelopeKind, AgentId,
    ProtocolError, ProtocolErrorCode, RootOperationId, RuntimeId, SignatureAlgorithm,
};

fn fixture_envelope() -> AgentEnvelope {
    let mut envelope = AgentEnvelope::new(
        AgentEnvelopeKind::Command,
        &AgentId::new("styrene:agent:source").unwrap(),
        RuntimeId::from_bytes([0x11; 16]),
        &AgentId::new("styrene:agent:target").unwrap(),
        &RootOperationId::new("root-1").unwrap(),
        Some("task-1".to_owned()),
        "task-1",
        1,
        1_700_000_000_000,
        "a2a.message/1.0",
        br#"{"kind":"message"}"#.to_vec(),
    );
    envelope.message_id = [0x22; 16];
    envelope.payload_digest = envelope.computed_payload_digest();
    envelope.signature_algorithm = SignatureAlgorithm::Ed25519;
    envelope.signing_key_id = "identity:source#signing-1".to_owned();
    envelope
}

#[test]
fn canonical_signing_input_is_deterministic_and_excludes_signature() {
    let envelope = fixture_envelope();
    let canonical = envelope.canonical_signing_input().unwrap();
    assert_eq!(canonical, envelope.canonical_signing_input().unwrap());

    let mut signed = envelope.clone();
    signed.signature = Some(vec![0x55; 64]);
    assert_eq!(canonical, signed.canonical_signing_input().unwrap());
}

#[test]
fn canonical_signing_input_changes_when_payload_or_protected_index_changes() {
    let envelope = fixture_envelope();
    let canonical = envelope.canonical_signing_input().unwrap();

    let mut changed = envelope.clone();
    changed.target_agent_id = "styrene:agent:other".to_owned();
    assert_ne!(canonical, changed.canonical_signing_input().unwrap());

    let mut changed = envelope;
    changed.a2a_payload = br#"{"kind":"other"}"#.to_vec();
    assert!(changed.canonical_signing_input().is_err());
}

#[test]
fn receipt_and_protocol_error_round_trip_as_typed_json() {
    let receipt = AcceptanceReceipt {
        message_id: [0x22; 16],
        runtime_id: RuntimeId::from_bytes([0x33; 16]),
        disposition: AcceptanceDisposition::Accepted,
        accepted_at_ms: 1_700_000_000_100,
    };
    let receipt_json = serde_json::to_vec(&receipt).unwrap();
    assert_eq!(serde_json::from_slice::<AcceptanceReceipt>(&receipt_json).unwrap(), receipt);

    let error = ProtocolError {
        code: ProtocolErrorCode::InvalidEnvelope,
        message: "payload digest mismatch".to_owned(),
        retryable: false,
        message_id: Some([0x22; 16]),
    };
    let error_json = serde_json::to_vec(&error).unwrap();
    assert_eq!(serde_json::from_slice::<ProtocolError>(&error_json).unwrap(), error);
}

#[test]
fn canonical_signing_input_matches_golden_vector() {
    let actual = fixture_envelope().canonical_signing_input().unwrap();
    let expected = include_bytes!("fixtures/envelope-signing-input-v1.cbor");
    assert_eq!(actual.as_slice(), expected);
}
