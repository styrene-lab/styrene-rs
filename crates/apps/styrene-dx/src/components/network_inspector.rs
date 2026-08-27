use dioxus::prelude::*;

use crate::state::{GraphNode, GraphNodeType, LinkInfo, MeshStatusInfo, PeerEntry};

#[component]
pub(crate) fn NetworkInspector(
    selected: Option<GraphNode>,
    peers: Vec<PeerEntry>,
    links: Vec<LinkInfo>,
    status: MeshStatusInfo,
    mut search_query: Signal<String>,
    search_count: usize,
    hidden_count: usize,
    mut show_all: Signal<bool>,
    mut physics_on: Signal<bool>,
    mut nodes: Signal<Vec<GraphNode>>,
    on_select_peer: EventHandler<String>,
    on_browse_page: EventHandler<String>,
) -> Element {
    let online_count =
        peers.iter().filter(|peer| peer.status != "offline" && !peer.status.is_empty()).count();
    rsx! {
        aside { class: "graph-sidebar", aria_label: "Network inspector", aria_live: "polite",
            div {
                h3 { "Search" }
                input {
                    class: "graph-search",
                    r#type: "text",
                    aria_label: "Search network nodes",
                    placeholder: "Name or hash...",
                    value: "{search_query}",
                    oninput: move |event| search_query.set(event.value()),
                }
                if !search_query.read().is_empty() {
                    div { class: "search-count", "{search_count} matches" }
                }
            }
            if let Some(detail) = selected {
                {
                    let peer = peers.iter().find(|peer| peer.hash == detail.id);
                    let link = links.iter().find(|link| link.peer_hash == detail.id);
                    let last_seen = peer
                        .and_then(|peer| peer.last_announce)
                        .map(format_relative_time)
                        .unwrap_or_else(|| "unknown".into());
                    let rtt = link
                        .and_then(|link| link.rtt_ms)
                        .map(|value| format!("{value:.0} ms"))
                        .unwrap_or_else(|| "unknown".into());
                    let message_hash = detail.id.clone();
                    let content_hash = detail.id.clone();
                    rsx! {
                        div { class: "node-detail",
                            h3 { "Selected Node" }
                            div { class: "detail-name", "{detail.label}" }
                            div { class: "detail-hash", "{detail.id}" }
                            div { class: "detail-type", "{detail.type_label()}" }
                            div { class: "detail-meta",
                                span { class: "detail-meta-label", "Last seen" }
                                span { class: "detail-meta-value", "{last_seen}" }
                            }
                            div { class: "detail-meta",
                                span { class: "detail-meta-label", "RTT" }
                                span { class: "detail-meta-value", "{rtt}" }
                            }
                            if !detail.capabilities.is_empty() {
                                div { class: "detail-caps",
                                    for capability in detail.capabilities {
                                        span { class: "cap-badge", "{capability}" }
                                    }
                                }
                            }
                            div { class: "detail-actions",
                                if !matches!(detail.node_type, GraphNodeType::Local | GraphNodeType::Interface { .. }) {
                                    button {
                                        class: "action-btn primary",
                                        onclick: move |_| on_select_peer.call(message_hash.clone()),
                                        "Message"
                                    }
                                }
                                if matches!(detail.node_type, GraphNodeType::PageHost { .. } | GraphNodeType::Hub { .. }) {
                                    button {
                                        class: "action-btn",
                                        onclick: move |_| on_browse_page.call(content_hash.clone()),
                                        "Browse Content"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div {
                h3 { "Visibility" }
                button {
                    class: "physics-toggle",
                    aria_pressed: if *show_all.read() { "true" } else { "false" },
                    onclick: move |_| show_all.toggle(),
                    span { class: if *show_all.read() { "toggle-indicator on" } else { "toggle-indicator off" } }
                    span { class: "toggle-label", if *show_all.read() { "All Nodes" } else { "Named Only" } }
                }
                if hidden_count > 0 {
                    div { class: "search-count", "{hidden_count} anonymous nodes hidden" }
                }
            }
            div {
                h3 { "Layout" }
                button {
                    class: "physics-toggle",
                    aria_pressed: if *physics_on.read() { "true" } else { "false" },
                    onclick: move |_| {
                        if !*physics_on.read() {
                            for node in nodes.write().iter_mut() {
                                node.vx += 0.5;
                                node.vy += 0.5;
                            }
                        }
                        physics_on.toggle();
                    },
                    span { class: if *physics_on.read() { "toggle-indicator on" } else { "toggle-indicator off" } }
                    span { class: "toggle-label", if *physics_on.read() { "Live Layout" } else { "Layout Paused" } }
                }
            }
            div {
                h3 { "Network Stats" }
                Stat { label: "Peers", value: peers.len().to_string() }
                Stat { label: "Online", value: online_count.to_string() }
                Stat { label: "Links", value: status.link_count.to_string() }
                Stat { label: "Interfaces", value: status.interface_count.to_string() }
                Stat { label: "Uptime", value: format_uptime(status.uptime) }
            }
            div {
                h3 { "Edge Legend" }
                div { class: "graph-legend",
                    Legend { class: "interface", label: "Interface" }
                    Legend { class: "route", label: "Route" }
                    Legend { class: "link", label: "Link" }
                    Legend { class: "association", label: "Association" }
                }
            }
        }
    }
}

#[component]
fn Stat(label: &'static str, value: String) -> Element {
    rsx! {
        div { class: "graph-stat",
            span { class: "graph-stat-label", "{label}" }
            span { class: "graph-stat-value", "{value}" }
        }
    }
}

#[component]
fn Legend(class: &'static str, label: &'static str) -> Element {
    rsx! {
        div { class: "legend-item",
            span { class: "legend-line {class}" }
            "{label}"
        }
    }
}

fn format_relative_time(timestamp: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let seconds = now.saturating_sub(timestamp);
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86_400)
    }
}

fn format_uptime(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    } else {
        format!("{}d {}h", seconds / 86_400, (seconds % 86_400) / 3600)
    }
}
