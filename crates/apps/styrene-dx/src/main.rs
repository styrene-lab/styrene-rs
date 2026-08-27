//! Styrene DX — Dioxus desktop app for mesh chat and fleet management.
//!
//! Uses an explicit Live, Embedded, or Fixture runtime profile for graphical
//! peer management, messaging, Micron pages, and mesh status visualization.
//!
//! Run: `cargo run -p styrene-dx`

use std::sync::Arc;

use dioxus::prelude::*;
use futures_util::StreamExt;

mod backend;
mod components;
mod daemon_bridge;
mod safety;
mod scenario;
mod scenario_process;
mod state;
mod stores;

use scenario::ScenarioBackend;

fn main() {
    // Initialize tracing — RUST_LOG=dx=debug for bridge diagnostics
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("dx=info,styrene=info")),
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
    let initial_profile = backend::RuntimeProfile::from_environment().unwrap_or_else(|error| {
        tracing::warn!(target: "dx::session", %error, "invalid environment profile; showing explicit Live configuration");
        backend::RuntimeProfile::Live { socket_path: styrene_ipc_server::default_socket_path() }
    });
    let mut selected_profile = use_signal(|| initial_profile.clone());
    let profile_label = selected_profile.read().label().to_string();
    let system_profile = Some(selected_profile.read().clone());
    let mut profile_kind = use_signal(|| initial_profile.label().to_ascii_lowercase());
    let mut socket_path = use_signal(|| match &initial_profile {
        backend::RuntimeProfile::Live { socket_path } => socket_path.display().to_string(),
        _ => styrene_ipc_server::default_socket_path().display().to_string(),
    });
    let mut fixture_id = use_signal(|| match &initial_profile {
        backend::RuntimeProfile::Fixture { fixture } => fixture_name(*fixture).to_string(),
        _ => "healthy".to_string(),
    });
    let mut profile_error = use_signal(|| None::<String>);
    let mut stores = use_signal(stores::DomainStores::default);
    let mut selected_peer = use_signal(|| None::<String>);
    let mut active_route = use_signal(state::AppRoute::default);
    let mut activity_open = use_signal(|| false);
    let mut activity_export = use_signal(|| None::<String>);
    let scenario_backend = use_hook(|| {
        Arc::new(scenario::FixtureScenarioBackend::default()) as Arc<dyn ScenarioBackend>
    });

    // Bridge handle — shared with UI for RPC calls (send_chat, browse_page, etc.)
    let mut session: Signal<Option<Arc<dyn backend::BackendSession>>> = use_signal(|| None);

    // Command channel — UI sends commands, spawned task processes them
    let mut cmd_tx: Signal<Option<tokio::sync::mpsc::Sender<daemon_bridge::DaemonCommand>>> =
        use_signal(|| None);

    // Own exactly one backend. Profile requests are handled before a replacement opens.
    let profile_session =
        use_coroutine(move |mut profile_rx: UnboundedReceiver<backend::RuntimeProfile>| {
            let mut requested_profile = initial_profile.clone();
            async move {
                loop {
                    let generation = backend::ConnectionGeneration::next();
                    stores.write().begin_session(requested_profile.label(), generation);
                    match backend::open_session(requested_profile.clone(), generation).await {
                        Ok(opened) => {
                            tracing::info!(
                                target: "dx::session",
                                generation = opened.generation.0,
                                profile = opened.backend.profile().label(),
                                "backend session opened"
                            );
                            let br = opened.backend;
                            let mut event_rx = opened.events;
                            session.set(Some(br.clone()));

                            // Spawn command handler task (on Dioxus runtime so signals are accessible)
                            let (tx, mut cmd_rx) = tokio::sync::mpsc::channel(64);
                            let tx_init = tx.clone();
                            cmd_tx.set(Some(tx));

                            let cmd_bridge = br.clone();
                            let command_task = spawn(async move {
                                while let Some(cmd) = cmd_rx.recv().await {
                                    handle_ui_command(
                                        cmd,
                                        &cmd_bridge,
                                        &mut stores,
                                        opened.generation,
                                    )
                                    .await;
                                }
                            });

                            // Initial data fetch
                            let _ =
                                tx_init.try_send(daemon_bridge::DaemonCommand::RefreshPathTable);
                            let _ =
                                tx_init.try_send(daemon_bridge::DaemonCommand::RefreshInterfaces);
                            let _ = tx_init.try_send(daemon_bridge::DaemonCommand::RefreshLinks);
                            let _ =
                                tx_init.try_send(daemon_bridge::DaemonCommand::RefreshOperations);
                            let _ = tx_init.try_send(daemon_bridge::DaemonCommand::RefreshRequests);
                            let _ =
                                tx_init.try_send(daemon_bridge::DaemonCommand::RefreshResources);
                            let _ = tx_init.try_send(
                                daemon_bridge::DaemonCommand::RefreshPropagation { cursor: None },
                            );
                            let _ =
                                tx_init.try_send(daemon_bridge::DaemonCommand::LoadConversations {
                                    cursor: None,
                                });

                            // Process daemon events until a replacement profile is requested.
                            let next_profile = loop {
                                let ev = tokio::select! {
                                    profile = profile_rx.next() => break profile,
                                    event = event_rx.recv() => event,
                                };
                                let Some(ev) = ev else {
                                    break profile_rx.next().await;
                                };
                                let sparse_message_id = match &ev {
                                    daemon_bridge::DaemonEvent::MessageReceived(message)
                                        if !message.projection_complete =>
                                    {
                                        Some(message.id.clone())
                                    }
                                    _ => None,
                                };
                                let refresh_paths =
                                    matches!(&ev, daemon_bridge::DaemonEvent::RouteLifecycle(_));
                                let reconcile_requests = matches!(
                                    &ev,
                                    daemon_bridge::DaemonEvent::ReconcileRequests { .. }
                                );
                                let reconcile_all = matches!(
                                    &ev,
                                    daemon_bridge::DaemonEvent::ReconcileRequired { .. }
                                );
                                let requery_standard = matches!(
                                    &ev,
                                    daemon_bridge::DaemonEvent::StandardPropagationChanged { .. }
                                );
                                let status_negotiated =
                                    matches!(&ev, daemon_bridge::DaemonEvent::Status(_));
                                let applied =
                                    stores.write().apply_daemon_event(opened.generation, ev);
                                if let Some(message_id) = sparse_message_id {
                                    let _ = tx_init.try_send(
                                        daemon_bridge::DaemonCommand::QueryMessage { message_id },
                                    );
                                }
                                if !applied {
                                    continue;
                                }
                                if refresh_paths {
                                    let _ = tx_init
                                        .try_send(daemon_bridge::DaemonCommand::RefreshPathTable);
                                }
                                if requery_standard {
                                    let _ = tx_init.try_send(
                                        daemon_bridge::DaemonCommand::RefreshStandardPropagation,
                                    );
                                }
                                if status_negotiated {
                                    let _ = tx_init.try_send(
                                        daemon_bridge::DaemonCommand::RefreshStandardPropagation,
                                    );
                                }
                                if reconcile_requests {
                                    let _ = tx_init
                                        .try_send(daemon_bridge::DaemonCommand::RefreshRequests);
                                }
                                if reconcile_all {
                                    let loaded_peers = stores.read().loaded_message_peers();
                                    for command in [
                                        daemon_bridge::DaemonCommand::RefreshPathTable,
                                        daemon_bridge::DaemonCommand::RefreshInterfaces,
                                        daemon_bridge::DaemonCommand::RefreshLinks,
                                        daemon_bridge::DaemonCommand::RefreshOperations,
                                        daemon_bridge::DaemonCommand::RefreshRequests,
                                        daemon_bridge::DaemonCommand::RefreshResources,
                                        daemon_bridge::DaemonCommand::RefreshStandardPropagation,
                                    ] {
                                        let _ = tx_init.try_send(command);
                                    }
                                    if tx_init
                                        .send(daemon_bridge::DaemonCommand::LoadConversations {
                                            cursor: None,
                                        })
                                        .await
                                        .is_err()
                                    {
                                        break profile_rx.next().await;
                                    }
                                    for peer_hash in loaded_peers {
                                        if tx_init
                                            .send(daemon_bridge::DaemonCommand::LoadMessages {
                                                peer_hash,
                                                cursor: None,
                                            })
                                            .await
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }
                                }
                            };
                            command_task.cancel();
                            cmd_tx.set(None);
                            session.set(None);
                            br.shutdown().await;
                            let Some(profile) = next_profile else {
                                return;
                            };
                            selected_profile.set(profile.clone());
                            requested_profile = profile;
                        }
                        Err(e) => {
                            stores.write().fail_session(generation, e);
                            let Some(profile) = profile_rx.next().await else {
                                return;
                            };
                            selected_profile.set(profile.clone());
                            requested_profile = profile;
                        }
                    }
                }
            }
        });

    let view = stores.read().clone();
    let id_display = view
        .identity
        .current
        .as_ref()
        .map(|id| {
            let name = if id.display_name.is_empty() { "unnamed" } else { &id.display_name };
            let hash_short = &id.destination_hash[..12.min(id.destination_hash.len())];
            format!("{name} ({hash_short}...)")
        })
        .unwrap_or_else(|| "loading...".into());

    let local_hash =
        view.identity.current.as_ref().map(|id| id.destination_hash.clone()).unwrap_or_default();
    let local_name = view
        .identity
        .current
        .as_ref()
        .and_then(|id| (!id.display_name.is_empty()).then(|| id.display_name.clone()));
    let capabilities = backend::BackendCapabilities::negotiated(
        view.runtime.connected,
        view.runtime.server_generation,
        view.runtime.capabilities.as_ref(),
    );
    let safety_profile = system_profile.clone();
    let safety = use_memo(move || {
        let current = stores.read();
        safety::SafetyContext::new(
            safety_profile.as_ref(),
            current.runtime.connected,
            current.runtime.generation.0,
            current.runtime.server_generation,
            current.runtime.capabilities.as_ref(),
        )
    });
    let diagnostics =
        session.read().as_ref().map(|backend| backend.diagnostics()).unwrap_or_default();
    let route_state = view.route_state(&active_route.read(), capabilities);
    let command = view.command_summary();
    let mut chat_input = use_signal(String::new);
    let mut chat_title = use_signal(String::new);
    let mut chat_method = use_signal(|| "direct".to_string());
    let mut chat_attachments = use_signal(Vec::<styrene_ipc::types::AttachmentInput>::new);
    let selected_draft = selected_peer
        .read()
        .as_ref()
        .and_then(|peer| view.messages.drafts.get(peer))
        .map(|draft| (draft.peer_hash.clone(), draft.content.clone(), draft.updated_at));
    use_effect(move || {
        if let Some((peer, content, _)) = &selected_draft {
            if selected_peer.read().as_deref() == Some(peer.as_str())
                && chat_input.read().is_empty()
            {
                chat_input.set(content.clone());
            }
        }
    });
    let accepted_compose = view.messages.accepted_compose.clone();
    use_effect(move || {
        if let Some((peer, content)) = &accepted_compose {
            if selected_peer.read().as_deref() == Some(peer.as_str())
                && chat_input.read().as_str() == content
            {
                chat_input.set(String::new());
                chat_title.set(String::new());
                chat_attachments.set(Vec::new());
                stores.write().messages.accepted_compose = None;
            }
        }
    });
    let chat_unavailable = view.delivery_method_availability(&chat_method.read()).err();
    let chat_send_reason = chat_unavailable.clone().or_else(|| {
        let content = chat_input.read();
        if content.is_empty() {
            Some("Enter a message before sending".into())
        } else if content.len() > styrene_ipc::types::MAX_CHAT_CONTENT_BYTES {
            Some("Message exceeds 65536 UTF-8 bytes".into())
        } else {
            None
        }
    });
    let page_unavailable = view.mutation_availability("page.browse").err();

    // Helper to send commands to the daemon
    let send_cmd = move |cmd: daemon_bridge::DaemonCommand| {
        if let Some(ref tx) = *cmd_tx.read() {
            if tx.try_send(cmd).is_err() {
                tracing::warn!(target: "dx::command", "UI command queue is full");
            }
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
                    class: if view.runtime.connected { "badge connected" } else { "badge disconnected" },
                    "{profile_label}: {view.runtime.connection_mode}"
                }
                div { class: "profile-config", aria_label: "Runtime profile configuration",
                    select {
                        aria_label: "Runtime profile",
                        value: "{profile_kind}",
                        onchange: move |event| profile_kind.set(event.value()),
                        option { value: "live", "Live" }
                        option { value: "embedded", "Embedded" }
                        option { value: "fixture", "Fixture" }
                    }
                    if profile_kind.read().as_str() == "live" {
                        input {
                            aria_label: "Live daemon socket",
                            value: "{socket_path}",
                            oninput: move |event| socket_path.set(event.value()),
                        }
                    }
                    if profile_kind.read().as_str() == "fixture" {
                        select {
                            aria_label: "Fixture state",
                            value: "{fixture_id}",
                            onchange: move |event| fixture_id.set(event.value()),
                            option { value: "empty", "Empty" }
                            option { value: "healthy", "Healthy" }
                            option { value: "degraded", "Degraded" }
                            option { value: "error", "Error" }
                            option { value: "high-cardinality", "High cardinality" }
                            option { value: "active-scenario", "Active scenario" }
                        }
                    }
                    button {
                        onclick: move |_| {
                            let profile = match profile_kind.read().as_str() {
                                "live" => backend::RuntimeProfile::live(socket_path.read().as_str()),
                                "embedded" => Ok(backend::RuntimeProfile::embedded()),
                                "fixture" => fixture_from_name(fixture_id.read().as_str())
                                    .map(backend::RuntimeProfile::fixture),
                                _ => Err("unknown runtime profile".into()),
                            };
                            match profile {
                                Ok(profile) => {
                                    profile_error.set(None);
                                    profile_session.send(profile);
                                }
                                Err(error) => profile_error.set(Some(error)),
                            }
                        },
                        "Activate"
                    }
                }
                if let Some(error) = profile_error.read().as_ref() {
                    span { class: "profile-error", role: "alert", "{error}" }
                }
                if !view.runtime.connected {
                    span { class: "alert-count", "1 alert" }
                }
                button {
                    class: "activity-toggle",
                    onclick: move |_| activity_open.toggle(),
                    "Activity {view.activity.entries.len()}"
                }
            }

            // Tab bar
            nav { class: "tab-bar", aria_label: "Primary navigation",
                button {
                    class: if *active_route.read() == state::AppRoute::Command { "tab active" } else { "tab" },
                    aria_current: (*active_route.read() == state::AppRoute::Command).then_some("page"),
                    onclick: move |_| active_route.set(state::AppRoute::Command),
                    "Command"
                }
                button {
                    class: if *active_route.read() == state::AppRoute::Network { "tab active" } else { "tab" },
                    aria_current: (*active_route.read() == state::AppRoute::Network).then_some("page"),
                    onclick: move |_| active_route.set(state::AppRoute::Network),
                    "Network"
                }
                button {
                    class: if *active_route.read() == state::AppRoute::Messages { "tab active" } else { "tab" },
                    aria_current: (*active_route.read() == state::AppRoute::Messages).then_some("page"),
                    onclick: move |_| active_route.set(state::AppRoute::Messages),
                    "Messages"
                }
                button {
                    class: if *active_route.read() == state::AppRoute::Fleet { "tab active" } else { "tab" },
                    aria_current: (*active_route.read() == state::AppRoute::Fleet).then_some("page"),
                    onclick: move |_| active_route.set(state::AppRoute::Fleet),
                    "Fleet"
                }
                button {
                    class: if *active_route.read() == state::AppRoute::Propagation { "tab active" } else { "tab" },
                    aria_current: (*active_route.read() == state::AppRoute::Propagation).then_some("page"),
                    onclick: move |_| active_route.set(state::AppRoute::Propagation),
                    "Propagation"
                }
                button {
                    class: if *active_route.read() == state::AppRoute::Content { "tab active" } else { "tab" },
                    aria_current: (*active_route.read() == state::AppRoute::Content).then_some("page"),
                    onclick: move |_| active_route.set(state::AppRoute::Content),
                    "Content"
                }
                button {
                    class: if *active_route.read() == state::AppRoute::Lab { "tab active" } else { "tab" },
                    aria_current: (*active_route.read() == state::AppRoute::Lab).then_some("page"),
                    onclick: move |_| active_route.set(state::AppRoute::Lab),
                    "Lab"
                }
                button {
                    class: if *active_route.read() == state::AppRoute::System { "tab active" } else { "tab" },
                    aria_current: (*active_route.read() == state::AppRoute::System).then_some("page"),
                    onclick: move |_| active_route.set(state::AppRoute::System),
                    "System"
                }
            }

            match &route_state {
                stores::DataState::Loading => rsx! {
                    div { class: "state-banner loading", "Loading current {profile_label} state..." }
                },
                stores::DataState::Degraded { reason } => rsx! {
                    div { class: "state-banner degraded", "Degraded: {reason}" }
                },
                stores::DataState::Error { message } => rsx! {
                    div { class: "state-banner error", "Error: {message}" }
                },
                stores::DataState::Empty | stores::DataState::Ready => rsx! {},
            }

            // Body
            div { class: "body",
                match *active_route.read() {
                    state::AppRoute::Command => rsx! {
                        div { class: "main command-page",
                            div { class: "empty-state",
                                h2 { "Command" }
                                p { "Operational summary for the active {profile_label} session." }
                            }
                            div { class: "command-grid",
                                div { class: "command-card",
                                    h3 { "Transport" }
                                    strong { if command.transport_active { "Active" } else { "Inactive" } }
                                    p { "{command.interface_count} interfaces" }
                                }
                                div { class: "command-card",
                                    h3 { "Network" }
                                    strong { "{command.observed_peers} observed peers" }
                                    p { "{command.route_count} current routes" }
                                }
                                div { class: "command-card",
                                    h3 { "Links" }
                                    strong { "{command.active_links} active" }
                                    p { "{command.link_records} lifecycle records" }
                                }
                                div { class: "command-card",
                                    h3 { "Propagation" }
                                    strong { if command.propagation_enabled { "Enabled" } else { "Disabled" } }
                                    p { "Open Propagation for queue and peer state." }
                                }
                            }
                        }
                    },
                    state::AppRoute::Network => rsx! {
                        components::NetworkPage {
                            peers: view.network.peers.clone(),
                            paths: view.network.paths.clone(),
                            status: view.network.status.clone(),
                            local_hash: local_hash.clone(),
                            local_name: local_name.clone(),
                            on_select_peer: move |hash: String| {
                                selected_peer.set(Some(hash.clone()));
                                send_cmd(daemon_bridge::DaemonCommand::LoadMessages {
                                    peer_hash: hash,
                                    cursor: None,
                                });
                                active_route.set(state::AppRoute::Messages);
                            },
                            links: view.network.links.clone(),
                            interfaces: view.network.interfaces.clone(),
                            operations: view.network.operations.clone(),
                            requests: view.network.requests.clone(),
                            resources: view.network.resources.clone(),
                            safety,
                            on_network_command: send_cmd,
                            on_browse_page: move |host_hash: String| {
                                match state::PageAddress::remote_index(&host_hash) {
                                    Ok(address) => {
                                        send_cmd(daemon_bridge::DaemonCommand::BrowsePage { address });
                                        active_route.set(state::AppRoute::Content);
                                    }
                                    Err(error) => tracing::warn!(target: "dx::content", %error, %host_hash, "invalid page host"),
                                }
                            },
                        }
                    },

                    state::AppRoute::Messages => rsx! {
                        // Sidebar — active conversations only
                        div { class: "sidebar",
                            div { class: "sidebar-header", "Conversations" }
                            if view.messages.conversations.is_empty() {
                                div { class: "sidebar-empty",
                                    "No conversations yet. Select a peer from the Network tab to start one."
                                }
                            }
                            for convo in view.messages.conversations.iter() {
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
                                    let draft_content = view.messages.drafts.get(&hash)
                                        .map(|draft| draft.content.clone());
                                    rsx! {
                                        button {
                                            class: if is_selected { "convo-item selected" } else { "convo-item" },
                                            aria_current: is_selected.then_some("page"),
                                            onclick: move |_| {
                                                if let Some(previous) = selected_peer.read().clone() {
                                                    if previous != hash && !chat_input.read().is_empty() {
                                                        send_cmd(daemon_bridge::DaemonCommand::SaveDraft {
                                                            peer_hash: previous,
                                                            content: chat_input.read().clone(),
                                                        });
                                                    }
                                                }
                                                selected_peer.set(Some(hash.clone()));
                                                chat_input.set(draft_content.clone().unwrap_or_default());
                                                send_cmd(daemon_bridge::DaemonCommand::LoadDraft {
                                                    peer_hash: hash.clone(),
                                                });
                                                send_cmd(daemon_bridge::DaemonCommand::LoadMessages {
                                                    peer_hash: load_hash.clone(),
                                                    cursor: None,
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
                            if let Some(cursor) = &view.messages.conversation_cursor {
                                button {
                                    class: "load-older",
                                    onclick: {
                                        let cursor = cursor.clone();
                                        move |_| send_cmd(daemon_bridge::DaemonCommand::LoadConversations {
                                            cursor: Some(cursor.clone()),
                                        })
                                    },
                                    "Load Older"
                                }
                            }
                        }

                        // Main content — chat
                        div { class: "main",
                            if let Some(ref peer_hash) = *selected_peer.read() {
                                {
                                    let peer_name = view.network.peers.iter()
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
                                                for msg in view.messages.messages.iter().filter(|m|
                                                    m.source == *peer_hash || m.destination == *peer_hash
                                                ) {
                                                    components::MessageBubble { message: msg.clone() }
                                                    if msg.is_outgoing {
                                                        div { class: "message-actions",
                                                            button {
                                                                disabled: view.mutation_availability("messaging.lifecycle").is_err(),
                                                                onclick: {
                                                                    let message_id = msg.id.clone();
                                                                    move |_| send_cmd(daemon_bridge::DaemonCommand::RetryMessage {
                                                                        message_id: message_id.clone(),
                                                                    })
                                                                },
                                                                "Retry"
                                                            }
                                                            button {
                                                                disabled: view.mutation_availability("messaging.lifecycle").is_err(),
                                                                onclick: {
                                                                    let message_id = msg.id.clone();
                                                                    move |_| send_cmd(daemon_bridge::DaemonCommand::CancelMessage {
                                                                        message_id: message_id.clone(),
                                                                    })
                                                                },
                                                                "Cancel"
                                                            }
                                                        }
                                                    }
                                                }
                                                if let Some(cursor) = view.messages.message_cursors.get(peer_hash) {
                                                    button {
                                                        class: "load-older",
                                                        onclick: {
                                                            let peer = peer_hash.clone();
                                                            let cursor = cursor.clone();
                                                            move |_| send_cmd(daemon_bridge::DaemonCommand::LoadMessages {
                                                                peer_hash: peer.clone(), cursor: Some(cursor.clone()),
                                                            })
                                                        },
                                                        "Load Older"
                                                    }
                                                }
                                            }
                                            // Chat input
                                            div { class: "chat-input-bar",
                                                if let Some(reason) = &chat_send_reason {
                                                    p { id: "chat-send-disabled-reason", class: "control-disabled-reason", "Chat controls disabled: {reason}" }
                                                }
                                                input {
                                                    class: "chat-input",
                                                    aria_label: "Message title",
                                                    r#type: "text",
                                                    placeholder: "Optional title",
                                                    value: "{chat_title}",
                                                    oninput: move |evt| chat_title.set(evt.value()),
                                                }
                                                select {
                                                    aria_label: "Delivery method",
                                                    value: "{chat_method}",
                                                    onchange: move |evt| chat_method.set(evt.value()),
                                                    option { value: "direct", "Direct" }
                                                    option { value: "opportunistic", "Opportunistic" }
                                                    option { value: "propagated", "Propagated" }
                                                    option { value: "paper", "Paper" }
                                                }
                                                input {
                                                    class: "chat-input",
                                                    aria_label: "Message content",
                                                    disabled: chat_unavailable.is_some(),
                                                    title: chat_unavailable.as_deref().unwrap_or("Message peer"),
                                                    aria_describedby: chat_unavailable.as_ref().map(|_| "chat-send-disabled-reason"),
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
                                                                        title: (!chat_title.read().is_empty()).then(|| chat_title.read().clone()),
                                                                        delivery_method: chat_method.read().clone(),
                                                                        attachments: chat_attachments.read().clone(),
                                                                    });
                                                                }
                                                            }
                                                        }
                                                    },
                                                }
                                                button {
                                                    class: "chat-send-btn",
                                                    disabled: chat_send_reason.is_some(),
                                                    title: chat_send_reason.as_deref().unwrap_or("Send message"),
                                                    aria_describedby: chat_send_reason.as_ref().map(|_| "chat-send-disabled-reason"),
                                                    onclick: {
                                                        let ph3 = ph.clone();
                                                        move |_| {
                                                            let content = chat_input.read().clone();
                                                            if !content.trim().is_empty() {
                                                                send_cmd(daemon_bridge::DaemonCommand::SendChat {
                                                                    peer_hash: ph3.clone(),
                                                                    content,
                                                                    title: (!chat_title.read().is_empty()).then(|| chat_title.read().clone()),
                                                                    delivery_method: chat_method.read().clone(),
                                                                    attachments: chat_attachments.read().clone(),
                                                                });
                                                            }
                                                        }
                                                    },
                                                    "Send"
                                                }
                                                button {
                                                    disabled: view.mutation_availability("messaging.manage").is_err(),
                                                    onclick: {
                                                        let peer = ph.clone();
                                                        move |_| {
                                                            send_cmd(daemon_bridge::DaemonCommand::DiscardDraft { peer_hash: peer.clone() });
                                                            chat_input.set(String::new());
                                                        }
                                                    },
                                                    "Discard Draft"
                                                }
                                                input {
                                                    r#type: "file",
                                                    aria_label: "Message attachments",
                                                    multiple: true,
                                                    onchange: move |event| {
                                                        let files = event.files();
                                                        spawn(async move {
                                                            let mut selected = Vec::new();
                                                            for file in files.into_iter().take(styrene_ipc::types::MAX_CHAT_ATTACHMENTS) {
                                                                if file.size() as usize > styrene_ipc::types::MAX_CHAT_ATTACHMENT_BYTES
                                                                    || file.name().len() > styrene_ipc::types::MAX_CHAT_ATTACHMENT_NAME_BYTES
                                                                {
                                                                    tracing::warn!(target: "dx::chat", name = %file.name(), "attachment exceeds compose limits");
                                                                    continue;
                                                                }
                                                                match file.read_bytes().await {
                                                                    Ok(bytes) => {
                                                                        let mut attachment = styrene_ipc::types::AttachmentInput::default();
                                                                        attachment.name = file.name();
                                                                        attachment.content_type = file.content_type();
                                                                        attachment.bytes = bytes.to_vec();
                                                                        selected.push(attachment);
                                                                    }
                                                                    Err(error) => tracing::warn!(target: "dx::chat", %error, "attachment read failed"),
                                                                }
                                                            }
                                                            chat_attachments.set(selected);
                                                        });
                                                    },
                                                }
                                                for attachment in chat_attachments.read().iter() {
                                                    span { class: "compose-note", "{attachment.name} ({attachment.bytes.len()} bytes)" }
                                                }
                                                span { class: "compose-note", "Attachments are not retained in drafts." }
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

                    state::AppRoute::Fleet => rsx! {
                        components::FleetPage {
                            peers: view.network.peers.clone(),
                            managed_peers: view.fleet.managed_peers.clone(),
                            jobs: view.fleet.jobs.clone(),
                            safety,
                            on_command: send_cmd,
                        }
                    },
                    state::AppRoute::Propagation => rsx! {
                        components::PropagationPage {
                            snapshot: view.propagation.snapshot.clone(),
                            standard_snapshot: view.propagation.standard_snapshot.clone(),
                            state: view.propagation.state.clone(),
                            standard_state: view.propagation.standard_state.clone(),
                            available: capabilities.propagation,
                            on_refresh: move |cursor| {
                                send_cmd(daemon_bridge::DaemonCommand::RefreshPropagation { cursor });
                                send_cmd(daemon_bridge::DaemonCommand::RefreshStandardPropagation);
                            },
                        }
                    },
                    state::AppRoute::Content => rsx! {
                        // Page host sidebar
                        div { class: "sidebar",
                            div { class: "sidebar-header", "Page Hosts" }
                            if let Some(reason) = &page_unavailable {
                                p { id: "page-host-controls-disabled", class: "control-disabled-reason", "Page host controls disabled: {reason}" }
                            }
                            button {
                                class: "peer-item",
                                disabled: !capabilities.content,
                                aria_describedby: (!capabilities.content).then_some("page-host-controls-disabled"),
                                onclick: move |_| {
                                    send_cmd(daemon_bridge::DaemonCommand::BrowsePage {
                                        address: state::PageAddress::local_index(),
                                    });
                                },
                                span { class: "peer-icon", style: "color: var(--accent);", "●" }
                                span { class: "peer-name", "Local Node" }
                            }
                            for entry in &view.content.local_inventory {
                                {
                                    let path = entry.path.clone();
                                    let is_page = entry.kind == "page";
                                    let active = entry.handler_active;
                                    let detail = format!(
                                        "{:?}{}{}",
                                        entry.kind,
                                        if entry.dynamic { " dynamic" } else { "" },
                                        if entry.restricted { " restricted" } else { "" },
                                    );
                                    rsx! {
                                        button {
                                            class: "peer-item",
                                            disabled: !capabilities.content || !is_page || !active,
                                            onclick: move |_| {
                                                if let Ok(address) = state::PageAddress::parse(&path) {
                                                    send_cmd(daemon_bridge::DaemonCommand::BrowsePage { address });
                                                }
                                            },
                                            span { class: "peer-icon", if active { "●" } else { "○" } }
                                            span { class: "peer-name", "{path}" }
                                            small { "{detail}" }
                                        }
                                    }
                                }
                            }
                            for peer in view.network.peers.iter().filter(|p| p.node_role == state::PeerRole::PageHost) {
                                {
                                    let hash = peer.hash.clone();
                                    let name = peer.name.clone().unwrap_or_else(|| hash[..8.min(hash.len())].to_string());
                                    rsx! {
                                        button {
                                            class: "peer-item",
                                            disabled: !capabilities.content,
                                            aria_describedby: (!capabilities.content).then_some("page-host-controls-disabled"),
                                            onclick: move |_| {
                                                match state::PageAddress::remote_index(&hash) {
                                                    Ok(address) => send_cmd(daemon_bridge::DaemonCommand::BrowsePage { address }),
                                                    Err(error) => tracing::warn!(target: "dx::content", %error, %hash, "invalid page host"),
                                                }
                                            },
                                            span { class: "peer-icon", style: "color: var(--green);", "●" }
                                            span { class: "peer-name", "{name}" }
                                            small { "native announce: " {peer.last_announce.map_or_else(|| "time unreported".into(), |value| value.to_string())} }
                                        }
                                    }
                                }
                            }
                        }

                        components::PageBrowser {
                            page: view.content.page.clone(),
                            download: view.content.download.clone(),
                            available: capabilities.content,
                            unavailable_reason: page_unavailable.clone(),
                            on_navigate: move |request| {
                                send_cmd(daemon_bridge::DaemonCommand::NavigatePage(request));
                            },
                            on_close: move |session_id| send_cmd(daemon_bridge::DaemonCommand::ClosePage { session_id }),
                            on_download: move |request| send_cmd(daemon_bridge::DaemonCommand::StartFileDownload(request)),
                            on_refresh_download: move |download_id| send_cmd(daemon_bridge::DaemonCommand::QueryFileDownload { download_id }),
                            on_cancel_download: move |download_id| send_cmd(daemon_bridge::DaemonCommand::CancelFileDownload { download_id }),
                            on_save_download: move |(download_id, destination)| send_cmd(daemon_bridge::DaemonCommand::SaveFileDownload { download_id, destination }),
                        }
                    },
                    state::AppRoute::Lab => rsx! {
                        components::LabPage {
                            profile_label: profile_label.clone(),
                            fixture_available: capabilities.scenarios,
                            live_available: scenario_backend.availability("direct").is_ok(),
                            safety,
                            definitions: scenario_backend.catalog().to_vec(),
                            run: view.scenario.run.clone(),
                            on_cancel: {
                                let backend = scenario_backend.clone();
                                move |run_id: String| {
                                    let backend = backend.clone();
                                    spawn(async move {
                                        match backend.cancel(&run_id).await {
                                            Ok(run) => stores.write().update_scenario_run(run),
                                            Err(error) => tracing::warn!(target: "dx::lab", %error, "scenario cancellation failed"),
                                        }
                                    });
                                }
                            },
                            on_export: {
                                let backend = scenario_backend.clone();
                                move |run_id: String| {
                                    let backend = backend.clone();
                                    spawn(async move {
                                        match backend.export(&run_id).await {
                                            Ok(run) => stores.write().update_scenario_run(run),
                                            Err(error) => tracing::warn!(target: "dx::lab", %error, "scenario evidence export failed"),
                                        }
                                    });
                                }
                            },
                            on_start: {
                                let backend = scenario_backend.clone();
                                move |scenario_id: String| {
                                    let backend = backend.clone();
                                    spawn(async move {
                                        match backend.start(&scenario_id).await {
                                            Ok(run) => {
                                                let run_id = run.run_id.clone();
                                                stores.write().set_scenario_run(run);
                                                let backend = backend.clone();
                                                spawn(async move {
                                                    match backend.wait(&run_id).await {
                                                        Ok(run) => stores.write().update_scenario_run(run),
                                                        Err(error) => tracing::warn!(target: "dx::lab", %error, "scenario runner did not publish a terminal result"),
                                                    }
                                                });
                                            }
                                            Err(error) => tracing::warn!(target: "dx::lab", %error, "scenario start failed"),
                                        }
                                    });
                                }
                            },
                        }
                    },
                    state::AppRoute::System => rsx! {
                        components::SystemPage {
                            profile: system_profile.clone(),
                            connected: view.runtime.connected,
                            connection_mode: view.runtime.connection_mode.clone(),
                            client_generation: view.runtime.generation.0,
                            server_generation: view.runtime.server_generation,
                            event_generation: view.runtime.event_server_generation,
                            identity: view.identity.current.clone(),
                            interfaces: view.network.interfaces.clone(),
                            capabilities: view.runtime.capabilities.clone(),
                            status: view.network.status.clone(),
                            propagation_queue: view.propagation.standard_snapshot.as_ref().map(|snapshot| (snapshot.queue.queued_count, snapshot.queue.queued_bytes)),
                            diagnostics,
                        }
                    },
                }
            }
            if *activity_open.read() {
                aside { class: "activity-drawer", aria_label: "Activity timeline",
                    div { class: "activity-drawer-header",
                        h2 { "Activity" }
                        button {
                            onclick: move |_| {
                                match stores.read().export_activity(diagnostics) {
                                    Ok(export) => activity_export.set(Some(export)),
                                    Err(error) => tracing::warn!(target: "dx::activity", %error, "activity export failed"),
                                }
                            },
                            "Export diagnostics"
                        }
                        button {
                            aria_label: "Close activity",
                            onclick: move |_| activity_open.set(false),
                            "Close"
                        }
                    }
                    div { class: "context-summary",
                        span { "Context" }
                        strong {
                            {selected_peer.read().as_deref().unwrap_or("No entity selected")}
                        }
                    }
                    if view.activity.entries.is_empty() {
                        p { class: "activity-empty", "No runtime activity recorded." }
                    }
                    for entry in view.activity.entries.iter().rev() {
                        div {
                            class: match entry.severity {
                                state::ActivitySeverity::Info => "activity-entry info",
                                state::ActivitySeverity::Warning => "activity-entry warning",
                                state::ActivitySeverity::Error => "activity-entry error",
                            },
                            div { class: "activity-meta",
                                span { "{entry.kind}" }
                                time { "{format_timestamp(entry.timestamp)}" }
                            }
                            p { "{entry.summary}" }
                            if let Some(entity) = &entry.entity {
                                code { "{entity}" }
                            }
                            span { class: "activity-provenance", "Source: {entry.provenance}" }
                            if let Some(correlation) = &entry.correlation_id {
                                code { "Correlation: {correlation}" }
                            }
                        }
                    }
                    if let Some(export) = activity_export.read().as_ref() {
                        textarea {
                            class: "activity-export",
                            aria_label: "Redacted activity diagnostics export",
                            readonly: true,
                            rows: 12,
                            value: "{export}",
                        }
                    }
                }
            }
            if let Some(export) = &view.messages.paper_export {
                div { class: "modal-backdrop",
                    div {
                        class: "modal-card paper-export-modal",
                        role: "dialog",
                        aria_modal: "true",
                        aria_labelledby: "paper-export-title",
                        onkeydown: move |event: KeyboardEvent| if event.key() == Key::Escape {
                            stores.write().messages.paper_export = None;
                        },
                        h2 { id: "paper-export-title", "Paper LXMF Export" }
                        p { "Select and copy this response-only URI. It is not stored in history." }
                        textarea {
                            readonly: true,
                            rows: 8,
                            value: "{export.uri}",
                        }
                        button {
                            autofocus: true,
                            onclick: move |_| stores.write().messages.paper_export = None,
                            "Dismiss"
                        }
                    }
                }
            }
        }
    }
}

// ── Event & command handlers ──────────────────────────────────────────────

async fn apply_lifecycle_outcome(
    bridge: &Arc<dyn backend::BackendSession>,
    stores: &mut Signal<stores::DomainStores>,
    generation: backend::ConnectionGeneration,
    outcome: styrene_ipc::types::MessagingOperationOutcome,
) {
    use styrene_ipc::types::MessagingDisposition;

    match outcome.disposition {
        MessagingDisposition::Applied
        | MessagingDisposition::Created
        | MessagingDisposition::Updated
        | MessagingDisposition::AlreadyCancelled => {}
        MessagingDisposition::Unchanged if outcome.message.is_none() => {
            tracing::warn!(
                target: "dx::chat",
                correlated_id = outcome.correlated_id.as_deref().unwrap_or("missing"),
                "unchanged lifecycle outcome omitted authoritative projection"
            );
        }
        MessagingDisposition::Unchanged => {}
        MessagingDisposition::TerminalConflict => tracing::warn!(
            target: "dx::chat",
            terminal_state = outcome.terminal_state.as_deref().unwrap_or("unknown"),
            "message lifecycle is already terminal"
        ),
        MessagingDisposition::NotFound => {}
        MessagingDisposition::Unknown => {
            tracing::warn!(target: "dx::chat", "unknown messaging lifecycle disposition")
        }
        _ => tracing::warn!(target: "dx::chat", "unsupported messaging lifecycle disposition"),
    }

    let requery_peer = stores.write().apply_lifecycle_outcome(generation, outcome);
    if let Some(peer) = requery_peer {
        match bridge.message_page(&peer, None).await {
            Ok(page) => {
                let mut stores = stores.write();
                stores.reset_peer_message_snapshot(generation, &peer);
                stores.merge_peer_message_page(
                    generation,
                    &peer,
                    page.items.into_iter().map(state::ChatMessage::from).collect(),
                    page.next_cursor,
                );
            }
            Err(error) => {
                tracing::warn!(target: "dx::chat", %error, %peer, "message requery failed")
            }
        }
    }
}

async fn handle_ui_command(
    cmd: daemon_bridge::DaemonCommand,
    bridge: &Arc<dyn backend::BackendSession>,
    stores: &mut Signal<stores::DomainStores>,
    generation: backend::ConnectionGeneration,
) {
    if let Some(capability) = cmd.required_capability() {
        if let Err(error) = stores.read().mutation_availability_at(generation, capability) {
            tracing::warn!(target: "dx::command", %error, %capability, "backend action blocked");
            return;
        }
    }
    match cmd {
        daemon_bridge::DaemonCommand::SendChat {
            peer_hash,
            content,
            title,
            delivery_method,
            attachments,
        } => {
            if let Err(error) = stores.read().mutation_availability_at(generation, "chat.send") {
                tracing::warn!(target: "dx::chat", %error, "chat mutation blocked");
                return;
            }
            if let Ok(draft) = bridge.save_draft(&peer_hash, &content).await {
                stores.write().set_draft(generation, &peer_hash, Some(draft));
            }
            let submitted_content = content.clone();
            let mut request = styrene_ipc::types::SendChatRequest::default();
            request.peer_hash = peer_hash.clone();
            request.content = content;
            request.title = title;
            request.delivery_method = Some(delivery_method);
            request.attachments = attachments;
            match bridge.send_chat_outcome(request).await {
                Ok(outcome) => {
                    let accepted = stores.write().apply_send_outcome(
                        generation,
                        peer_hash.clone(),
                        submitted_content,
                        outcome,
                    );
                    if accepted {
                        let _ = bridge.discard_draft(&peer_hash).await;
                        let mut stores = stores.write();
                        stores.set_draft(generation, &peer_hash, None);
                    }
                }
                Err(e) => eprintln!("[dx] send_chat failed: {e}"),
            }
        }
        daemon_bridge::DaemonCommand::LoadDraft { peer_hash } => {
            match bridge.draft(&peer_hash).await {
                Ok(draft) => stores.write().set_draft(generation, &peer_hash, draft),
                Err(error) => tracing::warn!(target: "dx::chat", %error, "load draft failed"),
            }
        }
        daemon_bridge::DaemonCommand::SaveDraft { peer_hash, content } => {
            match bridge.save_draft(&peer_hash, &content).await {
                Ok(draft) => stores.write().set_draft(generation, &peer_hash, Some(draft)),
                Err(error) => tracing::warn!(target: "dx::chat", %error, "save draft failed"),
            }
        }
        daemon_bridge::DaemonCommand::DiscardDraft { peer_hash } => {
            match bridge.discard_draft(&peer_hash).await {
                Ok(()) => stores.write().set_draft(generation, &peer_hash, None),
                Err(error) => tracing::warn!(target: "dx::chat", %error, "discard draft failed"),
            }
        }
        daemon_bridge::DaemonCommand::RetryMessage { message_id } => {
            match bridge.retry_message(&message_id).await {
                Ok(outcome) => apply_lifecycle_outcome(bridge, stores, generation, outcome).await,
                Err(error) => tracing::warn!(target: "dx::chat", %error, "retry rejected"),
            }
        }
        daemon_bridge::DaemonCommand::CancelMessage { message_id } => {
            match bridge.cancel_message(&message_id).await {
                Ok(outcome) => apply_lifecycle_outcome(bridge, stores, generation, outcome).await,
                Err(error) => tracing::warn!(target: "dx::chat", %error, "cancel rejected"),
            }
        }
        daemon_bridge::DaemonCommand::BrowsePage { address } => {
            if let Err(error) = stores.read().mutation_availability_at(generation, "page.browse") {
                tracing::warn!(target: "dx::content", %error, "page browse blocked");
                return;
            }
            let (host, path) = address.parts();
            let host = host.to_string();
            let path = path.to_string();
            let started = std::time::Instant::now();
            stores
                .write()
                .set_page(generation, state::PageView::loading(host.clone(), path.clone()));
            match bridge.browse_page(&host, &path).await {
                Ok(response) => {
                    stores
                        .write()
                        .set_page(generation, state::PageView::from_daemon(response.page));
                }
                Err(error) => {
                    stores.write().set_page(
                        generation,
                        state::PageView::failed_at(
                            host,
                            path,
                            error,
                            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                        ),
                    );
                }
            }
        }
        daemon_bridge::DaemonCommand::NavigatePage(request) => {
            if let Err(error) = stores.read().mutation_availability_at(generation, "page.browse") {
                tracing::warn!(target: "dx::content", %error, "page navigation blocked");
                return;
            }
            let started = std::time::Instant::now();
            match bridge.navigate_page(request).await {
                Ok(response) => {
                    stores
                        .write()
                        .set_page(generation, state::PageView::from_daemon(response.page));
                }
                Err(error) => {
                    tracing::warn!(target: "dx::content", %error, elapsed_ms = started.elapsed().as_millis(), "page navigation failed")
                }
            }
        }
        daemon_bridge::DaemonCommand::ClosePage { session_id } => {
            if let Err(error) = stores.read().mutation_availability_at(generation, "page.browse") {
                tracing::warn!(target: "dx::content", %error, "page close blocked");
                return;
            }
            match bridge.close_page(&session_id).await {
                Ok(()) => {
                    stores.write().clear_page(generation);
                }
                Err(error) => tracing::warn!(target: "dx::content", %error, "page close failed"),
            }
        }
        daemon_bridge::DaemonCommand::StartFileDownload(request) => {
            if let Err(error) = stores.read().mutation_availability_at(generation, "page.browse") {
                tracing::warn!(target: "dx::content", %error, "download start blocked");
                return;
            }
            match bridge.start_file_download(request).await {
                Ok(download) => stores.write().set_download(generation, download),
                Err(error) => {
                    tracing::warn!(target: "dx::content", %error, "download start failed")
                }
            }
        }
        daemon_bridge::DaemonCommand::QueryFileDownload { download_id } => {
            match bridge.file_download(&download_id).await {
                Ok(download) => stores.write().set_download(generation, download),
                Err(error) => {
                    tracing::warn!(target: "dx::content", %error, "download refresh failed")
                }
            }
        }
        daemon_bridge::DaemonCommand::CancelFileDownload { download_id } => {
            if let Err(error) = stores.read().mutation_availability_at(generation, "page.browse") {
                tracing::warn!(target: "dx::content", %error, "download cancellation blocked");
                return;
            }
            match bridge.cancel_file_download(&download_id).await {
                Ok(download) => stores.write().set_download(generation, download),
                Err(error) => {
                    tracing::warn!(target: "dx::content", %error, "download cancellation failed")
                }
            }
        }
        daemon_bridge::DaemonCommand::SaveFileDownload { download_id, destination } => {
            if let Err(error) = stores.read().mutation_availability_at(generation, "page.browse") {
                tracing::warn!(target: "dx::content", %error, "download save blocked");
                return;
            }
            match bridge.save_file_download(&download_id, &destination).await {
                Ok(download) => stores.write().set_download(generation, download),
                Err(error) => tracing::warn!(target: "dx::content", %error, "download save failed"),
            }
        }
        daemon_bridge::DaemonCommand::RefreshPathTable => match bridge.path_table().await {
            Ok(entries) => stores.write().set_paths(generation, entries),
            Err(e) => eprintln!("[dx] path_table failed: {e}"),
        },
        daemon_bridge::DaemonCommand::RefreshInterfaces => match bridge.interface_stats().await {
            Ok(ifaces) => stores.write().set_interfaces(generation, ifaces),
            Err(e) => tracing::warn!(target: "dx::iface", %e, "interface stats failed"),
        },
        daemon_bridge::DaemonCommand::RefreshLinks => match bridge.links().await {
            Ok(snapshot) => stores.write().set_links(generation, snapshot),
            Err(error) => {
                tracing::warn!(target: "dx::network", %error, "link reconciliation failed")
            }
        },
        daemon_bridge::DaemonCommand::RefreshOperations => {
            match bridge.network_operations().await {
                Ok(operations) => stores.write().set_operations(generation, operations),
                Err(error) => {
                    tracing::warn!(target: "dx::network", %error, "operation reconciliation failed")
                }
            }
        }
        daemon_bridge::DaemonCommand::RefreshRequests => match bridge.requests().await {
            Ok(requests) => stores.write().set_requests(generation, requests),
            Err(error) => {
                tracing::warn!(target: "dx::network", %error, "request reconciliation failed")
            }
        },
        daemon_bridge::DaemonCommand::RefreshResources => match bridge.resources().await {
            Ok(resources) => stores.write().set_resources(generation, resources),
            Err(error) => {
                tracing::warn!(target: "dx::network", %error, "resource reconciliation failed")
            }
        },
        daemon_bridge::DaemonCommand::StartNetworkOperation(request) => {
            let capability = format!("network.{}", request.kind.as_str());
            if let Err(error) = stores.read().mutation_availability_at(generation, &capability) {
                tracing::warn!(target: "dx::network", %error, "network mutation blocked");
                return;
            }
            match bridge.start_network_operation(request).await {
                Ok(operation) => {
                    stores.write().apply_daemon_event(
                        generation,
                        daemon_bridge::DaemonEvent::NetworkOperation(operation),
                    );
                }
                Err(error) => {
                    tracing::warn!(target: "dx::network", %error, "network operation rejected")
                }
            }
        }
        daemon_bridge::DaemonCommand::CancelNetworkOperation { operation_id } => {
            let capability = stores
                .read()
                .network
                .operations
                .iter()
                .find(|operation| operation.operation_id == operation_id)
                .map(|operation| format!("network.{}", operation.kind.as_str()));
            let Some(capability) = capability else {
                tracing::warn!(target: "dx::network", %operation_id, "cannot cancel unknown operation");
                return;
            };
            if let Err(error) = stores.read().mutation_availability_at(generation, &capability) {
                tracing::warn!(target: "dx::network", %error, "operation cancellation blocked");
                return;
            }
            match bridge.cancel_network_operation(&operation_id).await {
                Ok(operation) => {
                    stores.write().apply_daemon_event(
                        generation,
                        daemon_bridge::DaemonEvent::NetworkOperation(operation),
                    );
                }
                Err(error) => {
                    tracing::warn!(target: "dx::network", %error, "operation cancellation rejected")
                }
            }
        }
        daemon_bridge::DaemonCommand::StartRequest(request) => {
            if let Err(error) =
                stores.read().mutation_availability_at(generation, "network.request")
            {
                tracing::warn!(target: "dx::network", %error, "native request blocked");
                return;
            }
            match bridge.start_request(request).await {
                Ok(request) => {
                    stores.write().apply_daemon_event(
                        generation,
                        daemon_bridge::DaemonEvent::Request(request),
                    );
                }
                Err(error) => {
                    tracing::warn!(target: "dx::network", %error, "native request rejected")
                }
            }
        }
        daemon_bridge::DaemonCommand::CancelRequest { request_id } => {
            if let Err(error) =
                stores.read().mutation_availability_at(generation, "network.request_cancel")
            {
                tracing::warn!(target: "dx::network", %error, "request cancellation blocked");
                return;
            }
            match bridge.cancel_request(&request_id).await {
                Ok(request) => {
                    stores.write().apply_daemon_event(
                        generation,
                        daemon_bridge::DaemonEvent::Request(request),
                    );
                }
                Err(error) => {
                    tracing::warn!(target: "dx::network", %error, "request cancellation rejected")
                }
            }
        }
        daemon_bridge::DaemonCommand::CancelResource { resource_hash } => {
            if let Err(error) =
                stores.read().mutation_availability_at(generation, "network.resource_cancel")
            {
                tracing::warn!(target: "dx::network", %error, "resource cancellation blocked");
                return;
            }
            match bridge.cancel_resource(&resource_hash).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!(target: "dx::network", %resource_hash, "resource cancellation was not accepted")
                }
                Err(error) => {
                    tracing::warn!(target: "dx::network", %error, "resource cancellation rejected")
                }
            }
        }
        daemon_bridge::DaemonCommand::RefreshPropagation { cursor } => {
            match bridge.propagation_snapshot(cursor.as_deref()).await {
                Ok(snapshot) => {
                    stores.write().set_propagation_snapshot(generation, snapshot, cursor.is_some())
                }
                Err(error) => stores.write().fail_propagation(generation, error),
            }
        }
        daemon_bridge::DaemonCommand::RefreshStandardPropagation => {
            match bridge.standard_propagation_snapshot().await {
                Ok(snapshot) => {
                    stores.write().set_standard_propagation_snapshot(generation, snapshot)
                }
                Err(error) => stores.write().fail_standard_propagation(generation, error),
            }
        }
        daemon_bridge::DaemonCommand::LoadConversations { cursor } => {
            match bridge.conversation_page(cursor.as_deref()).await {
                Ok(page) => stores.write().merge_conversation_page(
                    generation,
                    page.items
                        .into_iter()
                        .map(|conversation| state::ConversationEntry {
                            peer_hash: conversation.peer_hash,
                            peer_name: conversation.peer_name,
                            last_message: conversation.last_message,
                            last_timestamp: conversation.last_timestamp,
                            unread_count: conversation.unread_count,
                            message_count: conversation.message_count,
                            pinned: conversation.pinned,
                            muted: conversation.muted,
                        })
                        .collect(),
                    page.next_cursor,
                    page.reset,
                ),
                Err(e) => tracing::warn!(target: "dx::chat", %e, "load conversations failed"),
            }
        }
        daemon_bridge::DaemonCommand::LoadMessages { peer_hash, cursor } => {
            stores.write().mark_message_peer_loaded(generation, &peer_hash);
            match bridge.message_page(&peer_hash, cursor.as_deref()).await {
                Ok(page) => {
                    if page.reset {
                        stores.write().reset_peer_message_snapshot(generation, &peer_hash);
                    }
                    let messages = page.items.into_iter().map(state::ChatMessage::from).collect();
                    stores.write().merge_peer_message_page(
                        generation,
                        &peer_hash,
                        messages,
                        page.next_cursor,
                    );
                }
                Err(e) => tracing::warn!(target: "dx::chat", %e, "load messages failed"),
            }
        }
        daemon_bridge::DaemonCommand::QueryMessage { message_id } => {
            match bridge.message(&message_id).await {
                Ok(message) => {
                    stores.write().resolve_message(generation, &message_id, message);
                }
                Err(error) => {
                    tracing::warn!(target: "dx::chat", %error, %message_id, "message requery failed")
                }
            }
        }
        daemon_bridge::DaemonCommand::FleetStatus { destination } => {
            if let Err(error) = stores.read().mutation_availability_at(generation, "rpc.status") {
                tracing::warn!(target: "dx::fleet", %error, "fleet status blocked");
                return;
            }
            let operation = stores::FleetOperation::Status;
            let job_id =
                { stores.write().begin_fleet_job(generation, destination.clone(), operation) };
            if let Some(id) = job_id {
                let outcome = bridge.fleet_status(&destination).await;
                if let Ok(status) = &outcome {
                    stores.write().set_fleet_status(generation, status.clone());
                }
                stores.write().finish_fleet_job(
                    generation,
                    &id,
                    outcome.map(|status| {
                        format!(
                            "version={}, uptime={}",
                            status.daemon_version.unwrap_or_else(|| "not reported".into()),
                            status
                                .uptime
                                .map(|uptime| uptime.to_string())
                                .unwrap_or_else(|| "not reported".into())
                        )
                    }),
                );
            }
        }
        daemon_bridge::DaemonCommand::FleetExec { destination, command, args } => {
            if let Err(error) = stores.read().mutation_availability_at(generation, "rpc.exec") {
                tracing::warn!(target: "dx::fleet", %error, "fleet execution blocked");
                return;
            }
            let operation = stores::FleetOperation::Execute {
                command: "[REDACTED]".into(),
                args: vec![format!("{} arguments redacted", args.len())],
            };
            let job_id =
                { stores.write().begin_fleet_job(generation, destination.clone(), operation) };
            if let Some(id) = job_id {
                let outcome =
                    bridge.fleet_exec(&destination, &command, &args).await.and_then(|result| {
                        let summary = format!(
                            "exit={}, stdout_bytes={}, stderr_bytes={}",
                            result.exit_code,
                            result.stdout.len(),
                            result.stderr.len()
                        );
                        if result.exit_code == 0 {
                            Ok(summary)
                        } else {
                            Err(format!("remote command failed: {summary}"))
                        }
                    });
                stores.write().finish_fleet_job(generation, &id, outcome);
            }
        }
        daemon_bridge::DaemonCommand::FleetReboot { destination, delay } => {
            if let Err(error) = stores.read().mutation_availability_at(generation, "rpc.reboot") {
                tracing::warn!(target: "dx::fleet", %error, "fleet reboot blocked");
                return;
            }
            let operation = stores::FleetOperation::Reboot { delay_secs: delay };
            let job_id =
                { stores.write().begin_fleet_job(generation, destination.clone(), operation) };
            if let Some(id) = job_id {
                let outcome = bridge.fleet_reboot(&destination, delay).await.and_then(|result| {
                    result
                        .accepted
                        .then(|| format!("accepted, delay={}", result.delay_secs.unwrap_or(0)))
                        .ok_or_else(|| "daemon rejected reboot".into())
                });
                stores.write().finish_fleet_job(generation, &id, outcome);
            }
        }
        daemon_bridge::DaemonCommand::FleetApply { destination, profile_base64 } => {
            if let Err(error) =
                stores.read().mutation_availability_at(generation, "rpc.fleet_apply")
            {
                tracing::warn!(target: "dx::fleet", %error, "fleet profile blocked");
                return;
            }
            let job_id = stores.write().begin_fleet_job(
                generation,
                destination.clone(),
                stores::FleetOperation::ApplyProfile,
            );
            if let Some(id) = job_id {
                let outcome =
                    bridge.fleet_apply(&destination, &profile_base64).await.and_then(|result| {
                        result
                            .success
                            .then(|| {
                                format!(
                                    "success, verified={}, exit={}",
                                    result.verified, result.exit_code
                                )
                            })
                            .ok_or_else(|| {
                                format!(
                                    "profile rejected: exit={}, stderr_bytes={}",
                                    result.exit_code,
                                    result.stderr.len()
                                )
                            })
                    });
                stores.write().finish_fleet_job(generation, &id, outcome);
            }
        }
        daemon_bridge::DaemonCommand::BlockPeer { identity_hash } => {
            if let Err(error) = stores.read().mutation_availability_at(generation, "policy.update")
            {
                tracing::warn!(target: "dx::fleet", %error, "peer block blocked");
                return;
            }
            let job_id = stores.write().begin_fleet_job(
                generation,
                identity_hash.clone(),
                stores::FleetOperation::Block,
            );
            if let Some(id) = job_id {
                let outcome =
                    bridge.block_peer(&identity_hash).await.map(|()| "peer blocked locally".into());
                stores.write().finish_fleet_job(generation, &id, outcome);
            }
        }
    }
}

