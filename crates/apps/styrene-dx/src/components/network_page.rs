use std::collections::HashSet;

use dioxus::prelude::*;

use crate::daemon_bridge::DaemonCommand;
use crate::safety::{ConfirmationToken, ControlPlane, SafetyAction, SafetyContext};
use crate::state::{InterfaceInfo, LinkInfo, MeshStatusInfo, PathEntry, PeerEntry, PeerRole};
use styrene_ipc::types::{
    NetworkOperationInfo, NetworkOperationKind, RequestObservationInfo, StartNetworkOperationInfo,
    StartRequestInfo,
};

use super::NetworkGraph;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NetworkMode {
    Discovery,
    Routes,
    Links,
    Interfaces,
    #[default]
    Combined,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RoleFilter {
    #[default]
    All,
    Styrene,
    Hub,
    PageHost,
    Rns,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum StatusFilter {
    #[default]
    All,
    Online,
    Offline,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FreshnessFilter {
    #[default]
    All,
    Recent,
    Stale,
    Unknown,
}

#[derive(Clone)]
struct NetworkConfirmation {
    label: String,
    target: String,
    parameters: String,
    consequence: String,
    action: SafetyAction,
    command: DaemonCommand,
    token: ConfirmationToken,
}

#[component]
pub fn NetworkPage(
    peers: Vec<PeerEntry>,
    paths: Vec<PathEntry>,
    status: MeshStatusInfo,
    local_hash: String,
    local_name: Option<String>,
    on_select_peer: EventHandler<String>,
    on_browse_page: EventHandler<String>,
    links: Vec<LinkInfo>,
    interfaces: Vec<InterfaceInfo>,
    operations: Vec<NetworkOperationInfo>,
    requests: Vec<RequestObservationInfo>,
    resources: Vec<styrene_ipc::types::ResourceTransferInfo>,
    safety: Memo<SafetyContext>,
    on_network_command: EventHandler<DaemonCommand>,
) -> Element {
    let safety_policy = safety;
    let mut mode = use_signal(NetworkMode::default);
    let mut query = use_signal(String::new);
    let mut capability = use_signal(String::new);
    let mut role = use_signal(RoleFilter::default);
    let mut peer_status = use_signal(StatusFilter::default);
    let mut freshness = use_signal(FreshnessFilter::default);
    let mut target = use_signal(String::new);
    let mut link_id = use_signal(String::new);
    let mut request_path = use_signal(|| "/status".to_string());
    let mut request_data = use_signal(String::new);
    let mut confirmation = use_signal(|| None::<NetworkConfirmation>);
    let capability_reason = safety_policy.read().operate_session_availability().err();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let query_value = query.read().trim().to_ascii_lowercase();
    let capability_value = capability.read().trim().to_ascii_lowercase();
    let filtered_peers: Vec<PeerEntry> = peers
        .iter()
        .filter(|peer| {
            (query_value.is_empty()
                || peer.hash.to_ascii_lowercase().contains(&query_value)
                || peer
                    .name
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(&query_value))
                && (capability_value.is_empty()
                    || peer
                        .capabilities
                        .iter()
                        .any(|item| item.to_ascii_lowercase().contains(&capability_value)))
                && role_matches(peer, *role.read())
                && status_matches(peer, *peer_status.read())
                && freshness_matches(peer, *freshness.read(), now)
        })
        .cloned()
        .collect();
    let visible_hashes: HashSet<&str> =
        filtered_peers.iter().map(|peer| peer.hash.as_str()).collect();
    let filtered_paths: Vec<PathEntry> = paths
        .iter()
        .filter(|path| visible_hashes.contains(path.destination_hash.as_str()))
        .cloned()
        .collect();
    let filtered_links: Vec<LinkInfo> = links
        .iter()
        .filter(|link| visible_hashes.contains(link.peer_hash.as_str()))
        .cloned()
        .collect();

    rsx! {
        div { class: "network-page",
            section { class: "network-operations",
                div { class: "network-operation-heading",
                    div {
                        h2 { "Operator Workflows" }
                        p { "Commands submit typed daemon operations. Progress and outcomes below are daemon observations." }
                    }
                    if let Some(reason) = &capability_reason {
                        span { id: "network-operation-disabled", class: "network-operation-disabled", "Mutations disabled: {reason}" }
                    } else {
                        span { class: "network-operation-ready", "Operate controls negotiated" }
                    }
                }
                div { class: "network-operation-inputs",
                    input {
                        aria_label: "Destination hash",
                        placeholder: "Destination hash",
                        value: "{target}",
                        oninput: move |event| target.set(event.value()),
                    }
                    input {
                        aria_label: "Link ID",
                        placeholder: "Link ID (close/request)",
                        value: "{link_id}",
                        oninput: move |event| link_id.set(event.value()),
                    }
                    input {
                        aria_label: "Request path",
                        placeholder: "/request/path",
                        value: "{request_path}",
                        oninput: move |event| request_path.set(event.value()),
                    }
                    input {
                        aria_label: "Request data",
                        placeholder: "Request data",
                        value: "{request_data}",
                        oninput: move |event| request_data.set(event.value()),
                    }
                }
                div { class: "network-operation-actions",
                    for (kind, label) in [
                        (NetworkOperationKind::Announce, "Announce"),
                        (NetworkOperationKind::PathRequest, "Request path"),
                        (NetworkOperationKind::Probe, "Probe"),
                        (NetworkOperationKind::LinkOpen, "Open link"),
                        (NetworkOperationKind::LinkClose, "Close link"),
                    ] {
                        {
                            let operation_capability = format!("network.{}", kind.as_str());
                            let destructive = kind == NetworkOperationKind::LinkClose;
                            let action = SafetyAction::operate(operation_capability.clone(), destructive);
                            let destination = target.read().trim().to_string();
                            let selected_link = link_id.read().trim().to_string();
                            let input_missing = (matches!(kind, NetworkOperationKind::PathRequest | NetworkOperationKind::LinkOpen) && destination.is_empty())
                                || (matches!(kind, NetworkOperationKind::Probe | NetworkOperationKind::LinkClose) && selected_link.is_empty());
                            let disabled_reason = if let Err(reason) = safety_policy.read().authorize(ControlPlane::Operate, &action) {
                                Some(reason)
                            } else if input_missing {
                                Some(if matches!(kind, NetworkOperationKind::Probe | NetworkOperationKind::LinkClose) { "Enter a link ID".into() } else { "Enter a destination hash".into() })
                            } else {
                                None
                            };
                            let reason_id = format!("network-{}-disabled", kind.as_str());
                            rsx! {
                                button {
                                    disabled: disabled_reason.is_some(),
                                    title: disabled_reason.clone().unwrap_or_else(|| "Submit typed daemon operation".into()),
                                    aria_describedby: disabled_reason.as_ref().map(|_| reason_id.as_str()),
                                    aria_label: format!("{label} using {}", if matches!(kind, NetworkOperationKind::Probe | NetworkOperationKind::LinkClose) { selected_link.as_str() } else { destination.as_str() }),
                                    onclick: move |_| {
                                        let mut request = StartNetworkOperationInfo::default();
                                        request.kind = kind;
                                        request.timeout_ms = 15_000;
                                        if matches!(kind, NetworkOperationKind::PathRequest | NetworkOperationKind::LinkOpen) && !destination.is_empty() {
                                            request.destination_hash = Some(destination.clone());
                                        }
                                        if matches!(kind, NetworkOperationKind::Probe | NetworkOperationKind::LinkClose) && !selected_link.is_empty() {
                                            request.link_id = Some(selected_link.clone());
                                        }
                                        let command = DaemonCommand::StartNetworkOperation(request);
                                        if action.is_destructive() {
                                            if let Some(pending) = network_confirmation(
                                                &safety_policy.read(),
                                                label,
                                                if selected_link.is_empty() { destination.clone() } else { selected_link.clone() },
                                                "timeout=15000 ms",
                                                "Closes the active link and interrupts traffic using it.",
                                                action.clone(),
                                                command,
                                            ) {
                                                confirmation.set(Some(pending));
                                            }
                                        } else {
                                            on_network_command.call(command);
                                        }
                                    },
                                    "{label}"
                                }
                                if let Some(reason) = &disabled_reason {
                                    small { id: "{reason_id}", class: "control-disabled-reason", "{label} disabled: {reason}" }
                                }
                            }
                        }
                    }
                    {
                        let request_action = SafetyAction::operate("network.request", false);
                        let request_reason = if let Err(reason) = safety_policy.read().authorize(ControlPlane::Operate, &request_action) {
                            Some(reason)
                        } else if link_id.read().trim().is_empty() {
                            Some("Enter a link ID".into())
                        } else if request_path.read().trim().is_empty() {
                            Some("Enter a request path".into())
                        } else {
                            None
                        };
                        rsx! {
                    button {
                        disabled: request_reason.is_some(),
                        title: request_reason.clone().unwrap_or_else(|| "Send native request".into()),
                        aria_describedby: request_reason.as_ref().map(|_| "network-request-disabled"),
                        onclick: move |_| {
                            let mut request = StartRequestInfo::default();
                            request.link_id = link_id.read().trim().to_string();
                            request.path = request_path.read().trim().to_string();
                            request.data = request_data.read().as_bytes().to_vec();
                            request.timeout_ms = 15_000;
                            request.max_response_size = 4 * 1024 * 1024;
                            on_network_command.call(DaemonCommand::StartRequest(request));
                        },
                        "Send request"
                    }
                    if let Some(reason) = &request_reason {
                        small { id: "network-request-disabled", class: "control-disabled-reason", "Send request disabled: {reason}" }
                    }
                        }
                    }
                }
                div { class: "network-observation-grid",
                    div {
                        h3 { "Operation observations" }
                        if operations.is_empty() { p { "No operation observations." } }
                        for operation in operations.iter().rev() {
                            article { class: "network-observation",
                                strong { "{operation.kind.as_str()}" }
                                code { "{operation.operation_id}" }
                                span { {operation.outcome.map(|value| value.as_str()).unwrap_or(operation.progress.as_str())} }
                                if let Some(detail) = &operation.detail { p { "{detail}" } }
                                if operation.cancellable && operation.outcome.is_none() {
                                    button {
                                        disabled: safety_policy.read().authorize(ControlPlane::Operate, &SafetyAction::operate(format!("network.{}", operation.kind.as_str()), true)).is_err(),
                                        title: safety_policy.read().authorize(ControlPlane::Operate, &SafetyAction::operate(format!("network.{}", operation.kind.as_str()), true)).err().unwrap_or_else(|| "Confirm cancellation".into()),
                                        aria_describedby: capability_reason.as_ref().map(|_| "network-operation-disabled"),
                                        onclick: {
                                            let id = operation.operation_id.clone();
                                            let capability = format!("network.{}", operation.kind.as_str());
                                            let target = operation.operation_id.clone();
                                            move |_| {
                                                let action = SafetyAction::operate(capability.clone(), true);
                                                if let Some(pending) = network_confirmation(
                                                    &safety_policy.read(),
                                                    "Cancel operation",
                                                    target.clone(),
                                                    "cancel the selected operation ID",
                                                    "Stops the in-flight network operation before it reaches a terminal outcome.",
                                                    action,
                                                    DaemonCommand::CancelNetworkOperation { operation_id: id.clone() },
                                                ) {
                                                    confirmation.set(Some(pending));
                                                }
                                            }
                                        },
                                        aria_label: format!("Cancel {} operation {}", operation.kind.as_str(), operation.operation_id),
                                        "Cancel"
                                    }
                                }
                            }
                        }
                    }
                    div {
                        h3 { "Request and resource observations" }
                        if requests.is_empty() { p { "No native request observations." } }
                        for request in requests.iter().rev() {
                            article { class: "network-observation",
                                strong { "{request.path_hash}" }
                                code { "{request.request_id}" }
                                span { {format!("{:?} · {:.0}%", request.state, request.progress * 100.0)} }
                                if let Some(resource) = &request.resource_hash { span { "resource {resource}" } }
                                if !request.state.is_terminal() {
                                    button {
                                        disabled: safety_policy.read().authorize(ControlPlane::Operate, &SafetyAction::operate("network.request_cancel", true)).is_err(),
                                        title: safety_policy.read().authorize(ControlPlane::Operate, &SafetyAction::operate("network.request_cancel", true)).err().unwrap_or_else(|| "Confirm request cancellation".into()),
                                        aria_describedby: "network-operation-disabled",
                                        onclick: {
                                            let id = request.request_id.clone();
                                            let target = request.request_id.clone();
                                            move |_| {
                                                let action = SafetyAction::operate("network.request_cancel", true);
                                                if let Some(pending) = network_confirmation(
                                                    &safety_policy.read(),
                                                    "Cancel request",
                                                    target.clone(),
                                                    "cancel the selected request ID",
                                                    "Stops the native request and discards any incomplete response.",
                                                    action,
                                                    DaemonCommand::CancelRequest { request_id: id.clone() },
                                                ) {
                                                    confirmation.set(Some(pending));
                                                }
                                            }
                                        },
                                        aria_label: format!("Cancel request {}", request.request_id),
                                        "Cancel"
                                    }
                                }
                            }
                        }
                    }
                    div {
                        h3 { "General resource transfers" }
                        if resources.is_empty() { p { "No resource transfer observations." } }
                        for resource in resources.iter().rev() {
                            article { class: "network-observation",
                                strong { {format!("{:?}", resource.direction)} }
                                code { "{resource.resource_hash}" }
                                span { {format!("{:?} · {:.0}%", resource.state, resource.progress * 100.0)} }
                                span { "{resource.received_bytes}/{resource.total_bytes} bytes" }
                                if resource.cancellable && !resource.state.is_terminal() {
                                    button {
                                        disabled: safety_policy.read().authorize(ControlPlane::Operate, &SafetyAction::operate("network.resource_cancel", true)).is_err(),
                                        title: safety_policy.read().authorize(ControlPlane::Operate, &SafetyAction::operate("network.resource_cancel", true)).err().unwrap_or_else(|| "Confirm resource cancellation".into()),
                                        onclick: {
                                            let hash = resource.resource_hash.clone();
                                            let target = resource.resource_hash.clone();
                                            move |_| {
                                                let action = SafetyAction::operate("network.resource_cancel", true);
                                                if let Some(pending) = network_confirmation(
                                                    &safety_policy.read(),
                                                    "Cancel resource",
                                                    target.clone(),
                                                    "cancel the selected resource hash",
                                                    "Stops this transfer and leaves the resource incomplete.",
                                                    action,
                                                    DaemonCommand::CancelResource { resource_hash: hash.clone() },
                                                ) {
                                                    confirmation.set(Some(pending));
                                                }
                                            }
                                        },
                                        aria_label: format!("Cancel resource {}", resource.resource_hash),
                                        "Cancel"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "network-mode-bar",
                for (value, label) in [
                    (NetworkMode::Discovery, "Discovery"),
                    (NetworkMode::Routes, "Routes"),
                    (NetworkMode::Links, "Links"),
                    (NetworkMode::Interfaces, "Interfaces"),
                    (NetworkMode::Combined, "Combined"),
                ] {
                    button {
                        class: if *mode.read() == value { "network-mode active" } else { "network-mode" },
                        onclick: move |_| mode.set(value),
                        "{label}"
                    }
                }
            }
            div { class: "network-filter-bar",
                input {
                    aria_label: "Search peers",
                    placeholder: "Search name or hash",
                    value: "{query}",
                    oninput: move |event| query.set(event.value()),
                }
                select {
                    aria_label: "Filter by role",
                    onchange: move |event| role.set(parse_role(&event.value())),
                    option { value: "all", "All roles" }
                    option { value: "styrene", "Styrene" }
                    option { value: "hub", "Hub" }
                    option { value: "pages", "Page host" }
                    option { value: "rns", "RNS" }
                }
                select {
                    aria_label: "Filter by status",
                    onchange: move |event| peer_status.set(parse_status(&event.value())),
                    option { value: "all", "All status" }
                    option { value: "online", "Online" }
                    option { value: "offline", "Offline" }
                }
                select {
                    aria_label: "Filter by freshness",
                    onchange: move |event| freshness.set(parse_freshness(&event.value())),
                    option { value: "all", "All freshness" }
                    option { value: "recent", "Recent <1h" }
                    option { value: "stale", "Stale ≥1h" }
                    option { value: "unknown", "Unknown" }
                }
                input {
                    aria_label: "Filter by capability",
                    placeholder: "Capability",
                    value: "{capability}",
                    oninput: move |event| capability.set(event.value()),
                }
                span { class: "network-filter-count", "{filtered_peers.len()}/{peers.len()} peers" }
            }

            match *mode.read() {
                NetworkMode::Discovery => rsx! {
                    DiscoveryView { peers: filtered_peers }
                },
                NetworkMode::Routes => rsx! {
                    RoutesView { paths: filtered_paths, peers: peers.clone() }
                },
                NetworkMode::Links => rsx! {
                    LinksView { links: filtered_links, peers: peers.clone() }
                },
                NetworkMode::Interfaces => rsx! {
                    InterfacesView { interfaces: interfaces.clone() }
                },
                NetworkMode::Combined => rsx! {
                    NetworkGraph {
                        peers: filtered_peers,
                        paths: filtered_paths.clone(),
                        status: status.clone(),
                        local_hash: local_hash.clone(),
                        local_name: local_name.clone(),
                        on_select_peer,
                        on_browse_page,
                        links: filtered_links,
                        interfaces: interfaces.clone(),
                    }
                },
            }
            if let Some(pending) = confirmation.read().clone() {
                div { class: "confirmation-backdrop",
                    div {
                        class: "confirmation-dialog",
                        role: "dialog",
                        aria_modal: "true",
                        aria_labelledby: "network-confirmation-title",
                        onkeydown: move |event: KeyboardEvent| if event.key() == Key::Escape {
                            confirmation.set(None);
                        },
                        h2 { id: "network-confirmation-title", "Confirm {pending.label}" }
                        p { "Target: {pending.target}" }
                        p { "Parameters: {pending.parameters}" }
                        p { "Consequence: {pending.consequence}" }
                        if let Some(capability) = pending.action.required_capability() {
                            p { "Required capability: {capability}" }
                        }
                        div { class: "confirmation-actions",
                            button { autofocus: true, onclick: move |_| confirmation.set(None), "Cancel" }
                            button {
                                class: "danger",
                                disabled: safety_policy.read().confirm(ControlPlane::Operate, &pending.action, pending.token).is_err(),
                                onclick: move |_| {
                                    match safety_policy.read().confirm(ControlPlane::Operate, &pending.action, pending.token) {
                                        Ok(()) => on_network_command.call(pending.command.clone()),
                                        Err(error) => tracing::warn!(target: "dx::safety", %error, "network confirmation rejected"),
                                    }
                                    confirmation.set(None);
                                },
                                "Confirm"
                            }
                        }
                        if let Err(reason) = safety_policy.read().confirm(ControlPlane::Operate, &pending.action, pending.token) {
                            p { class: "control-disabled-reason", "Confirmation unavailable: {reason}" }
                        }
                    }
                }
            }
        }
    }
}

fn network_confirmation(
    safety: &SafetyContext,
    label: impl Into<String>,
    target: String,
    parameters: impl Into<String>,
    consequence: impl Into<String>,
    action: SafetyAction,
    command: DaemonCommand,
) -> Option<NetworkConfirmation> {
    match safety.begin_confirmation(ControlPlane::Operate, &action) {
        Ok(token) => Some(NetworkConfirmation {
            label: label.into(),
            target,
            parameters: parameters.into(),
            consequence: consequence.into(),
            action,
            command,
            token,
        }),
        Err(error) => {
            tracing::warn!(target: "dx::safety", %error, "network action rejected");
            None
        }
    }
}

#[component]
fn DiscoveryView(peers: Vec<PeerEntry>) -> Element {
    rsx! {
        div { class: "network-mode-content",
            h2 { "Discovery Observations" }
            p { class: "network-mode-description", "Accepted announces. Discovery does not imply a route or active link." }
            if peers.is_empty() {
                div { class: "network-mode-empty", "No peer observations match the current filters." }
            } else {
                div { class: "network-record-list",
                    for peer in peers {
                        article { class: "network-record",
                            div { class: "network-record-primary",
                                strong { {peer.name.as_deref().unwrap_or("Unnamed peer")} }
                                code { "{peer.hash}" }
                            }
                            span { class: "network-record-kind", "{role_label(&peer.node_role)}" }
                            span { "{peer.status}" }
                            span { {peer.last_announce.map(format_epoch).unwrap_or_else(|| "unknown".into())} }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RoutesView(paths: Vec<PathEntry>, peers: Vec<PeerEntry>) -> Element {
    rsx! {
        div { class: "network-mode-content",
            h2 { "Route Table" }
            p { class: "network-mode-description", "Daemon-authoritative next hops and hop counts." }
            if paths.is_empty() {
                div { class: "network-mode-empty", "No routes match the current filters." }
            } else {
                div { class: "network-record-list",
                    for path in paths {
                        article { class: "network-record",
                            div { class: "network-record-primary",
                                strong { {peer_label(&peers, &path.destination_hash)} }
                                code { "{path.destination_hash}" }
                            }
                            span { class: "network-record-kind", "{path.hops} hops" }
                            span { "via {peer_label(&peers, &path.next_hop)}" }
                            span { "{path.interface}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn LinksView(links: Vec<LinkInfo>, peers: Vec<PeerEntry>) -> Element {
    rsx! {
        div { class: "network-mode-content",
            h2 { "Link Lifecycle" }
            p { class: "network-mode-description", "Observed RNS link state and round-trip telemetry." }
            if links.is_empty() {
                div { class: "network-mode-empty", "No links match the current filters." }
            } else {
                div { class: "network-record-list",
                    for link in links {
                        article { class: "network-record",
                            div { class: "network-record-primary",
                                strong { {peer_label(&peers, &link.peer_hash)} }
                                code { "{link.peer_hash}" }
                            }
                            span { class: "network-record-kind", "{link.status}" }
                            span { {format!("{:?}", link.activity)} }
                            code { "{link.link_id}" }
                            span { {link.rtt_ms.map(|value| format!("{value:.0} ms")).unwrap_or_else(|| "RTT unknown".into())} }
                            span { {if link.observation.stale { "stale".to_string() } else { format_epoch(link.timestamp) }} }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn InterfacesView(interfaces: Vec<InterfaceInfo>) -> Element {
    rsx! {
        div { class: "network-mode-content",
            h2 { "Transport Interfaces" }
            p { class: "network-mode-description", "Configured transport interfaces and observed byte counters." }
            if interfaces.is_empty() {
                div { class: "network-mode-empty", "No interface telemetry is available." }
            } else {
                div { class: "network-record-list",
                    for interface in interfaces {
                        article { class: "network-record",
                            div { class: "network-record-primary",
                                strong { "{interface.name}" }
                                code { "{interface.hash}" }
                            }
                            span { class: "network-record-kind", "{interface.status}" }
                            span { "type {interface.kind} · mode {interface.mode} · enabled {interface.enabled}" }
                            span { {format!("local {}", interface.local_endpoint.as_deref().unwrap_or("unknown"))} }
                            span { {format!("remote {}", interface.remote_endpoint.as_deref().unwrap_or("unknown"))} }
                            span { {format!("host {} · port {}", interface.host.as_deref().unwrap_or("unknown"), interface.port.map(|value| value.to_string()).unwrap_or_else(|| "unknown".into()))} }
                            span { {format!("parent {} · peers {}", interface.parent_hash.as_deref().unwrap_or("none"), interface.peers_connected)} }
                            span { "TX {format_bytes(interface.tx_bytes)}" }
                            span { "RX {format_bytes(interface.rx_bytes)}" }
                            span { {format!(
                                "source {:?} · observed {:?} · age {:?} · threshold {:?} · stale {} · generation {:?} · correlation {}",
                                interface.observation.source,
                                interface.observation.observed_at,
                                interface.observation.age_secs,
                                interface.observation.freshness_threshold_secs,
                                interface.observation.stale,
                                interface.observation.connection_generation,
                                interface.observation.correlation_id.as_deref().unwrap_or("none")
                            )} }
                        }
                    }
                }
            }
        }
    }
}

fn role_matches(peer: &PeerEntry, filter: RoleFilter) -> bool {
    match filter {
        RoleFilter::All => true,
        RoleFilter::Styrene => peer.node_role == PeerRole::Styrene,
        RoleFilter::Hub => peer.node_role == PeerRole::Hub,
        RoleFilter::PageHost => peer.node_role == PeerRole::PageHost,
        RoleFilter::Rns => peer.node_role == PeerRole::Rns,
    }
}

fn status_matches(peer: &PeerEntry, filter: StatusFilter) -> bool {
    let online = peer.status != "offline" && !peer.status.is_empty();
    match filter {
        StatusFilter::All => true,
        StatusFilter::Online => online,
        StatusFilter::Offline => !online,
    }
}

fn freshness_matches(peer: &PeerEntry, filter: FreshnessFilter, now: i64) -> bool {
    match (filter, peer.last_announce) {
        (FreshnessFilter::All, _) => true,
        (FreshnessFilter::Unknown, None) => true,
        (FreshnessFilter::Recent, Some(timestamp)) => now.saturating_sub(timestamp) < 3600,
        (FreshnessFilter::Stale, Some(timestamp)) => now.saturating_sub(timestamp) >= 3600,
        _ => false,
    }
}

fn parse_role(value: &str) -> RoleFilter {
    match value {
        "styrene" => RoleFilter::Styrene,
        "hub" => RoleFilter::Hub,
        "pages" => RoleFilter::PageHost,
        "rns" => RoleFilter::Rns,
        _ => RoleFilter::All,
    }
}

fn parse_status(value: &str) -> StatusFilter {
    match value {
        "online" => StatusFilter::Online,
        "offline" => StatusFilter::Offline,
        _ => StatusFilter::All,
    }
}

fn parse_freshness(value: &str) -> FreshnessFilter {
    match value {
        "recent" => FreshnessFilter::Recent,
        "stale" => FreshnessFilter::Stale,
        "unknown" => FreshnessFilter::Unknown,
        _ => FreshnessFilter::All,
    }
}

fn role_label(role: &PeerRole) -> &'static str {
    match role {
        PeerRole::Styrene => "Styrene",
        PeerRole::Hub => "Hub",
        PeerRole::PageHost => "Page host",
        PeerRole::Rns => "RNS",
    }
}

fn peer_label(peers: &[PeerEntry], hash: &str) -> String {
    peers
        .iter()
        .find(|peer| peer.hash == hash)
        .and_then(|peer| peer.name.clone())
        .unwrap_or_else(|| hash[..8.min(hash.len())].to_string())
}

fn format_epoch(timestamp: i64) -> String {
    if timestamp <= 0 {
        "unknown".into()
    } else {
        format!("epoch {timestamp}")
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(role: PeerRole, status: &str, last_announce: Option<i64>) -> PeerEntry {
        PeerEntry {
            hash: "peer".into(),
            identity_hash: None,
            name: Some("Test peer".into()),
            status: status.into(),
            node_role: role,
            capabilities: vec!["pages".into()],
            version: None,
            last_announce,
            announce_count: 1,
        }
    }

    #[test]
    fn filters_keep_role_status_and_freshness_semantics_independent() {
        let value = peer(PeerRole::Hub, "online", Some(9_500));
        assert!(role_matches(&value, RoleFilter::Hub));
        assert!(status_matches(&value, StatusFilter::Online));
        assert!(freshness_matches(&value, FreshnessFilter::Recent, 10_000));
        assert!(!freshness_matches(&value, FreshnessFilter::Stale, 10_000));
    }

    #[test]
    fn unknown_freshness_is_not_treated_as_recent() {
        let value = peer(PeerRole::Rns, "offline", None);
        assert!(freshness_matches(&value, FreshnessFilter::Unknown, 10_000));
        assert!(!freshness_matches(&value, FreshnessFilter::Recent, 10_000));
    }
}
