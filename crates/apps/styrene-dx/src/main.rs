//! Styrene DX — Dioxus desktop app for mesh chat and fleet management.
//!
//! Connects to a running `styrened` daemon via IPC, or boots one in-process
//! if no daemon is available. Provides graphical peer management, messaging,
//! Micron page browsing, and mesh status visualization.
//!
//! Run: `cargo run -p styrene-dx`

use std::sync::Arc;

use dioxus::prelude::*;
use tokio::sync::Mutex;

mod components;
mod daemon_bridge;
mod state;

fn main() {
    // Initialize tracing — RUST_LOG=dx=debug for bridge diagnostics
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dx=info,styrene=info".parse().unwrap()),
        )
        .with_target(true)
        .compact()
        .init();

    dioxus::LaunchBuilder::new()
        .with_cfg(
            dioxus::desktop::Config::new().with_window(
                dioxus::desktop::WindowBuilder::new()
                    .with_title("Styrene Mesh")
                    .with_always_on_top(false),
            ),
        )
        .launch(App);
}

#[component]
fn App() -> Element {
    // Reactive state
    let mut identity = use_signal(|| None::<state::NodeIdentity>);
    let mut peers = use_signal(Vec::<state::PeerEntry>::new);
    let mut status = use_signal(state::MeshStatusInfo::default);
    let mut connected = use_signal(|| false);
    let mut connection_mode = use_signal(|| String::from("connecting..."));
    let mut messages = use_signal(Vec::<state::ChatMessage>::new);
    let mut selected_peer = use_signal(|| None::<String>);
    let mut active_tab = use_signal(state::Tab::default);
    let mut path_table = use_signal(Vec::<state::PathEntry>::new);
    let mut conversations = use_signal(Vec::<state::ConversationEntry>::new);
    let mut links = use_signal(Vec::<state::LinkInfo>::new);
    let mut interfaces = use_signal(Vec::<state::InterfaceInfo>::new);
    let mut announce_log = use_signal(Vec::<state::AnnounceEvent>::new);

    // Bridge handle — shared with UI for RPC calls (send_chat, browse_page, etc.)
    let mut bridge: Signal<Option<Arc<Mutex<daemon_bridge::DaemonBridge>>>> = use_signal(|| None);

    // Page browsing state
    let mut page_content = use_signal(|| None::<state::PageView>);

    // Command channel — UI sends commands, spawned task processes them
    let mut cmd_tx: Signal<
        Option<tokio::sync::mpsc::UnboundedSender<daemon_bridge::DaemonCommand>>,
    > = use_signal(|| None);

    // Boot daemon connection + process events
    let _daemon_task =
        use_coroutine(move |_rx: UnboundedReceiver<daemon_bridge::DaemonCommand>| async move {
            match daemon_bridge::connect().await {
                Ok((br, mut event_rx, mode)) => {
                    connection_mode.set(match mode {
                        daemon_bridge::ConnectionMode::Ipc => "IPC".into(),
                        daemon_bridge::ConnectionMode::Embedded => "Embedded".into(),
                    });
                    bridge.set(Some(br.clone()));
                    connected.set(true);

                    // Spawn command handler task (on Dioxus runtime so signals are accessible)
                    let (tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel();
                    let tx_init = tx.clone();
                    cmd_tx.set(Some(tx));

                    let cmd_bridge = br.clone();
                    spawn(async move {
                        while let Some(cmd) = cmd_rx.recv().await {
                            handle_ui_command(
                                cmd,
                                &cmd_bridge,
                                &mut messages,
                                &mut page_content,
                                &mut path_table,
                                &mut conversations,
                                &mut interfaces,
                            )
                            .await;
                        }
                    });

                    // Initial data fetch
                    let _ = tx_init.send(daemon_bridge::DaemonCommand::RefreshPathTable);
                    let _ = tx_init.send(daemon_bridge::DaemonCommand::RefreshInterfaces);
                    let _ = tx_init.send(daemon_bridge::DaemonCommand::LoadConversations);

                    // Process daemon events
                    while let Some(ev) = event_rx.recv().await {
                        handle_daemon_event(
                            ev,
                            &mut identity,
                            &mut status,
                            &mut peers,
                            &mut messages,
                            &mut connected,
                            &mut connection_mode,
                            &mut path_table,
                            &mut links,
                            &mut announce_log,
                        );
                    }
                }
                Err(e) => {
                    connection_mode.set(format!("failed: {e}"));
                }
            }
        });

    let id_display = identity
        .read()
        .as_ref()
        .map(|id| {
            let name = id.display_name.as_deref().unwrap_or("unnamed");
            let hash_short = &id.hash[..12.min(id.hash.len())];
            format!("{name} ({hash_short}...)")
        })
        .unwrap_or_else(|| "loading...".into());

    let local_hash = identity.read().as_ref().map(|id| id.hash.clone()).unwrap_or_default();
    let local_name = identity.read().as_ref().and_then(|id| id.display_name.clone());

    // Chat input state
    let mut chat_input = use_signal(String::new);

    // Helper to send commands to the daemon
    let send_cmd = move |cmd: daemon_bridge::DaemonCommand| {
        if let Some(ref tx) = *cmd_tx.read() {
            let _ = tx.send(cmd);
        }
    };

    rsx! {
        style { {include_str!("assets/style.css")} }

        div { class: "app",
            // Top bar
            div { class: "topbar",
                span { class: "brand", "Styrene Mesh" }
                span { class: "identity", "{id_display}" }
                span {
                    class: if *connected.read() { "badge connected" } else { "badge disconnected" },
                    "{connection_mode}"
                }
            }

            // Tab bar
            div { class: "tab-bar",
                div {
                    class: if *active_tab.read() == state::Tab::Network { "tab active" } else { "tab" },
                    onclick: move |_| active_tab.set(state::Tab::Network),
                    "Network"
                }
                div {
                    class: if *active_tab.read() == state::Tab::Conversations { "tab active" } else { "tab" },
                    onclick: move |_| active_tab.set(state::Tab::Conversations),
                    "Conversations"
                }
                div {
                    class: if *active_tab.read() == state::Tab::Pages { "tab active" } else { "tab" },
                    onclick: move |_| active_tab.set(state::Tab::Pages),
                    "Pages"
                }
            }

            // Body
            div { class: "body",
                match *active_tab.read() {
                    state::Tab::Network => rsx! {
                        components::NetworkGraph {
                            peers: peers.read().clone(),
                            paths: path_table.read().clone(),
                            status: status.read().clone(),
                            local_hash: local_hash.clone(),
                            local_name: local_name.clone(),
                            on_select_peer: move |hash: String| {
                                selected_peer.set(Some(hash.clone()));
                                send_cmd(daemon_bridge::DaemonCommand::LoadMessages {
                                    peer_hash: hash,
                                });
                                active_tab.set(state::Tab::Conversations);
                            },
                            links: links.read().clone(),
                            interfaces: interfaces.read().clone(),
                            announce_log: announce_log.read().clone(),
                            path_entries: path_table.read().clone(),
                            on_browse_page: move |host_hash: String| {
                                send_cmd(daemon_bridge::DaemonCommand::BrowsePage {
                                    host: host_hash,
                                    path: "/".into(),
                                });
                                active_tab.set(state::Tab::Pages);
                            },
                        }
                    },

                    state::Tab::Conversations => rsx! {
                        // Sidebar — active conversations only
                        div { class: "sidebar",
                            div { class: "sidebar-header", "Conversations" }
                            if conversations.read().is_empty() {
                                div { class: "sidebar-empty",
                                    "No conversations yet. Select a peer from the Network tab to start one."
                                }
                            }
                            for convo in conversations.read().iter() {
                                {
                                    let hash = convo.peer_hash.clone();
                                    let is_selected = selected_peer.read().as_deref() == Some(&hash);
                                    let name = convo.peer_name.clone()
                                        .unwrap_or_else(|| hash[..8.min(hash.len())].to_string());
                                    let preview = convo.last_message.clone()
                                        .map(|m| if m.len() > 40 { format!("{}...", &m[..40]) } else { m })
                                        .unwrap_or_default();
                                    let time = convo.last_timestamp.map(format_timestamp).unwrap_or_default();
                                    let unread = convo.unread_count;
                                    let load_hash = hash.clone();
                                    rsx! {
                                        div {
                                            class: if is_selected { "convo-item selected" } else { "convo-item" },
                                            onclick: move |_| {
                                                selected_peer.set(Some(hash.clone()));
                                                send_cmd(daemon_bridge::DaemonCommand::LoadMessages {
                                                    peer_hash: load_hash.clone(),
                                                });
                                            },
                                            div { class: "convo-row",
                                                span { class: "convo-name", "{name}" }
                                                if !time.is_empty() {
                                                    span { class: "convo-time", "{time}" }
                                                }
                                            }
                                            div { class: "convo-row",
                                                span { class: "convo-preview", "{preview}" }
                                                if unread > 0 {
                                                    span { class: "convo-unread", "{unread}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Main content — chat
                        div { class: "main",
                            if let Some(ref peer_hash) = *selected_peer.read() {
                                {
                                    let peer_name = peers.read().iter()
                                        .find(|p| p.hash == *peer_hash)
                                        .and_then(|p| p.name.clone())
                                        .unwrap_or_else(|| peer_hash[..8].to_string());
                                    let ph = peer_hash.clone();
                                    rsx! {
                                        div { class: "peer-detail",
                                            div { class: "chat-header",
                                                h2 { "{peer_name}" }
                                                p { class: "peer-hash", "{peer_hash}" }
                                            }
                                            div { class: "chat-area",
                                                for msg in messages.read().iter().filter(|m|
                                                    m.source == *peer_hash || m.destination == *peer_hash
                                                ) {
                                                    div {
                                                        class: if msg.is_outgoing { "message sent" } else { "message received" },
                                                        div { class: "message-content", "{msg.content}" }
                                                        div { class: "message-meta",
                                                            span { {format_timestamp(msg.timestamp)} }
                                                            if msg.is_outgoing {
                                                                span { class: "message-status",
                                                                    {match msg.status.as_str() {
                                                                        "delivered" => " ✓✓",
                                                                        "read" => " ✓✓",
                                                                        "failed" => " ✗",
                                                                        "pending" | "" => " ✓",
                                                                        _ => "",
                                                                    }}
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            // Chat input
                                            div { class: "chat-input-bar",
                                                input {
                                                    class: "chat-input",
                                                    r#type: "text",
                                                    placeholder: "Type a message...",
                                                    value: "{chat_input}",
                                                    oninput: move |evt| chat_input.set(evt.value()),
                                                    onkeypress: {
                                                        let ph2 = ph.clone();
                                                        move |evt: KeyboardEvent| {
                                                            if evt.key() == Key::Enter {
                                                                let content = chat_input.read().clone();
                                                                if !content.trim().is_empty() {
                                                                    send_cmd(daemon_bridge::DaemonCommand::SendChat {
                                                                        peer_hash: ph2.clone(),
                                                                        content,
                                                                    });
                                                                    send_cmd(daemon_bridge::DaemonCommand::LoadConversations);
                                                                    chat_input.set(String::new());
                                                                }
                                                            }
                                                        }
                                                    },
                                                }
                                                button {
                                                    class: "chat-send-btn",
                                                    disabled: chat_input.read().trim().is_empty(),
                                                    onclick: {
                                                        let ph3 = ph.clone();
                                                        move |_| {
                                                            let content = chat_input.read().clone();
                                                            if !content.trim().is_empty() {
                                                                send_cmd(daemon_bridge::DaemonCommand::SendChat {
                                                                    peer_hash: ph3.clone(),
                                                                    content,
                                                                });
                                                                send_cmd(daemon_bridge::DaemonCommand::LoadConversations);
                                                                chat_input.set(String::new());
                                                            }
                                                        }
                                                    },
                                                    "Send"
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                div { class: "empty-state",
                                    p { "Select a peer to start a conversation" }
                                }
                            }
                        }
                    },

                    state::Tab::Pages => rsx! {
                        // Page host sidebar
                        div { class: "sidebar",
                            div { class: "sidebar-header", "Page Hosts" }
                            div {
                                class: "peer-item",
                                onclick: move |_| {
                                    send_cmd(daemon_bridge::DaemonCommand::BrowsePage {
                                        host: String::new(),
                                        path: "/".into(),
                                    });
                                },
                                span { class: "peer-icon", style: "color: var(--accent);", "●" }
                                span { class: "peer-name", "Local Node" }
                            }
                            for peer in peers.read().iter().filter(|p|
                                p.node_role == state::PeerRole::PageHost || p.node_role == state::PeerRole::Hub
                            ) {
                                {
                                    let hash = peer.hash.clone();
                                    let name = peer.name.clone().unwrap_or_else(|| hash[..8.min(hash.len())].to_string());
                                    rsx! {
                                        div {
                                            class: "peer-item",
                                            onclick: move |_| {
                                                send_cmd(daemon_bridge::DaemonCommand::BrowsePage {
                                                    host: hash.clone(),
                                                    path: "/".into(),
                                                });
                                            },
                                            span { class: "peer-icon", style: "color: var(--green);", "●" }
                                            span { class: "peer-name", "{name}" }
                                        }
                                    }
                                }
                            }
                        }

                        components::PageBrowser {
                            page: page_content.read().clone(),
                            on_navigate: move |url: String| {
                                let (host, path) = parse_page_url(&url);
                                send_cmd(daemon_bridge::DaemonCommand::BrowsePage {
                                    host,
                                    path,
                                });
                            },
                        }
                    },
                }
            }
        }
    }
}

// ── Event & command handlers ──────────────────────────────────────────────

fn handle_daemon_event(
    ev: daemon_bridge::DaemonEvent,
    identity: &mut Signal<Option<state::NodeIdentity>>,
    status: &mut Signal<state::MeshStatusInfo>,
    peers: &mut Signal<Vec<state::PeerEntry>>,
    messages: &mut Signal<Vec<state::ChatMessage>>,
    connected: &mut Signal<bool>,
    connection_mode: &mut Signal<String>,
    path_table: &mut Signal<Vec<state::PathEntry>>,
    links: &mut Signal<Vec<state::LinkInfo>>,
    announce_log: &mut Signal<Vec<state::AnnounceEvent>>,
) {
    match ev {
        daemon_bridge::DaemonEvent::Identity(info) => {
            identity.set(Some(state::NodeIdentity {
                hash: info.destination_hash.clone(),
                display_name: if info.display_name.is_empty() {
                    None
                } else {
                    Some(info.display_name.clone())
                },
            }));
        }
        daemon_bridge::DaemonEvent::Status(s) => {
            status.set(state::MeshStatusInfo {
                transport_active: s.transport_enabled,
                peer_count: s.device_count,
                link_count: s.active_links,
                interface_count: s.interface_count,
                propagation_enabled: s.propagation_enabled,
                uptime: s.uptime,
                version: s.daemon_version.clone(),
            });
        }
        daemon_bridge::DaemonEvent::PeerDiscovered(dev) => {
            let mut p = peers.write();
            if !p.iter().any(|e| e.hash == dev.destination_hash) {
                let parsed = state::parse_announce_name(&dev.name);
                let role = if dev.is_styrene_node || parsed.is_styrene {
                    parsed.role
                } else if dev.device_type == "page_host" {
                    state::PeerRole::PageHost
                } else {
                    state::PeerRole::Rns
                };
                let display =
                    if parsed.display_name.is_empty() { None } else { Some(parsed.display_name) };
                let peer_hash = dev.destination_hash.clone();
                let peer_name = display.clone();
                let peer_role = role.clone();
                p.push(state::PeerEntry {
                    hash: dev.destination_hash,
                    name: display,
                    status: dev.status,
                    node_role: role,
                    capabilities: parsed.capabilities,
                    version: parsed.version,
                    last_announce: dev.last_announce,
                    announce_count: dev.announce_count,
                });
                // Push to announce log
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let mut log = announce_log.write();
                log.push(state::AnnounceEvent {
                    peer_hash,
                    peer_name: peer_name,
                    timestamp: dev.last_announce.unwrap_or(now),
                    node_role: peer_role,
                });
                // Cap at 200 entries
                if log.len() > 200 {
                    let excess = log.len() - 200;
                    log.drain(..excess);
                }
            }
        }
        daemon_bridge::DaemonEvent::MessageReceived(msg) => {
            messages.write().push(state::ChatMessage {
                id: msg.id,
                source: msg.source_hash,
                destination: msg.destination_hash,
                content: msg.content,
                timestamp: msg.timestamp,
                is_outgoing: msg.is_outgoing,
                status: msg.status,
            });
        }
        daemon_bridge::DaemonEvent::MessageStatusChanged { id, status: new_status } => {
            let mut msgs = messages.write();
            if let Some(msg) = msgs.iter_mut().find(|m| m.id == id) {
                msg.status = new_status;
            }
        }
        daemon_bridge::DaemonEvent::PathTable(entries) => {
            let multi_hop = entries.iter().filter(|e| e.next_hop != e.destination_hash).count();
            let unique_ifaces: std::collections::HashSet<&str> =
                entries.iter().map(|e| e.interface.as_str()).collect();
            tracing::info!(target: "dx::graph", total = entries.len(), multi_hop, interfaces = unique_ifaces.len(), "path table updated");
            path_table.set(
                entries
                    .into_iter()
                    .map(|e| state::PathEntry {
                        destination_hash: e.destination_hash,
                        hops: e.hops,
                        next_hop: e.next_hop,
                        interface: e.interface,
                    })
                    .collect(),
            );
        }
        daemon_bridge::DaemonEvent::LinkUpdate { peer_hash, status: link_status, rtt_ms } => {
            let mut l = links.write();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            if let Some(existing) = l.iter_mut().find(|li| li.peer_hash == peer_hash) {
                existing.status = link_status;
                existing.rtt_ms = rtt_ms;
                existing.timestamp = now;
            } else {
                l.push(state::LinkInfo { peer_hash, status: link_status, rtt_ms, timestamp: now });
            }
        }
        daemon_bridge::DaemonEvent::Disconnected(reason) => {
            connected.set(false);
            connection_mode.set(format!("disconnected: {reason}"));
        }
        _ => {}
    }
}

async fn handle_ui_command(
    cmd: daemon_bridge::DaemonCommand,
    bridge: &Arc<Mutex<daemon_bridge::DaemonBridge>>,
    messages: &mut Signal<Vec<state::ChatMessage>>,
    page_content: &mut Signal<Option<state::PageView>>,
    path_table: &mut Signal<Vec<state::PathEntry>>,
    conversations: &mut Signal<Vec<state::ConversationEntry>>,
    interfaces: &mut Signal<Vec<state::InterfaceInfo>>,
) {
    match cmd {
        daemon_bridge::DaemonCommand::SendChat { peer_hash, content } => {
            let mut br = bridge.lock().await;
            match br.send_chat(&peer_hash, &content).await {
                Ok(msg_id) => {
                    messages.write().push(state::ChatMessage {
                        id: msg_id,
                        source: String::new(), // local
                        destination: peer_hash,
                        content,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0),
                        is_outgoing: true,
                        status: "pending".into(),
                    });
                }
                Err(e) => eprintln!("[dx] send_chat failed: {e}"),
            }
        }
        daemon_bridge::DaemonCommand::BrowsePage { host, path } => {
            page_content.set(Some(state::PageView {
                host: host.clone(),
                path: path.clone(),
                content: None,
                loading: true,
                error: None,
            }));
            let mut br = bridge.lock().await;
            match br.browse_page(&host, &path).await {
                Ok(source) => {
                    page_content.set(Some(state::PageView {
                        host,
                        path,
                        content: Some(source),
                        loading: false,
                        error: None,
                    }));
                }
                Err(e) => {
                    page_content.set(Some(state::PageView {
                        host,
                        path,
                        content: None,
                        loading: false,
                        error: Some(e),
                    }));
                }
            }
        }
        daemon_bridge::DaemonCommand::RefreshPathTable => {
            let mut br = bridge.lock().await;
            match br.path_table().await {
                Ok(entries) => {
                    path_table.set(
                        entries
                            .into_iter()
                            .map(|e| state::PathEntry {
                                destination_hash: e.destination_hash,
                                hops: e.hops,
                                next_hop: e.next_hop,
                                interface: e.interface,
                            })
                            .collect(),
                    );
                }
                Err(e) => eprintln!("[dx] path_table failed: {e}"),
            }
        }
        daemon_bridge::DaemonCommand::RefreshInterfaces => {
            let mut br = bridge.lock().await;
            match br.interface_stats().await {
                Ok(ifaces) => {
                    interfaces.set(
                        ifaces
                            .into_iter()
                            .map(|i| state::InterfaceInfo {
                                name: i.name,
                                hash: i.hash,
                                status: i.status,
                                tx_bytes: i.tx_bytes,
                                rx_bytes: i.rx_bytes,
                            })
                            .collect(),
                    );
                }
                Err(e) => tracing::warn!(target: "dx::iface", %e, "interface stats failed"),
            }
        }
        daemon_bridge::DaemonCommand::LoadConversations => {
            let mut br = bridge.lock().await;
            match br.query_conversations().await {
                Ok(convos) => {
                    conversations.set(
                        convos
                            .into_iter()
                            .filter_map(|c| {
                                let peer_hash =
                                    c.get("peer_hash").and_then(|v| v.as_str())?.to_string();
                                let peer_name = c
                                    .get("peer_name")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    .map(|s| s.to_string());
                                let last_message = c
                                    .get("last_message_content")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                let last_timestamp =
                                    c.get("last_message_timestamp").and_then(|v| v.as_i64());
                                let unread_count =
                                    c.get("unread_count").and_then(|v| v.as_u64()).unwrap_or(0)
                                        as u32;
                                let message_count =
                                    c.get("message_count").and_then(|v| v.as_u64()).unwrap_or(0)
                                        as u32;
                                Some(state::ConversationEntry {
                                    peer_hash,
                                    peer_name,
                                    last_message,
                                    last_timestamp,
                                    unread_count,
                                    message_count,
                                })
                            })
                            .collect(),
                    );
                }
                Err(e) => tracing::warn!(target: "dx::chat", %e, "load conversations failed"),
            }
        }
        daemon_bridge::DaemonCommand::LoadMessages { peer_hash } => {
            let mut br = bridge.lock().await;
            match br.query_messages(&peer_hash, 50).await {
                Ok(msgs) => {
                    // Replace messages for this peer (not append)
                    let mut all = messages.read().clone();
                    all.retain(|m| m.source != peer_hash && m.destination != peer_hash);
                    for msg in msgs {
                        all.push(state::ChatMessage {
                            id: msg.id,
                            source: msg.source_hash,
                            destination: msg.destination_hash,
                            content: msg.content,
                            timestamp: msg.timestamp,
                            is_outgoing: msg.is_outgoing,
                            status: msg.status,
                        });
                    }
                    all.sort_by_key(|m| m.timestamp);
                    messages.set(all);
                }
                Err(e) => tracing::warn!(target: "dx::chat", %e, "load messages failed"),
            }
        }
        _ => {}
    }
}

fn format_timestamp(ts: i64) -> String {
    if ts == 0 {
        return String::new();
    }
    // Simple HH:MM format
    let secs = ts % 86400;
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    format!("{hours:02}:{mins:02}")
}

fn parse_page_url(url: &str) -> (String, String) {
    // Format: "hash:/path" or just "/path" (local)
    if let Some(idx) = url.find(":/") {
        let host = url[..idx].to_string();
        let path = url[idx + 1..].to_string();
        (host, path)
    } else if url.starts_with('/') {
        (String::new(), url.to_string())
    } else {
        (url.to_string(), "/".to_string())
    }
}