const fn fixture_name(fixture: backend::FixtureId) -> &'static str {
    match fixture {
        backend::FixtureId::Empty => "empty",
        backend::FixtureId::Healthy => "healthy",
        backend::FixtureId::Degraded => "degraded",
        backend::FixtureId::HighCardinality => "high-cardinality",
        backend::FixtureId::ActiveScenario => "active-scenario",
        backend::FixtureId::Error => "error",
    }
}

fn fixture_from_name(value: &str) -> Result<backend::FixtureId, String> {
    match value {
        "empty" => Ok(backend::FixtureId::Empty),
        "healthy" => Ok(backend::FixtureId::Healthy),
        "degraded" => Ok(backend::FixtureId::Degraded),
        "high-cardinality" => Ok(backend::FixtureId::HighCardinality),
        "active-scenario" => Ok(backend::FixtureId::ActiveScenario),
        "error" => Ok(backend::FixtureId::Error),
        _ => Err(format!("unknown Fixture state '{value}'")),
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

#[cfg(test)]
mod accessibility_tests {
    fn relative_luminance(rgb: [u8; 3]) -> f64 {
        let channel = |value: u8| {
            let normalized = f64::from(value) / 255.0;
            if normalized <= 0.04045 {
                normalized / 12.92
            } else {
                ((normalized + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
    }

    fn contrast(first: [u8; 3], second: [u8; 3]) -> f64 {
        let first = relative_luminance(first);
        let second = relative_luminance(second);
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }

    #[test]
    fn authored_focus_and_reduced_motion_rules_are_present() {
        let css = include_str!("assets/style.css");
        assert!(css.contains("button:focus-visible"));
        assert!(css.contains("outline: 3px solid var(--accent)"));
        assert!(css.contains("@media (prefers-reduced-motion: reduce)"));
        assert!(css.contains("animation-iteration-count: 1 !important"));
    }

    #[test]
    fn meaningful_text_and_accent_surfaces_meet_normal_text_contrast() {
        let background = [0x0d, 0x11, 0x17];
        let text_dim = [0x8b, 0x94, 0x9e];
        let accent = [0x58, 0xa6, 0xff];
        assert!(contrast(background, text_dim) >= 4.5);
        assert!(contrast(background, accent) >= 4.5);
    }

    #[test]
    fn primary_routes_and_safety_overlays_have_accessible_semantics() {
        let app = include_str!("main.rs");
        let fleet = include_str!("components/fleet_page.rs");
        let network = include_str!("components/network_page.rs");
        let lab = include_str!("components/lab_page.rs");
        assert!(app.contains("nav { class: \"tab-bar\", aria_label: \"Primary navigation\""));
        let Some(shell) = app.split("match &route_state").next() else {
            panic!("application shell source is missing");
        };
        assert_eq!(shell.matches("aria_current:").count(), 8);
        for source in [app, fleet, network, lab] {
            assert!(source.contains("role: \"dialog\""));
            assert!(source.contains("aria_modal: \"true\""));
            assert!(source.contains("autofocus: true"));
        }
    }

    #[test]
    fn primary_interactions_have_keyboard_controls_and_names() {
        let app = include_str!("main.rs");
        let inspector = include_str!("components/network_inspector.rs");
        let renderer = include_str!("components/network_renderer.rs");
        let page = include_str!("components/page_browser.rs");
        for label in ["Message title", "Delivery method", "Message content", "Message attachments"]
        {
            assert!(app.contains(&format!("aria_label: \"{label}\"")));
        }
        assert!(inspector.contains("aria_pressed:"));
        assert!(inspector.contains("aria_label: \"Search network nodes\""));
        assert!(renderer.contains("tabindex: \"0\""));
        assert!(renderer.contains("onkeydown:"));
        for label in ["Back", "Forward", "Page address", "Save destination"] {
            assert!(page.contains(&format!("aria_label: \"{label}\"")));
        }
    }
}
