use dioxus::prelude::*;
use styrene_ipc::types::{ActiveCapabilitiesInfo, IdentityInfo, InterfaceDetail};

use crate::backend::{FixtureId, RuntimeProfile};
use crate::daemon_bridge::BrokerDiagnostics;
use crate::state::MeshStatusInfo;

#[component]
pub fn SystemPage(
    profile: Option<RuntimeProfile>,
    connected: bool,
    connection_mode: String,
    client_generation: u64,
    server_generation: Option<u64>,
    event_generation: Option<u64>,
    identity: Option<IdentityInfo>,
    interfaces: Vec<InterfaceDetail>,
    capabilities: Option<ActiveCapabilitiesInfo>,
    status: MeshStatusInfo,
    propagation_queue: Option<(u64, u64)>,
    diagnostics: BrokerDiagnostics,
) -> Element {
    let profile_rows = profile_rows(profile.as_ref());
    let identity_rows = identity_rows(identity.as_ref());
    let storage_rows = storage_rows(profile.as_ref(), propagation_queue);
    let config_update = capabilities
        .as_ref()
        .map(|active| authorized(active, "rpc.config_update"))
        .unwrap_or("unknown");
    let policy_update = capabilities
        .as_ref()
        .map(|active| authorized(active, "policy.update"))
        .unwrap_or("unknown");
    rsx! {
        main { class: "main system-page",
            header { class: "system-header",
                div {
                    span { class: "system-kicker", "Authoritative local runtime state" }
                    h2 { "System" }
                    p { "This IPC version has no typed local configuration mutation contract. Runtime state is read-only." }
                }
                span { class: if connected { "system-connection ready" } else { "system-connection degraded" },
                    if connected { "Connected" } else { "Disconnected" }
                }
            }
            section { class: "system-grid",
                SettingsPanel { title: "Runtime profile", rows: profile_rows, unavailable: None }
                SettingsPanel {
                    title: "Identity",
                    rows: identity_rows,
                    unavailable: identity.is_none().then(|| "Identity has not been reported for this session.".to_string()),
                }
                article { class: "system-panel wide",
                    h3 { "Runtime interfaces" }
                    p { class: "system-note", "Observed daemon interfaces. Editing and rebinding are not supported by this IPC version." }
                    if interfaces.is_empty() {
                        p { class: "system-unavailable", "No runtime interfaces reported." }
                    } else {
                        div { class: "system-records",
                            for interface in interfaces {
                                {
                                    let local = endpoint_presence(interface.local_endpoint.as_deref());
                                    let remote = endpoint_presence(interface.remote_endpoint.as_deref());
                                    let source = interface.observation.source.as_str();
                                    let generation = optional_u64(interface.observation.connection_generation);
                                    rsx! { div { class: "system-record",
                                        strong { "{interface.name}" }
                                        code { "{interface.hash}" }
                                        span { "{interface.kind} / {interface.mode} / {interface.status}" }
                                        span { "local {local} / remote {remote}" }
                                        span { "RX {interface.rx_bytes} / TX {interface.tx_bytes} / peers {interface.peers_connected}" }
                                        span { "source {source} / generation {generation} / stale {interface.observation.stale}" }
                                    } }
                                }
                            }
                        }
                    }
                }
                article { class: "system-panel wide",
                    h3 { "Policy and capabilities" }
                    p { class: "system-note", "Runtime support and caller authorization are independent." }
                    if let Some(active) = capabilities.as_ref() {
                        PolicyList { label: "Active runtime", values: active.runtime.clone() }
                        PolicyList { label: "Authorized operations", values: active.authorized_operations.clone() }
                        if active.degraded.is_empty() {
                            p { class: "system-empty", "No degraded capabilities reported." }
                        } else {
                            div { class: "system-policy-list",
                                strong { "Degraded" }
                                for item in &active.degraded { code { "{item.id}: daemon reported degraded" } }
                            }
                        }
                        p { class: "system-note", "Advertised remote config capability: {config_update} / policy capability: {policy_update}. Neither provides a local System configuration contract." }
                    } else {
                        p { class: "system-unavailable", "Generation-valid policy state is unavailable." }
                    }
                }
                SettingsPanel { title: "Storage", rows: storage_rows, unavailable: None }
                article { class: "system-panel wide",
                    h3 { "Diagnostics" }
                    div { class: "diagnostics-grid",
                        Diagnostic { value: client_generation.to_string(), label: "Client generation" }
                        Diagnostic { value: optional_u64(server_generation), label: "Request generation" }
                        Diagnostic { value: optional_u64(event_generation), label: "Event generation" }
                        Diagnostic { value: connection_mode, label: "Connection mode" }
                        Diagnostic { value: status.version, label: "Daemon version" }
                        Diagnostic { value: format!("{}s", status.uptime), label: "Uptime" }
                        Diagnostic { value: diagnostics.queue_depth.to_string(), label: "Queued" }
                        Diagnostic { value: diagnostics.in_flight.to_string(), label: "In flight" }
                        Diagnostic { value: diagnostics.completed.to_string(), label: "Completed" }
                        Diagnostic { value: format!("{} ms", diagnostics.last_latency_ms), label: "Last latency" }
                        Diagnostic { value: diagnostics.timed_out.to_string(), label: "Timeouts" }
                        Diagnostic { value: diagnostics.cancelled.to_string(), label: "Cancelled" }
                        Diagnostic { value: diagnostics.overloaded.to_string(), label: "Overloaded" }
                        Diagnostic { value: diagnostics.disconnected.to_string(), label: "Disconnects" }
                        Diagnostic { value: diagnostics.reconnects.to_string(), label: "Reconnects" }
                        Diagnostic { value: diagnostics.stale_responses.to_string(), label: "Stale responses" }
                        Diagnostic { value: diagnostics.dropped_responses.to_string(), label: "Dropped responses" }
                        Diagnostic { value: diagnostics.dropped_updates.to_string(), label: "Dropped updates" }
                    }
                    p { class: "system-note", "Raw payloads, logs, keys, and generic configuration maps are never shown here." }
                }
            }
        }
    }
}

