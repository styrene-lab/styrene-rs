use dioxus::prelude::*;

use crate::state::ChatMessage;

#[component]
pub fn MessageBubble(message: ChatMessage) -> Element {
    let status_mark = match message.lifecycle.state {
        styrene_ipc::types::MessageLifecycleState::Delivered => "✓✓",
        styrene_ipc::types::MessageLifecycleState::Failed
        | styrene_ipc::types::MessageLifecycleState::Expired
        | styrene_ipc::types::MessageLifecycleState::Rejected => "✗",
        styrene_ipc::types::MessageLifecycleState::Queued
        | styrene_ipc::types::MessageLifecycleState::Sending
        | styrene_ipc::types::MessageLifecycleState::Sent => "✓",
        _ => "?",
    };
    rsx! {
        div { class: if message.is_outgoing { "message sent" } else { "message received" },
            div { class: "message-content", "{message.content}" }
            div { class: "message-meta",
                span { {format_timestamp(message.timestamp)} }
                if message.is_outgoing {
                    span { class: "message-status", "{status_mark}" }
                }
            }
            details { class: "message-lifecycle",
                summary { "Delivery details" }
                div { class: "message-lifecycle-grid",
                    Field { label: "Status", value: message.status.clone() }
                    Field { label: "Lifecycle", value: format!("{:?}", message.lifecycle.state) }
                    Field {
                        label: "Terminal detail",
                        value: message.lifecycle.terminal_detail.clone().unwrap_or_else(|| "Not reported".into()),
                    }
                    Field { label: "Authenticity", value: format!("{:?}", message.lifecycle.authentication) }
                    Field { label: "Stamp state", value: format!("{:?}", message.lifecycle.stamp_state) }
                    Field {
                        label: "Stamp cost",
                        value: message.lifecycle.stamp_cost.map(|value| value.to_string()).unwrap_or_else(|| "Not reported".into()),
                    }
                    Field {
                        label: "Stamp value",
                        value: message.lifecycle.stamp_value.map(|value| value.to_string()).unwrap_or_else(|| "Not reported".into()),
                    }
                    Field {
                        label: "Requested method",
                        value: message.lifecycle.requested_method.clone().unwrap_or_else(|| "Not reported".into()),
                    }
                    Field {
                        label: "Actual method",
                        value: message.lifecycle.actual_method.clone().unwrap_or_else(|| "Not reported".into()),
                    }
                    if message.lifecycle.evidence.is_empty() {
                        Field { label: "Delivery evidence", value: String::from("Not reported") }
                    }
                    for evidence in &message.lifecycle.evidence {
                        Field {
                            label: "Delivery evidence",
                            value: format!(
                                "kind={:?} hash={} representation={} state={:?} outcome={} attempt={} correlation={} observed={} terminal={}",
                                evidence.kind,
                                evidence.hash,
                                evidence.representation,
                                evidence.state,
                                evidence.outcome.as_deref().unwrap_or("Not reported"),
                                evidence.attempt.map(|value| value.to_string()).unwrap_or_else(|| "Not reported".into()),
                                evidence.correlation_id.as_deref().unwrap_or("Not reported"),
                                evidence.observed_at,
                                evidence.terminal_at.map(|value| value.to_string()).unwrap_or_else(|| "Not reported".into()),
                            ),
                        }
                    }
                    if message.lifecycle.attachments.is_empty() {
                        Field { label: "Attachments", value: String::from("Not reported") }
                    }
                    for attachment in &message.lifecycle.attachments {
                        Field {
                            label: "Attachment",
                            value: format!("{} size={} checksum={} integrity={} availability={}", attachment.name, attachment.size, attachment.checksum, attachment.integrity, attachment.availability),
                        }
                        if let Some(transfer) = attachment.transfer.as_deref() {
                            Field {
                                label: "Transfer",
                                value: format!("resource={} progress={}/{} checksum_verified={} cancellable={} state={} error={}", transfer.resource_hash.as_deref().unwrap_or("Not reported"), transfer.transferred, transfer.total, transfer.checksum_verified, transfer.cancellable, transfer.state, transfer.error.as_deref().unwrap_or("Not reported")),
                            }
                        }
                    }
                    if message.lifecycle.propagation.is_empty() {
                        Field { label: "Propagation", value: String::from("Not reported") }
                    }
                    for correlation in &message.lifecycle.propagation {
                        Field {
                            label: "Propagation",
                            value: format!("relation={} transient={} attempt={} peer={} state={} created={} updated={}", correlation.relation, correlation.transient_id, correlation.attempt_id.as_deref().unwrap_or("Not reported"), correlation.peer_hash.as_deref().unwrap_or("Not reported"), correlation.state, correlation.created_at, correlation.updated_at),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Field(label: &'static str, value: String) -> Element {
    rsx! {
        div {
            span { "{label}" }
            code { "{value}" }
        }
    }
}

fn format_timestamp(timestamp: i64) -> String {
    if timestamp == 0 {
        return String::new();
    }
    let seconds = timestamp % 86_400;
    format!("{:02}:{:02}", seconds / 3600, (seconds % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::MessageLifecycle;

    #[test]
    fn lifecycle_view_keeps_missing_daemon_fields_explicit() {
        let message = ChatMessage {
            id: "correlation".into(),
            source: String::new(),
            destination: "peer".into(),
            content: "message".into(),
            timestamp: 0,
            is_outgoing: true,
            status: "pending".into(),
            lifecycle: MessageLifecycle::default(),
        };
        assert!(message.lifecycle.requested_method.is_none());
        assert!(message.lifecycle.actual_method.is_none());
        assert!(message.lifecycle.fallback_reason.is_none());
        assert!(message.lifecycle.correlation_id.is_none());
        assert!(message.lifecycle.attempts.is_empty());
        assert!(message.lifecycle.propagation.is_empty());
    }
}
