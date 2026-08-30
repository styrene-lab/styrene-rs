use styrene_ipc::types::{
    MessageAttemptInfo, MessageAttemptInterfaceObservation, MessageAttemptRouteOutcome,
};

#[test]
fn route_observation_round_trips_without_private_underlay() {
    let mut attempt = MessageAttemptInfo::default();
    attempt.message_id = "aa".repeat(32);
    attempt.number = 1;
    attempt.bearer = Some("tcp".into());
    attempt.route.outcome = MessageAttemptRouteOutcome::Observed;
    attempt.route.connection_generation = Some(4);
    attempt.route.observed_at = Some(10);
    attempt.route.next_hop = Some("bb".repeat(16));
    attempt.route.hops = Some(1);
    let mut interface = MessageAttemptInterfaceObservation::default();
    interface.id = "cc".repeat(16);
    interface.kind = "tcp_client".into();
    interface.generation = 4;
    attempt.route.interface = Some(interface);

    let json = serde_json::to_string(&attempt).unwrap();
    let decoded: MessageAttemptInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, attempt);
    for forbidden in ["endpoint", "address", "device", "path", "credential"] {
        assert!(!json.contains(forbidden));
    }
}

#[test]
fn legacy_attempt_defaults_route_to_explicit_unknown() {
    let attempt: MessageAttemptInfo = serde_json::from_value(serde_json::json!({
        "message_id": "legacy",
        "number": 1,
        "state": "sent"
    }))
    .unwrap();
    assert_eq!(attempt.route.outcome, MessageAttemptRouteOutcome::Unknown);
    assert_eq!(attempt.bearer, None);
    assert_eq!(attempt.route.interface, None);
}