fn profile_rows(profile: Option<&RuntimeProfile>) -> Vec<(String, String)> {
    match profile {
        Some(RuntimeProfile::Live { socket_path }) => vec![
            ("Profile".into(), "Live".into()),
            (
                "Socket".into(),
                if socket_path.as_os_str().is_empty() {
                    "Not configured".into()
                } else {
                    "Configured local Unix socket".into()
                },
            ),
            ("Lifecycle".into(), "Externally managed daemon".into()),
            ("Persistence".into(), "Not reported by IPC".into()),
        ],
        Some(RuntimeProfile::Embedded { ephemeral }) => vec![
            ("Profile".into(), "Embedded".into()),
            ("Ephemeral".into(), ephemeral.to_string()),
            ("Lifecycle".into(), "Owned by this desktop session".into()),
            ("Persistence".into(), "Temporary database and identity".into()),
            ("Cleanup".into(), "Owned root removed on shutdown".into()),
        ],
        Some(RuntimeProfile::Fixture { fixture }) => vec![
            ("Profile".into(), "Fixture".into()),
            ("Fixture".into(), fixture_name(*fixture).into()),
            ("Lifecycle".into(), "No daemon process".into()),
            ("Network".into(), "External networking disabled".into()),
            ("Persistence".into(), "Deterministic in-memory state".into()),
        ],
        None => vec![("Profile".into(), "Invalid or unavailable".into())],
    }
}

fn identity_rows(identity: Option<&IdentityInfo>) -> Vec<(String, String)> {
    identity
        .map(|identity| {
            vec![
                ("Identity hash".into(), identity.identity_hash.clone()),
                ("Delivery destination".into(), identity.destination_hash.clone()),
                ("LXMF destination".into(), identity.lxmf_destination_hash.clone()),
                ("Display name".into(), empty_as_unreported(&identity.display_name)),
                ("Short name".into(), optional(identity.short_name.as_deref())),
                ("Icon".into(), optional(identity.icon.as_deref())),
            ]
        })
        .unwrap_or_default()
}

fn storage_rows(
    profile: Option<&RuntimeProfile>,
    propagation_queue: Option<(u64, u64)>,
) -> Vec<(String, String)> {
    let persistence = match profile {
        Some(RuntimeProfile::Embedded { .. }) => "Temporary; removed on owned shutdown",
        Some(RuntimeProfile::Fixture { .. }) => "No persistent storage",
        Some(RuntimeProfile::Live { .. }) => "General location and retention not reported by IPC",
        None => "Unavailable",
    };
    let (count, bytes) = propagation_queue
        .map(|(count, bytes)| (count.to_string(), bytes.to_string()))
        .unwrap_or_else(|| ("Not reported".into(), "Not reported".into()));
    vec![
        ("Persistence".into(), persistence.into()),
        ("Propagation queue".into(), count),
        ("Propagation bytes".into(), bytes),
        ("Destructive controls".into(), "Unsupported by this IPC version".into()),
    ]
}

