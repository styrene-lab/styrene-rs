use rumqttc::v5::mqttbytes::QoS;

/// Determine the appropriate QoS level for an event type.
///
/// Policy:
/// - QoS 0 (at most once): streaming deltas where loss is tolerable
/// - QoS 1 (at least once): state transitions, tool lifecycle
/// - QoS 2 (exactly once): session/agent lifecycle events
pub fn qos_for_event(event_type: &str) -> QoS {
    match event_type {
        // Streaming — loss tolerable, high frequency
        "message.delta" | "thinking.delta" | "tool.updated" => QoS::AtMostOnce,

        // Lifecycle — exactly-once semantics required
        "session.reset" | "agent.completed" | "decomposition.started"
        | "decomposition.completed" => QoS::ExactlyOnce,

        // Everything else — at least once
        _ => QoS::AtLeastOnce,
    }
}

/// Override the default QoS policy for a specific publish.
#[derive(Debug, Clone, Copy, Default)]
pub enum QosOverride {
    /// Use the default policy from [`qos_for_event`].
    #[default]
    Policy,
    /// Force a specific QoS level.
    Force(QoS),
}

impl QosOverride {
    /// Resolve to a concrete QoS level given the event type.
    pub fn resolve(self, event_type: &str) -> QoS {
        match self {
            Self::Policy => qos_for_event(event_type),
            Self::Force(qos) => qos,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_events_use_qos0() {
        assert_eq!(qos_for_event("message.delta"), QoS::AtMostOnce);
        assert_eq!(qos_for_event("thinking.delta"), QoS::AtMostOnce);
        assert_eq!(qos_for_event("tool.updated"), QoS::AtMostOnce);
    }

    #[test]
    fn lifecycle_events_use_qos2() {
        assert_eq!(qos_for_event("session.reset"), QoS::ExactlyOnce);
        assert_eq!(qos_for_event("agent.completed"), QoS::ExactlyOnce);
        assert_eq!(qos_for_event("decomposition.started"), QoS::ExactlyOnce);
        assert_eq!(qos_for_event("decomposition.completed"), QoS::ExactlyOnce);
    }

    #[test]
    fn state_transition_events_use_qos1() {
        assert_eq!(qos_for_event("turn.started"), QoS::AtLeastOnce);
        assert_eq!(qos_for_event("turn.ended"), QoS::AtLeastOnce);
        assert_eq!(qos_for_event("tool.started"), QoS::AtLeastOnce);
        assert_eq!(qos_for_event("tool.ended"), QoS::AtLeastOnce);
        assert_eq!(qos_for_event("phase.changed"), QoS::AtLeastOnce);
    }

    #[test]
    fn override_forces_qos() {
        let o = QosOverride::Force(QoS::AtMostOnce);
        assert_eq!(o.resolve("session.reset"), QoS::AtMostOnce);
    }

    #[test]
    fn policy_delegates_to_default() {
        let o = QosOverride::Policy;
        assert_eq!(o.resolve("message.delta"), QoS::AtMostOnce);
        assert_eq!(o.resolve("turn.started"), QoS::AtLeastOnce);
    }
}
