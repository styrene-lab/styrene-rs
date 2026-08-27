//! Deterministic operator-fixture coverage catalog.
//!
//! These cases are internal UX evidence. They do not establish protocol interoperability.

use crate::types::ObservationSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperatorFixtureFamily {
    Network,
    Messaging,
    Propagation,
    Content,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperatorFixtureState {
    Disconnected,
    Stale,
    Denied,
    Unsupported,
    TimedOut,
    Cancelled,
    PartialFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperatorFixtureOperation {
    pub id: &'static str,
    pub family: OperatorFixtureFamily,
    pub capability: &'static str,
    pub timeout: bool,
    pub cancellation: bool,
    pub partial_failure: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperatorFixtureEvidence {
    pub source: ObservationSource,
    pub observed_at: i64,
    pub connection_generation: u64,
    pub correlation_id: &'static str,
    pub terminal_outcome: Option<&'static str>,
    pub retryable: bool,
    pub preserves_prior_state: bool,
    pub completed_stages: u8,
}

impl OperatorFixtureOperation {
    pub const fn applies(self, state: OperatorFixtureState) -> bool {
        match state {
            OperatorFixtureState::Disconnected
            | OperatorFixtureState::Stale
            | OperatorFixtureState::Denied
            | OperatorFixtureState::Unsupported => true,
            OperatorFixtureState::TimedOut => self.timeout,
            OperatorFixtureState::Cancelled => self.cancellation,
            OperatorFixtureState::PartialFailure => self.partial_failure,
        }
    }

    pub const fn not_applicable_reason(self, state: OperatorFixtureState) -> Option<&'static str> {
        if self.applies(state) {
            return None;
        }
        match state {
            OperatorFixtureState::TimedOut => {
                Some("operation has no awaited terminal result that can time out")
            }
            OperatorFixtureState::Cancelled => {
                Some("operation is atomic or observational and has no cancellable lifecycle")
            }
            OperatorFixtureState::PartialFailure => {
                Some("operation has no multi-stage success to preserve after a later failure")
            }
            OperatorFixtureState::Disconnected
            | OperatorFixtureState::Stale
            | OperatorFixtureState::Denied
            | OperatorFixtureState::Unsupported => None,
        }
    }
}

pub const fn operator_fixture_evidence(
    operation: OperatorFixtureOperation,
    state: OperatorFixtureState,
) -> Option<OperatorFixtureEvidence> {
    if !operation.applies(state) {
        return None;
    }
    let (terminal_outcome, retryable, preserves_prior_state, completed_stages) = match state {
        OperatorFixtureState::Disconnected => (None, true, true, 0),
        OperatorFixtureState::Stale => (None, true, true, 0),
        OperatorFixtureState::Denied => (Some("denied"), false, true, 0),
        OperatorFixtureState::Unsupported => (Some("unsupported"), false, true, 0),
        OperatorFixtureState::TimedOut => (Some("timed_out"), true, false, 0),
        OperatorFixtureState::Cancelled => (Some("cancelled"), false, false, 0),
        OperatorFixtureState::PartialFailure => (Some("failed"), true, true, 1),
    };
    Some(OperatorFixtureEvidence {
        source: ObservationSource::Fixture,
        observed_at: 1_700_000_000,
        connection_generation: 7,
        correlation_id: "fixture:operator:deterministic",
        terminal_outcome,
        retryable,
        preserves_prior_state,
        completed_stages,
    })
}

macro_rules! operation {
    ($id:literal, $family:ident, $capability:literal, $timeout:literal, $cancel:literal, $partial:literal) => {
        OperatorFixtureOperation {
            id: $id,
            family: OperatorFixtureFamily::$family,
            capability: $capability,
            timeout: $timeout,
            cancellation: $cancel,
            partial_failure: $partial,
        }
    };
}

pub const OPERATOR_FIXTURE_OPERATIONS: &[OperatorFixtureOperation] = &[
    operation!("network.announce", Network, "network.announce", false, false, false),
    operation!("network.path_request", Network, "network.path_request", true, true, false),
    operation!("network.probe", Network, "network.probe", true, true, false),
    operation!("network.link_open", Network, "network.link_open", true, true, true),
    operation!("network.link_close", Network, "network.link_close", true, true, false),
    operation!("network.operation_cancel", Network, "network.probe", false, true, false),
    operation!("network.request_start", Network, "network.request", true, true, true),
    operation!("network.request_cancel", Network, "network.request_cancel", false, true, false),
    operation!("network.resource_cancel", Network, "network.resource_cancel", false, true, false),
    operation!("message.send_direct", Messaging, "chat.send", true, true, true),
    operation!("message.send_opportunistic", Messaging, "chat.send", true, true, true),
    operation!("message.send_propagated", Messaging, "chat.send", true, true, true),
    operation!("message.export_paper", Messaging, "chat.send", false, false, true),
    operation!("message.draft", Messaging, "messaging.manage", false, false, false),
    operation!("message.retry", Messaging, "messaging.lifecycle", true, true, true),
    operation!("message.cancel", Messaging, "messaging.lifecycle", false, true, true),
    operation!("message.history", Messaging, "messaging.history.read", true, false, true),
    operation!("propagation.local_refresh", Propagation, "rpc.status", true, false, true),
    operation!("propagation.standard_refresh", Propagation, "rpc.status", true, false, true),
    operation!("propagation.attempt", Propagation, "rpc.status", true, true, true),
    operation!("content.host_inventory", Content, "page.browse", true, false, true),
    operation!("content.local_inventory", Content, "page.browse", true, false, true),
    operation!("content.browse", Content, "page.browse", true, true, true),
    operation!("content.navigate", Content, "page.browse", true, true, true),
    operation!("content.form_submit", Content, "page.browse", true, true, true),
    operation!("content.close", Content, "page.browse", true, true, false),
    operation!("content.file_start", Content, "page.browse", true, true, true),
    operation!("content.file_cancel", Content, "page.browse", false, true, false),
    operation!("content.file_save", Content, "page.browse", true, false, true),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_is_unique_and_covers_every_family_and_failure_class() {
        let ids = OPERATOR_FIXTURE_OPERATIONS
            .iter()
            .map(|operation| operation.id)
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), OPERATOR_FIXTURE_OPERATIONS.len());
        for family in [
            OperatorFixtureFamily::Network,
            OperatorFixtureFamily::Messaging,
            OperatorFixtureFamily::Propagation,
            OperatorFixtureFamily::Content,
        ] {
            assert!(OPERATOR_FIXTURE_OPERATIONS.iter().any(|operation| operation.family == family));
        }
        for state in [
            OperatorFixtureState::Disconnected,
            OperatorFixtureState::Stale,
            OperatorFixtureState::Denied,
            OperatorFixtureState::Unsupported,
            OperatorFixtureState::TimedOut,
            OperatorFixtureState::Cancelled,
            OperatorFixtureState::PartialFailure,
        ] {
            assert!(OPERATOR_FIXTURE_OPERATIONS.iter().any(|operation| operation.applies(state)));
        }
    }

    #[test]
    fn every_matrix_cell_is_applicable_or_has_an_explicit_reason() {
        for operation in OPERATOR_FIXTURE_OPERATIONS {
            for state in [
                OperatorFixtureState::Disconnected,
                OperatorFixtureState::Stale,
                OperatorFixtureState::Denied,
                OperatorFixtureState::Unsupported,
                OperatorFixtureState::TimedOut,
                OperatorFixtureState::Cancelled,
                OperatorFixtureState::PartialFailure,
            ] {
                assert_ne!(
                    operation.applies(state),
                    operation.not_applicable_reason(state).is_some(),
                    "{} {state:?}",
                    operation.id
                );
                if let Some(evidence) = operator_fixture_evidence(*operation, state) {
                    assert_eq!(evidence.source, ObservationSource::Fixture);
                    assert_eq!(evidence.connection_generation, 7);
                    assert!(evidence.correlation_id.starts_with("fixture:"));
                    if state == OperatorFixtureState::PartialFailure {
                        assert!(evidence.completed_stages > 0);
                        assert!(evidence.preserves_prior_state);
                    }
                }
            }
        }
    }
}