fn authorized(active: &ActiveCapabilitiesInfo, capability: &str) -> &'static str {
    if active.authorized_operations.iter().any(|item| item == capability) {
        "authorized"
    } else {
        "denied"
    }
}

const fn fixture_name(fixture: FixtureId) -> &'static str {
    match fixture {
        FixtureId::Empty => "empty",
        FixtureId::Healthy => "healthy",
        FixtureId::Degraded => "degraded",
        FixtureId::HighCardinality => "high-cardinality",
        FixtureId::ActiveScenario => "active-scenario",
        FixtureId::Error => "error",
    }
}

fn empty_as_unreported(value: &str) -> String {
    if value.is_empty() {
        "Not reported".into()
    } else {
        value.into()
    }
}

fn optional(value: Option<&str>) -> String {
    value.filter(|value| !value.is_empty()).unwrap_or("Not reported").into()
}

fn endpoint_presence(value: Option<&str>) -> &'static str {
    if value.is_some_and(|value| !value.is_empty()) {
        "reported"
    } else {
        "Not reported"
    }
}

fn optional_u64(value: Option<u64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_else(|| "Not reported".into())
}

#[component]
fn SettingsPanel(
    title: &'static str,
    rows: Vec<(String, String)>,
    unavailable: Option<String>,
) -> Element {
    rsx! {
        article { class: "system-panel",
            h3 { "{title}" }
            if let Some(reason) = unavailable { p { class: "system-unavailable", "{reason}" } }
            dl {
                for (label, value) in rows { div { dt { "{label}" } dd { "{value}" } } }
            }
        }
    }
}

#[component]
fn PolicyList(label: &'static str, values: Vec<String>) -> Element {
    rsx! {
        div { class: "system-policy-list",
            strong { "{label}" }
            if values.is_empty() { span { "None reported" } }
            for value in values { code { "{value}" } }
        }
    }
}

#[component]
fn Diagnostic(value: String, label: &'static str) -> Element {
    rsx! { div { strong { "{value}" } span { "{label}" } } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn profiles_explain_lifecycle_network_and_storage_without_guessing_live_paths() {
        let live = profile_rows(Some(&RuntimeProfile::Live {
            socket_path: PathBuf::from("/run/styrene.sock"),
        }));
        assert!(live.iter().any(|(label, value)| {
            label == "Socket" && value == "Configured local Unix socket"
        }));
        assert!(!format!("{live:?}").contains("/run/styrene.sock"));
        assert!(live.iter().any(|(_, value)| value == "Not reported by IPC"));

        let fixture = profile_rows(Some(&RuntimeProfile::Fixture { fixture: FixtureId::Healthy }));
        assert!(fixture.iter().any(|(_, value)| value == "External networking disabled"));
        assert!(fixture.iter().any(|(_, value)| value == "No daemon process"));
    }

    #[test]
    fn identity_projection_contains_only_public_ipc_fields() {
        let mut identity = IdentityInfo::default();
        identity.identity_hash = "identity".into();
        identity.destination_hash = "delivery".into();
        identity.lxmf_destination_hash = "lxmf".into();
        let rows = identity_rows(Some(&identity));
        assert_eq!(rows.len(), 6);
        let rendered = format!("{rows:?}");
        assert!(rendered.contains("identity"));
        assert!(!rendered.contains("private"));
        assert!(!rendered.contains("signing key"));
    }

    #[test]
    fn policy_does_not_treat_remote_exec_as_local_administration() {
        let mut active = ActiveCapabilitiesInfo::default();
        active.authorized_operations = vec!["rpc.exec".into()];
        assert_eq!(authorized(&active, "rpc.config_update"), "denied");
        assert_eq!(authorized(&active, "policy.update"), "denied");
    }

    #[test]
    fn diagnostics_report_endpoint_presence_without_retaining_endpoint_values() {
        assert_eq!(endpoint_presence(Some("tcp://user:secret@example.test:1234")), "reported");
        assert_eq!(endpoint_presence(None), "Not reported");
    }
}
