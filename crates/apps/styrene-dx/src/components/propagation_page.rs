use dioxus::prelude::*;
use styrene_ipc::types::{PropagationSnapshot, StandardPropagationSnapshot};

use crate::stores::DataState;

#[component]
pub fn PropagationPage(
    snapshot: Option<PropagationSnapshot>,
    standard_snapshot: Option<StandardPropagationSnapshot>,
    state: DataState,
    standard_state: DataState,
    available: bool,
    on_refresh: EventHandler<Option<String>>,
) -> Element {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    rsx! {
        main { class: "main propagation-page",
            div { class: "propagation-header",
                div {
                    h2 { "Propagation" }
                    p { "Local store-and-forward queue and synchronization capabilities." }
                }
                button { disabled: !available, onclick: move |_| on_refresh.call(None), "Refresh" }
            }
            if !available {
                div { class: "propagation-disabled",
                    h3 { "Unsupported backend" }
                    p { "The current daemon generation does not authorize propagation status or advertise a standard LXMF propagation runtime." }
                }
            } else {
                if let DataState::Degraded { reason } = &standard_state {
                    div { class: "propagation-disabled error",
                        h3 { "Standard propagation refresh degraded" }
                        p { "{reason}" }
                        p { "The last generation-valid snapshot remains visible below." }
                    }
                }
                if let Some(standard) = standard_snapshot {
                    if standard.observed_at.is_none() && !standard.registered && standard.policy.is_none() {
                        div { class: "propagation-disabled",
                            h3 { "Standard LXMF runtime unavailable" }
                            p { "The daemon reports no active standard propagation host or client observation for this generation." }
                        }
                    }
                    section { class: "propagation-section",
                        h3 { "Standard LXMF status" }
                        div { class: "propagation-summary",
                            Summary { label: "Runtime", value: if standard.active { "Active".to_string() } else { "Inactive".to_string() } }
                            Summary { label: "Destination", value: if standard.registered { "Registered".to_string() } else { "Not registered".to_string() } }
                            Summary { label: "Observed", value: format_optional_time(standard.observed_at) }
                            Summary { label: "Generation", value: standard.connection_generation.map(|value| value.to_string()).unwrap_or_else(|| "Not reported".into()) }
                        }
                        p { class: "propagation-selection",
                            "Selected peer: "
                            strong { "{selection_peer(&standard)}" }
                            "  mode: {selection_mode(&standard)}"
                        }
                    }
                    section { class: "propagation-section",
                        h3 { "Capacity and policy" }
                        div { class: "propagation-summary",
                            Summary { label: "Queued", value: standard.queue.queued_count.to_string() }
                            Summary { label: "Queued bytes", value: format_bytes(standard.queue.queued_bytes) }
                            Summary { label: "Acknowledged", value: standard.queue.acknowledged_count.to_string() }
                            Summary { label: "Expired", value: standard.queue.expired_count.to_string() }
                        }
                        if let Some(policy) = &standard.policy {
                            div { class: "propagation-policy-grid",
                                span { "Count limit " strong { "{policy.queue_max_count}" } }
                                span { "Byte limit " strong { "{format_bytes(policy.queue_max_bytes)}" } }
                                span { "Expiry " strong { "{format_duration(policy.expiry_secs)}" } }
                                span { "Throttle " strong { "{format_duration(policy.throttle_secs)}" } }
                                span { "Transfer limit " strong { "{policy.transfer_limit_kb} kB" } }
                                span { "Sync limit " strong { "{policy.sync_limit_kb} kB" } }
                                span { "Offer links " strong { "{policy.max_offer_links}" } }
                                span { "Stamp target/flexibility/peer " strong { "{policy.target_cost}/{policy.flexibility}/{policy.peering_cost}" } }
                            }
                        } else {
                            p { class: "propagation-empty", "Policy is not reported for this runtime." }
                        }
                    }
                    section { class: "propagation-section",
                        h3 { "Peers" }
                        if standard.peers.is_empty() {
                            p { class: "propagation-empty", "No standard propagation peers are reported." }
                        } else {
                            div { class: "propagation-records",
                                for peer in &standard.peers {
                                    article { class: "propagation-record standard",
                                        div { strong { "{short_hash(&peer.peer_hash)}" } span { "configured {peer.configured} · enabled {peer.enabled}" } }
                                        span { "last seen {peer.last_seen_at}" }
                                        span { "retry {format_optional_time(peer.retry_at)} · backoff {peer.backoff_count}" }
                                        span { "offered/wanted/accepted {peer.offered_count}/{peer.wanted_count}/{peer.accepted_count}" }
                                        span { "accepted {format_bytes(peer.accepted_bytes)} · failures {peer.failure_count}" }
                                        span { "costs {format_optional(peer.stamp_cost)}/{format_optional(peer.stamp_flexibility)}/{format_optional(peer.peering_cost)}" }
                                    }
                                }
                            }
                        }
                    }
                    section { class: "propagation-section",
                        h3 { "Synchronization and transfers" }
                        if standard.attempts.is_empty() {
                            p { class: "propagation-empty", "No standard propagation attempts are reported." }
                        } else {
                            div { class: "propagation-records",
                                for attempt in &standard.attempts {
                                    article { class: "propagation-record standard",
                                        div { strong { "{short_hash(&attempt.attempt_id)}" } code { "corr {short_hash(&attempt.correlation_id)}" } }
                                        span { "{enum_label(attempt.direction)} / {enum_label(attempt.stage)}" }
                                        span { class: "propagation-state", "{enum_label(attempt.state)} · {enum_label(attempt.outcome)}" }
                                        span { "peer {optional_hash(attempt.peer_hash.as_deref())}" }
                                        span { "offered/wanted/accepted {attempt.offered_count}/{attempt.wanted_count}/{attempt.accepted_count} · {format_bytes(attempt.accepted_bytes)}" }
                                        span { "updated {attempt.updated_at} · deadline {format_optional_time(attempt.deadline_at)}" }
                                        if let Some(code) = &attempt.failure_code { span { class: "propagation-failure-code", "failure {code}" } }
                                    }
                                }
                            }
                        }
                    }
                    section { class: "propagation-section split",
                        div {
                            h3 { "Checkpoints" }
                            if standard.checkpoints.is_empty() {
                                p { class: "propagation-empty", "No completed checkpoints are reported." }
                            }
                            for checkpoint in &standard.checkpoints {
                                article { class: "propagation-record compact",
                                    strong { "{short_hash(&checkpoint.peer_hash)}" }
                                    span { "{enum_label(checkpoint.direction)} / {enum_label(checkpoint.completed_stage)}" }
                                    span { "{checkpoint.item_count} items · {format_bytes(checkpoint.byte_count)} · updated {checkpoint.updated_at}" }
                                    span { "attempt {optional_hash(checkpoint.last_attempt_id.as_deref())}" }
                                }
                            }
                        }
                        div {
                            h3 { "Failures" }
                            if standard.failures.is_empty() {
                                p { class: "propagation-empty", "No retained failures are reported." }
                            }
                            for failure in &standard.failures {
                                article { class: "propagation-failure",
                                    strong { "{failure.code}" }
                                    span { "at {failure.occurred_at}" }
                                    span { "peer {optional_hash(failure.peer_hash.as_deref())} · attempt {optional_hash(failure.attempt_id.as_deref())}" }
                                }
                            }
                        }
                    }
                    if standard.peers_truncated || standard.attempts_truncated || standard.checkpoints_truncated || standard.failures_truncated {
                        div { class: "propagation-truncation",
                            "Daemon retention limits truncated: {truncation_summary(&standard)}."
                        }
                    }
                } else if !matches!(standard_state, DataState::Degraded { .. }) {
                    div { class: "propagation-disabled",
                        h3 { "Loading standard propagation state" }
                        p { "Waiting for the generation-scoped daemon snapshot." }
                    }
                }

                if let DataState::Degraded { reason } = &state {
                    div { class: "propagation-disabled error",
                        h3 { "Local Styrene queue unavailable" }
                        p { "{reason}" }
                    }
                } else if let Some(snapshot) = snapshot {
                    section { class: "propagation-section legacy-propagation",
                        h3 { "Styrene local queue" }
                        p { "This queue is separate from standard LXMF propagation." }
                        if !snapshot.enabled {
                            p { class: "propagation-empty", "The local Styrene store-and-forward queue is disabled." }
                        } else {
                            div { class: "propagation-summary",
                                Summary { label: "Queued", value: snapshot.queue_count.to_string() }
                                Summary { label: "Stored bytes", value: format_bytes(snapshot.queue_size_bytes) }
                                Summary { label: "Expiry policy", value: format_duration(snapshot.expiry_secs) }
                                Summary { label: "Capacity", value: snapshot.capacity_bytes.map(format_bytes).unwrap_or_else(|| "Not reported".into()) }
                            }
                            if snapshot.queue.is_empty() {
                                p { class: "propagation-empty", "The local queue is empty." }
                            } else {
                                div { class: "propagation-records",
                                    for entry in snapshot.queue {
                                        article { class: "propagation-record",
                                            div { strong { "{short_hash(&entry.destination_hash)}" } code { "{entry.id}" } }
                                            span { "{format_bytes(entry.size_bytes)} · age {format_age(now, entry.received_at)} · expires {format_age(entry.expires_at, now)}" }
                                            span { class: "propagation-state", "{entry.state}" }
                                        }
                                    }
                                }
                            }
                            if let Some(cursor) = snapshot.next_cursor.clone() {
                                button { onclick: move |_| on_refresh.call(Some(cursor.clone())), "Load more local records" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Summary(label: &'static str, value: String) -> Element {
    rsx! {
        div {
            strong { "{value}" }
            span { "{label}" }
        }
    }
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn optional_hash(hash: Option<&str>) -> String {
    hash.map(short_hash).unwrap_or_else(|| "not reported".into())
}

fn format_optional(value: Option<u32>) -> String {
    value.map_or_else(|| "?".into(), |value| value.to_string())
}

fn format_optional_time(value: Option<i64>) -> String {
    value.map_or_else(|| "Not reported".into(), |value| value.to_string())
}

fn enum_label(value: impl std::fmt::Debug) -> String {
    format!("{value:?}").to_ascii_lowercase()
}

fn selection_peer(snapshot: &StandardPropagationSnapshot) -> String {
    snapshot
        .selection
        .as_ref()
        .and_then(|selection| selection.peer_hash.as_deref())
        .map(short_hash)
        .unwrap_or_else(|| "not reported".into())
}

fn selection_mode(snapshot: &StandardPropagationSnapshot) -> &str {
    snapshot.selection.as_ref().map_or("not reported", |selection| selection.mode.as_str())
}

fn truncation_labels(snapshot: &StandardPropagationSnapshot) -> Vec<&'static str> {
    [
        (snapshot.peers_truncated, "peers"),
        (snapshot.attempts_truncated, "attempts"),
        (snapshot.checkpoints_truncated, "checkpoints"),
        (snapshot.failures_truncated, "failures"),
    ]
    .into_iter()
    .filter_map(|(truncated, label)| truncated.then_some(label))
    .collect()
}

fn truncation_summary(snapshot: &StandardPropagationSnapshot) -> String {
    truncation_labels(snapshot).join(", ")
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_duration(seconds: u64) -> String {
    if seconds > 0 && seconds.is_multiple_of(86_400) {
        format!("{} days", seconds / 86_400)
    } else {
        format!("{seconds} seconds")
    }
}

fn format_age(later: i64, earlier: i64) -> String {
    let seconds = later.saturating_sub(earlier);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_formatters_are_bounded_and_human_readable() {
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_duration(604_800), "7 days");
        assert_eq!(format_duration(0), "0 seconds");
        assert_eq!(format_age(7200, 0), "2h");
        assert_eq!(short_hash("ééééééééééééé"), "éééééééééééé");
    }

    #[test]
    fn standard_snapshot_helpers_do_not_infer_selection_and_disclose_truncation() {
        let mut snapshot = StandardPropagationSnapshot::default();
        assert_eq!(selection_peer(&snapshot), "not reported");
        assert_eq!(selection_mode(&snapshot), "not reported");
        snapshot.peers_truncated = true;
        snapshot.failures_truncated = true;
        assert_eq!(truncation_summary(&snapshot), "peers, failures");
    }
}
