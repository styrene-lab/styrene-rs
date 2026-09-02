//! Styrene TUI — three-workspace terminal UI for the Styrene mesh daemon.
//!
//! Workspaces:
//!   - Home:     Activity feed, node status, signal waveform
//!   - Peers:    Peer browser with Status/Chat/Pages/Terminal/Commands tabs
//!   - Messages: Conversation threads
//!
//! Run: `cargo run -p styrene-tui`

pub mod action;
mod app;
/// Daemon connection, command surface, and event decoding over the shared IPC client.
pub mod daemon;
mod ghost;
mod ghost_preferences;
mod mesh_state;
mod micron_widget;
mod onboarding;
mod runtime;
mod tui;

pub use ghost::run_ghost_lifecycle_check;
pub use onboarding::detect::{EnvironmentReport, scan_environment as onboarding_report};
pub use onboarding::paths::{StyrenePaths, TuiOptions};
pub use onboarding::setup::{SetupResult, run_clean_room_check};
pub use runtime::{
    RuntimeContext, RuntimeEnvironment, RuntimeHost, RuntimeOverrides, RuntimeProfile,
};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use action::{Action, action_for_key};
use app::{App, Focus, InputMode, Workspace};
use tui::splash;

/// Launch the Styrene terminal application using platform-default paths.
pub fn run_default() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(run(TuiOptions::default()))
}

/// Launch the Styrene terminal application with explicit installation options.
pub async fn run(options: TuiOptions) -> Result<()> {
    styrened::diagnostics::set_enabled(false);
    rns_core::diagnostics::set_enabled(false);
    if options.runtime_profile.is_ephemeral() {
        let preference_paths = match &options.runtime_profile {
            RuntimeProfile::PortableGhost { root } => StyrenePaths::new(
                root.join("config"),
                root.join("data"),
                root.join("run/styrene.sock"),
                root.join("home"),
            ),
            _ => StyrenePaths::standard_preferences(),
        };
        let preferences =
            ghost_preferences::GhostPreferences::load(&preference_paths.ghost_preferences_path());
        preferences.write_session_config(&options.paths.config_path())?;
    }
    let _ghost_session = ghost::GhostSession::for_paths(
        options.runtime_profile.is_ephemeral(),
        &options.paths.data_dir,
    );
    tui::spinner::seed(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as usize)
            .unwrap_or(42),
    );

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let backend = ratatui::backend::CrosstermBackend::new(io::stdout());
    let mut terminal = ratatui::Terminal::new(backend)?;

    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));

    let result = run_terminal(&mut terminal, &options).await;

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    result
}

/// The host-private parent for managed profile runtime roots. It stays short
/// because Unix socket paths have a hard length limit.
fn runtime_parent() -> Result<std::path::PathBuf, String> {
    let runtime_parent = std::env::temp_dir().join("styrene-rt");
    private_dir(&runtime_parent)?;
    Ok(runtime_parent)
}

fn private_dir(dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Start the managed profile session the TUI owns for the length of a
/// terminal session: a Quick profile for ephemeral runtime profiles, and a
/// Local profile under the data directory otherwise. A legacy layout on the
/// configured paths is adopted into the Local profile once, read-only.
pub async fn start_profile_session(
    options: &TuiOptions,
) -> Result<styrene_session::Session, String> {
    let runtime_parent = runtime_parent()?;
    if options.runtime_profile.is_ephemeral() {
        let profiles_parent = options.paths.data_dir.join("profiles");
        private_dir(&profiles_parent)?;
        return styrene_session::Session::managed(styrene_session::ManagedTarget::Quick {
            roots: styrene_session::ProfileRoots { profiles_parent, runtime_parent },
            display_name: "Ghost session",
        })
        .await
        .map_err(|error| error.to_string());
    }
    let root = options.paths.data_dir.join("profile");
    private_dir(&options.paths.data_dir)?;
    if !root.join("manifest.toml").is_file() {
        let legacy = styrened::operator_profile::LegacyLayout::for_dirs(
            &options.paths.config_dir,
            &options.paths.data_dir,
        );
        if legacy.has_state() {
            let adopted = styrened::operator_profile::StoppedManagedProfile::adopt_legacy(
                &root,
                &runtime_parent,
                "Local node",
                &legacy,
            )
            .map_err(|error| format!("adopt legacy layout: {error}"))?;
            drop(adopted);
        }
    }
    styrene_session::Session::managed(styrene_session::ManagedTarget::Local {
        root,
        runtime_parent,
        display_name: Some("Local node"),
    })
    .await
    .map_err(|error| error.to_string())
}

/// Connect to a daemon socket that may still be coming up, retrying briefly.
pub async fn connect_with_retry(
    socket: &std::path::Path,
) -> Result<daemon::DaemonConnection, String> {
    let mut last_error = "socket not ready".to_string();
    for _ in 0..30 {
        match daemon::connect(Some(socket)).await {
            Ok(connection) => return Ok(connection),
            Err(error) => last_error = error,
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(last_error)
}

#[derive(Debug)]
pub(crate) enum ConnectionControl {
    Connect(std::path::PathBuf),
    Disconnect,
}

struct ActiveDaemonSession {
    _connection: daemon::DaemonConnection,
    forwarder: tokio::task::JoinHandle<()>,
    executor: tokio::task::JoinHandle<()>,
    poller: tokio::task::JoinHandle<()>,
}

impl Drop for ActiveDaemonSession {
    fn drop(&mut self) {
        self.forwarder.abort();
        self.executor.abort();
        self.poller.abort();
    }
}

fn install_daemon_session(
    app: &mut App,
    mut connection: daemon::DaemonConnection,
    event_tx: tokio::sync::mpsc::Sender<daemon::TuiEvent>,
) -> ActiveDaemonSession {
    let handle = Arc::new(Mutex::new(connection.take_handle()));
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<daemon::QueuedDaemonCmd>(64);
    app.cmd_tx = Some(cmd_tx);
    let executor = daemon::spawn_command_executor(handle.clone(), cmd_rx, event_tx.clone());
    let poller = daemon::spawn_poll_task(handle, event_tx.clone(), 10);
    let (_closed_tx, closed_rx) = tokio::sync::mpsc::channel(1);
    let mut events = std::mem::replace(&mut connection.events, closed_rx);
    let forwarder = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if event_tx.send(event).await.is_err() {
                break;
            }
        }
    });
    ActiveDaemonSession { _connection: connection, forwarder, executor, poller }
}

fn clear_daemon_authority(app: &mut App) {
    app.daemon_connected = false;
    app.daemon_session_accepting_events = false;
    app.rns_initialized = false;
    app.transport_active = false;
    app.connection_generation = None;
    app.event_connection_generation = None;
    app.active_capabilities = None;
    app.cmd_tx = None;
}

async fn run_terminal(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    options: &TuiOptions,
) -> Result<()> {
    let mut app = App::new();
    app.runtime_profile = options.runtime_profile.clone();

    // ── Splash ──────────────────────────────────────────────────────────────
    let size = terminal.size()?;
    if let Some(mut splash) = splash::SplashScreen::new(size.width, size.height) {
        let start = std::time::Instant::now();
        loop {
            terminal.draw(|f| splash.draw(f, app.theme.as_ref()))?;
            let interval = splash::SplashScreen::frame_interval();
            if event::poll(interval)?
                && let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
                && (splash.ready_to_dismiss() || start.elapsed() > Duration::from_millis(300))
            {
                break;
            }
            splash.tick();
            if splash.ready_to_dismiss() && splash.hold_count > splash::HOLD_FRAMES + 30 {
                break;
            }
            if start.elapsed() > Duration::from_secs(5) {
                break;
            }
        }
    }

    // ── Onboarding wizard ─────────────────────────────────────────────────────
    let env = onboarding::detect::scan_environment(&options.paths);
    let daemon_mode = if options.runtime_profile.is_ephemeral() {
        onboarding::setup::DaemonMode::Embedded
    } else if env.needs_wizard() {
        let mut wizard = onboarding::WizardState::new(env);
        loop {
            terminal.draw(|f| wizard.draw(f, app.theme.as_ref()))?;
            if event::poll(Duration::from_millis(16))?
                && let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
            {
                match wizard.handle_key(k) {
                    onboarding::WizardAction::Complete(result) => {
                        if let Err(e) = result.apply(&options.paths) {
                            app.conversation.push_system(&format!(
                                "⬡ setup error: {e} — continuing with defaults"
                            ));
                        }
                        break result.daemon_mode;
                    }
                    onboarding::WizardAction::Quit => return Ok(()),
                    onboarding::WizardAction::Continue => {}
                }
            }
        }
    } else {
        onboarding::load_tui_prefs(&options.paths).daemon_mode_or_default()
    };

    // ── Welcome + effects ────────────────────────────────────────────────────
    {
        let t = app.theme.as_ref();
        app.effects.queue_startup(t);
    }
    app.push_welcome();

    // ── Daemon connection (mode-aware) ──────────────────────────────────────
    let (mut daemon_tx, mut daemon_rx) = tokio::sync::mpsc::channel::<daemon::TuiEvent>(128);
    let (connection_tx, mut connection_rx) = tokio::sync::mpsc::unbounded_channel();
    app.connection_tx = Some(connection_tx);

    // Owned daemons run as managed profiles through the shared session layer
    // and describe their own profile; nothing is derived from a mode name.
    let mut profile_session = None;
    if daemon_mode == onboarding::setup::DaemonMode::Embedded {
        match start_profile_session(options).await {
            Ok(session) => {
                app.conversation.push_system(&format!("⬡ {} profile ready", session.profile()));
                profile_session = Some(session);
            }
            Err(error) => {
                app.conversation.push_system(&format!("⬡ profile failed: {error}"));
            }
        }
    }

    let connect_result = match daemon_mode {
        onboarding::setup::DaemonMode::Embedded => match profile_session.as_ref() {
            Some(session) => connect_with_retry(&session.metadata().endpoint).await,
            None => Err("managed profile failed to start".into()),
        },
        onboarding::setup::DaemonMode::Background => {
            app.conversation.push_system(
                "⬡ background mode requires an externally managed daemon; trying configured socket",
            );
            daemon::connect(Some(&options.paths.daemon_socket)).await
        }
        onboarding::setup::DaemonMode::ConnectExisting => {
            daemon::connect(Some(&options.paths.daemon_socket)).await
        }
    };

    let mut active_session = match connect_result {
        Ok(connection) => {
            app.daemon_connected = true;
            app.daemon_session_accepting_events = true;
            app.conversation.push_system("⬡ daemon connected");
            Some(install_daemon_session(&mut app, connection, daemon_tx.clone()))
        }
        Err(e) => {
            app.conversation.push_system(&format!("⬡ daemon unavailable ({e}) — demo mode"));
            None
        }
    };

    // Keep the owned profile alive for the full terminal session.
    let mut profile_session = profile_session;

    // ── Main event loop — 60fps ──────────────────────────────────────────────
    loop {
        while let Ok(control) = connection_rx.try_recv() {
            active_session.take();
            clear_daemon_authority(&mut app);
            let (next_tx, next_rx) = tokio::sync::mpsc::channel::<daemon::TuiEvent>(128);
            daemon_tx = next_tx;
            daemon_rx = next_rx;
            if let ConnectionControl::Connect(path) = control {
                match daemon::connect(Some(&path)).await {
                    Ok(connection) => {
                        app.daemon_connected = true;
                        app.daemon_session_accepting_events = true;
                        app.conversation
                            .push_system(&format!("⬡ daemon connected: {}", path.display()));
                        active_session =
                            Some(install_daemon_session(&mut app, connection, daemon_tx.clone()));
                    }
                    Err(error) => app.conversation.push_system(&format!(
                        "⬡ connection failed for {}: {error}",
                        path.display()
                    )),
                }
            }
        }

        // Drain daemon events
        while let Ok(ev) = daemon_rx.try_recv() {
            let disconnected = matches!(ev, daemon::TuiEvent::Disconnected(_));
            daemon::apply_event(&mut app, ev);
            if disconnected {
                active_session.take();
                let (_detached_tx, next_rx) = tokio::sync::mpsc::channel::<daemon::TuiEvent>(128);
                daemon_rx = next_rx;
                break;
            }
        }

        terminal.draw(|f| app.draw(f))?;

        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Mouse(m) => {
                    use crossterm::event::MouseEventKind;
                    match m.kind {
                        MouseEventKind::ScrollUp => app.active_conversation_mut().scroll_up(3),
                        MouseEventKind::ScrollDown => app.active_conversation_mut().scroll_down(3),
                        _ => {}
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press && handle_key(&mut app, key) => {
                    break;
                }
                _ => {}
            }
        }

        app.tick();
    }

    // The owned daemon stops with the session; a Quick root goes with it.
    if let Some(mut session) = profile_session.take() {
        session.close().await;
    }

    Ok(())
}

/// Returns true if the app should quit.
fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if let Some(export) = &app.paper_export {
        match key.code {
            KeyCode::Esc => app.paper_export = None,
            KeyCode::Char('s') => {
                let path = format!("paper-{}.lxm", export.message_id);
                match std::fs::write(&path, export.uri.as_bytes()) {
                    Ok(()) => {
                        app.conversation.push_system(&format!("paper export saved to {path}"))
                    }
                    Err(error) => {
                        app.conversation.push_system(&format!("paper export save failed: {error}"))
                    }
                }
            }
            _ => {}
        }
        return false;
    }
    // ── Input mode routing ──────────────────────────────────────────────────
    match &app.input_mode {
        InputMode::Compose => return handle_compose_key(app, key),
        InputMode::Command { .. } => return handle_command_key(app, key),
        InputMode::Search { .. } => return handle_search_key(app, key),
        InputMode::PageField { .. } => return handle_page_field_key(app, key),
        InputMode::SavePath { .. } => return handle_save_path_key(app, key),
        InputMode::Normal => {}
    }

    if let Some(action) = action_for_key(app, key) {
        let tab_handles_activation = action == Action::Activate
            && app.focus == Focus::Main
            && app.workspace == Workspace::Peers
            && matches!(
                app.peer_tab,
                app::PeerTab::Pages | app::PeerTab::Terminal | app::PeerTab::Commands
            );
        if !tab_handles_activation {
            if action == Action::Quit {
                let now = std::time::Instant::now();
                if let Some(last) = app.last_ctrl_c
                    && now.duration_since(last) < Duration::from_secs(1)
                {
                    return true;
                }
                app.last_ctrl_c = Some(now);
                app.conversation.push_system("Press Ctrl+C again to quit");
                return false;
            }
            return app.dispatch(action);
        }
    }

    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::NONE) if app.focus != Focus::Input => {
            return true;
        }

        // Workspace navigation accelerators
        (KeyCode::Char('1'), _) => app.set_workspace(Workspace::Home),
        (KeyCode::Char('2'), _) => app.set_workspace(Workspace::Peers),
        (KeyCode::Char('3'), _) => app.set_workspace(Workspace::Messages),
        (KeyCode::Char('4'), _) if app.workspace != Workspace::Peers => {
            app.set_workspace(Workspace::Propagation);
        }
        (KeyCode::Char('r'), KeyModifiers::NONE) if app.workspace == Workspace::Propagation => {
            app.send_daemon_cmd(daemon::DaemonCmd::RequeryStandardPropagation);
        }
        (KeyCode::Char('O'), _) => {
            if let Some(peer_hash) = app.compose_peer() {
                let cursor = app.message_cursors.get(&peer_hash).cloned();
                app.send_daemon_cmd(daemon::DaemonCmd::LoadMessagePage { peer_hash, cursor });
            }
        }
        (KeyCode::Char('o'), KeyModifiers::NONE) if app.workspace == Workspace::Messages => {
            let cursor = app.conversation_cursor.clone();
            app.send_daemon_cmd(daemon::DaemonCmd::LoadConversationPage { cursor });
        }
        (KeyCode::Char('R'), _)
            if app.workspace == Workspace::Messages
                || (app.workspace == Workspace::Peers && app.peer_tab == app::PeerTab::Chat) =>
        {
            let message_id = app.active_conversation_mut().last_sent_id().map(str::to_owned);
            if let Some(message_id) = message_id {
                app.send_daemon_cmd(daemon::DaemonCmd::RetryMessage { message_id });
            }
        }
        (KeyCode::Char('C'), _) => {
            let message_id = app.active_conversation_mut().last_sent_id().map(str::to_owned);
            if let Some(message_id) = message_id {
                app.send_daemon_cmd(daemon::DaemonCmd::CancelMessage { message_id });
            }
        }

        // Mode triggers
        (KeyCode::Char(':'), _) => {
            app.input_mode = InputMode::Command { buffer: String::new() };
            app.focus = Focus::Input;
        }

        // Sidebar navigation
        (KeyCode::Char('j') | KeyCode::Down, _) if app.focus == Focus::Sidebar => {
            let max = app.peers.len().saturating_sub(1);
            app.sidebar_selection = (app.sidebar_selection + 1).min(max);
        }
        (KeyCode::Char('k') | KeyCode::Up, _) if app.focus == Focus::Sidebar => {
            app.sidebar_selection = app.sidebar_selection.saturating_sub(1);
        }
        (KeyCode::Enter, _) if app.focus == Focus::Sidebar => {
            // Select peer from sidebar — use filtered items to handle search
            let items = app.sidebar_items();
            if let Some((hash, _, _)) = items.get(app.sidebar_selection) {
                let hash = hash.clone();
                match app.workspace {
                    Workspace::Peers => app.select_peer(hash),
                    Workspace::Messages => {
                        app.selected_conversation = Some(hash);
                        app.focus = Focus::Main;
                    }
                    Workspace::Home => {
                        // Jump to Peers workspace with this peer selected
                        app.select_peer(hash);
                        app.set_workspace(Workspace::Peers);
                        app.focus = Focus::Main;
                    }
                    Workspace::Propagation => {}
                }
            }
        }
        // Execute selected command in Commands tab
        (KeyCode::Enter, _)
            if app.focus == Focus::Main
                && app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Commands =>
        {
            if let Some(peer_hash) = app.selected_peer.clone() {
                let action = app::CommandAction::ALL[app.command_tab.selected];
                let capability = match action {
                    app::CommandAction::QueryStatus => Some("rpc.status"),
                    app::CommandAction::Reboot => Some("rpc.reboot"),
                    app::CommandAction::ConfigPush => None,
                };
                if let Some(capability) = capability
                    && let Err(reason) = app.mutation_availability(capability)
                {
                    app.command_tab.result_text = format!("  Command disabled: {reason}");
                    app.command_tab.is_executing = false;
                    return false;
                }
                app.command_tab.is_executing = true;
                app.command_tab.result_text = format!(
                    "  Executing {} on {}...",
                    action.title(),
                    &peer_hash[..8.min(peer_hash.len())]
                );

                use daemon::DaemonCmd;
                match action {
                    app::CommandAction::QueryStatus => {
                        app.send_daemon_cmd(DaemonCmd::DeviceStatus { dest_hash: peer_hash });
                    }
                    app::CommandAction::Reboot => {
                        app.send_daemon_cmd(DaemonCmd::RebootDevice {
                            dest_hash: peer_hash,
                            delay_secs: Some(5),
                        });
                    }
                    app::CommandAction::ConfigPush => {
                        app.command_tab.is_executing = false;
                        app.command_tab.result_text =
                            "  Config push requires a profile file. Use CLI: styrene fleet apply"
                                .into();
                    }
                }
            }
        }

        (KeyCode::Char('b'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_content.as_ref().is_some_and(|page| page.navigation.can_back) =>
        {
            let mut request = styrene_ipc::types::PageNavigationRequest::default();
            request.session_id =
                app.page_content.as_ref().map(|page| page.navigation.session_id.clone());
            request.action = styrene_ipc::types::PageNavigationAction::Back;
            app.send_daemon_cmd(daemon::DaemonCmd::NavigatePage(request));
        }
        (KeyCode::Char('f'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_content.as_ref().is_some_and(|page| page.navigation.can_forward) =>
        {
            let mut request = styrene_ipc::types::PageNavigationRequest::default();
            request.session_id =
                app.page_content.as_ref().map(|page| page.navigation.session_id.clone());
            request.action = styrene_ipc::types::PageNavigationAction::Forward;
            app.send_daemon_cmd(daemon::DaemonCmd::NavigatePage(request));
        }
        (KeyCode::Char('r'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_content.is_some() =>
        {
            let mut request = styrene_ipc::types::PageNavigationRequest::default();
            request.session_id =
                app.page_content.as_ref().map(|page| page.navigation.session_id.clone());
            request.action = styrene_ipc::types::PageNavigationAction::Reload;
            app.send_daemon_cmd(daemon::DaemonCmd::NavigatePage(request));
        }
        (KeyCode::Char('R'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_content.is_some() =>
        {
            let mut request = styrene_ipc::types::PageNavigationRequest::default();
            request.session_id =
                app.page_content.as_ref().map(|page| page.navigation.session_id.clone());
            request.target = app.page_content.as_ref().map(|page| page.navigation.address.clone());
            request.bypass_cache = true;
            app.send_daemon_cmd(daemon::DaemonCmd::NavigatePage(request));
        }
        (KeyCode::Char('x'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_content.is_some() =>
        {
            if app.page_content.is_some() {
                app.request_page_transition(app::PendingPageTransition::Dismiss);
            }
        }
        (KeyCode::Char('n'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_content.is_some() =>
        {
            let count = app.page_content.as_ref().map_or(0, |page| page.link_targets.len());
            if count > 0 {
                app.page_link_selection = (app.page_link_selection + 1) % count;
            }
        }
        (KeyCode::Char('p'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_content.is_some() =>
        {
            let count = app.page_content.as_ref().map_or(0, |page| page.link_targets.len());
            if count > 0 {
                app.page_link_selection =
                    app.page_link_selection.checked_sub(1).unwrap_or(count - 1);
            }
        }
        (KeyCode::Char('i'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_content.is_some() =>
        {
            let count = app.page_content.as_ref().map_or(0, |page| page.fields.len());
            if count > 0 {
                app.page_field_selection = (app.page_field_selection + 1) % count;
            }
        }
        (KeyCode::Char('u'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_content.is_some() =>
        {
            let count = app.page_content.as_ref().map_or(0, |page| page.fields.len());
            if count > 0 {
                app.page_field_selection =
                    app.page_field_selection.checked_sub(1).unwrap_or(count - 1);
            }
        }
        (KeyCode::Char('e'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_content.is_some() =>
        {
            if let Some(field) =
                app.page_content.as_ref().and_then(|page| page.fields.get(app.page_field_selection))
                && matches!(
                    field.kind,
                    styrene_ipc::types::PageFormFieldKind::Text
                        | styrene_ipc::types::PageFormFieldKind::Password
                )
            {
                let buffer = app
                    .page_field_values
                    .get(&field.name)
                    .and_then(|values| values.first())
                    .cloned()
                    .unwrap_or_default();
                app.input_mode = InputMode::PageField {
                    name: field.name.clone(),
                    password: field.kind == styrene_ipc::types::PageFormFieldKind::Password,
                    buffer,
                };
            }
        }
        (KeyCode::Char(' '), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_content.is_some() =>
        {
            if let Some(field) =
                app.page_content.as_ref().and_then(|page| page.fields.get(app.page_field_selection))
                && matches!(
                    field.kind,
                    styrene_ipc::types::PageFormFieldKind::Checkbox
                        | styrene_ipc::types::PageFormFieldKind::Radio
                )
                && let Some(value) = &field.value
            {
                if field.kind == styrene_ipc::types::PageFormFieldKind::Radio {
                    app.page_field_values.insert(field.name.clone(), vec![value.clone()]);
                } else {
                    let values = app.page_field_values.entry(field.name.clone()).or_default();
                    if values.contains(value) {
                        values.retain(|selected| selected != value);
                    } else {
                        values.push(value.clone());
                    }
                }
            }
        }
        (KeyCode::Char('d'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_download.is_some() =>
        {
            if let Some(download) = &app.page_download {
                let download_id = download.download_id.clone();
                app.send_daemon_cmd(daemon::DaemonCmd::QueryFileDownload { download_id });
            }
        }
        (KeyCode::Char('c'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app
                    .page_download
                    .as_ref()
                    .is_some_and(|download| !download.state.is_terminal()) =>
        {
            if let Some(download) = &app.page_download {
                let download_id = download.download_id.clone();
                app.send_daemon_cmd(daemon::DaemonCmd::CancelFileDownload { download_id });
            }
        }
        (KeyCode::Char('s'), _)
            if app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages
                && app.page_download.as_ref().is_some_and(|download| {
                    download.state == styrene_ipc::types::FileDownloadState::Completed
                        && download.integrity_verified
                }) =>
        {
            if let Some(download) = &app.page_download {
                app.input_mode = InputMode::SavePath {
                    download_id: download.download_id.clone(),
                    buffer: String::new(),
                };
                app.focus = Focus::Input;
            }
        }

        // Browse pages in Pages tab
        (KeyCode::Enter, _)
            if app.focus == Focus::Main
                && app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages =>
        {
            if let Some(peer_hash) = app.selected_peer.clone() {
                if let Some(page) = app.page_content.as_ref() {
                    if let Some(link) = page.link_targets.get(app.page_link_selection) {
                        if link.target.contains("/file/") {
                            let mut request = styrene_ipc::types::FileDownloadRequest::default();
                            request.session_id = Some(page.navigation.session_id.clone());
                            request.target = link.target.clone();
                            app.send_daemon_cmd(daemon::DaemonCmd::StartFileDownload(request));
                        } else {
                            let mut request = styrene_ipc::types::PageNavigationRequest::default();
                            request.session_id = Some(page.navigation.session_id.clone());
                            request.target = Some(link.target.clone());
                            if !link.submitted_fields.is_empty() {
                                let mut submission =
                                    styrene_ipc::types::PageFormSubmission::default();
                                submission.values = link
                                    .submitted_fields
                                    .iter()
                                    .filter_map(|name| {
                                        app.page_field_values
                                            .get(name)
                                            .cloned()
                                            .map(|values| (name.clone(), values))
                                    })
                                    .collect();
                                request.submission = Some(submission);
                            }
                            app.send_daemon_cmd(daemon::DaemonCmd::NavigatePage(request));
                        }
                    }
                } else if app.page_index.is_empty()
                    && app.peers.iter().any(|peer| peer.hash == peer_hash && peer.native_page_host)
                {
                    app.send_daemon_cmd(daemon::DaemonCmd::BrowsePage {
                        host: peer_hash,
                        path: "/page/index.mu".into(),
                    });
                    app.conversation.push_system("⬡ loading native page index...");
                } else {
                    let index = app.page_selection.min(app.page_index.len() - 1);
                    let path = app.page_index[index].clone();
                    app.send_daemon_cmd(daemon::DaemonCmd::BrowsePage { host: peer_hash, path });
                    app.conversation.push_system("⬡ loading selected page...");
                }
            }
        }

        // Open terminal session in Terminal tab
        (KeyCode::Enter, _)
            if app.focus == Focus::Main
                && app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Terminal
                && app.terminal_tab.session_id.is_none() =>
        {
            app.terminal_tab.status = app::TerminalStatus::Error(
                "Terminal sessions require daemon connection. Feature in progress.".into(),
            );
        }

        (KeyCode::Char('g'), _) if app.focus == Focus::Sidebar => {
            app.sidebar_selection = 0;
        }
        (KeyCode::Char('G'), _) if app.focus == Focus::Sidebar => {
            app.sidebar_selection = app.peers.len().saturating_sub(1);
        }

        // Sidebar toggle
        (KeyCode::Char('['), _) => app.sidebar_visible = false,
        (KeyCode::Char(']'), _) => app.sidebar_visible = true,

        // Focus cycling
        (KeyCode::Esc, _) => {
            // Close settings panel first if open
            if app.settings_open {
                app.settings_open = false;
            } else {
                match app.focus {
                    Focus::Main => app.focus = Focus::Sidebar,
                    Focus::Input => app.focus = Focus::Sidebar,
                    Focus::Sidebar => {
                        // Deselect
                        if app.page_content.is_some() {
                            app.request_page_transition(app::PendingPageTransition::Deselect);
                        } else {
                            app.selected_peer = None;
                            app.selected_conversation = None;
                        }
                    }
                }
            }
        }

        // Peer tab switching (in Peers workspace)
        (KeyCode::Char(n @ '4'..='5'), _) if app.workspace == Workspace::Peers => {
            let idx = (n as u8 - b'1') as usize;
            if let Some(tab) = app::PeerTab::ALL.get(idx) {
                app.set_peer_tab(*tab);
            }
        }

        // Scroll main pane
        (KeyCode::PageUp, _) => app.active_conversation_mut().scroll_up(20),
        (KeyCode::PageDown, _) => app.active_conversation_mut().scroll_down(20),
        (KeyCode::Char('j') | KeyCode::Down, _) if app.focus == Focus::Main => {
            if app.workspace == Workspace::Peers && app.peer_tab == app::PeerTab::Commands {
                let max = app::CommandAction::ALL.len().saturating_sub(1);
                app.command_tab.selected = (app.command_tab.selected + 1).min(max);
            } else {
                app.active_conversation_mut().scroll_down(3);
            }
        }
        (KeyCode::Char('k') | KeyCode::Up, _) if app.focus == Focus::Main => {
            if app.workspace == Workspace::Peers && app.peer_tab == app::PeerTab::Commands {
                app.command_tab.selected = app.command_tab.selected.saturating_sub(1);
            } else {
                app.active_conversation_mut().scroll_up(3);
            }
        }

        // Peer tab navigation
        (KeyCode::Right, _) if app.workspace == Workspace::Peers && app.focus == Focus::Main => {
            app.next_peer_tab();
        }

        // Demo triggers
        (KeyCode::Char('r'), _) if app.focus != Focus::Input => app.demo_announce(),
        (KeyCode::Char('l'), _) if app.focus != Focus::Input => app.demo_link(),

        _ => {}
    }

    false
}

fn handle_page_field_key(app: &mut App, key: KeyEvent) -> bool {
    let InputMode::PageField { name, password: _, buffer } = &mut app.input_mode else {
        return false;
    };
    match key.code {
        KeyCode::Esc => app.input_mode = InputMode::Normal,
        KeyCode::Enter => {
            let name = name.clone();
            let value = buffer.clone();
            app.page_field_values.insert(name, vec![value]);
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Backspace => {
            buffer.pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => buffer.push(c),
        _ => {}
    }
    false
}

fn handle_save_path_key(app: &mut App, key: KeyEvent) -> bool {
    let InputMode::SavePath { download_id, buffer } = &mut app.input_mode else {
        return false;
    };
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.focus = Focus::Main;
        }
        KeyCode::Enter => {
            let download_id = download_id.clone();
            let destination = buffer.clone();
            app.input_mode = InputMode::Normal;
            app.focus = Focus::Main;
            if destination.is_empty() {
                app.conversation.push_system("save path must be an explicit absolute path");
            } else {
                app.send_daemon_cmd(daemon::DaemonCmd::SaveFileDownload {
                    download_id,
                    destination,
                });
            }
        }
        KeyCode::Backspace => {
            buffer.pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => buffer.push(c),
        _ => {}
    }
    false
}

fn handle_compose_key(app: &mut App, key: KeyEvent) -> bool {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => {
            if let Some(peer_hash) = app.compose_peer() {
                let content = app.editor.render_text();
                app.send_daemon_cmd(crate::daemon::DaemonCmd::SetDraft { peer_hash, content });
            }
            app.input_mode = InputMode::Normal;
            app.focus = Focus::Sidebar;
        }
        (KeyCode::Enter, _) => {
            let text = app.editor.render_text();
            if !text.is_empty() {
                app.handle_compose_submit(text);
            }
        }
        (KeyCode::Tab, _) => app.cycle_delivery_method(),
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            if let Some(peer_hash) = app.compose_peer() {
                app.send_daemon_cmd(crate::daemon::DaemonCmd::ClearDraft { peer_hash });
            }
        }
        (KeyCode::Char(c), mods) if !mods.contains(KeyModifiers::CONTROL) => {
            app.editor.insert(c);
        }
        (KeyCode::Backspace, _) => app.editor.backspace(),
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => app.editor.clear_line(),
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => app.editor.kill_to_end(),
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => app.editor.delete_word_backward(),
        _ => {}
    }
    false
}

fn handle_command_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.focus = Focus::Sidebar;
        }
        KeyCode::Enter => {
            let buffer = match &app.input_mode {
                InputMode::Command { buffer } => buffer.clone(),
                _ => String::new(),
            };
            app.input_mode = InputMode::Normal;
            app.focus = Focus::Sidebar;
            return execute_command(app, &buffer);
        }
        KeyCode::Char(c) => {
            if let InputMode::Command { ref mut buffer } = app.input_mode {
                buffer.push(c);
            }
        }
        KeyCode::Backspace => {
            if let InputMode::Command { ref mut buffer } = app.input_mode {
                buffer.pop();
            }
        }
        _ => {}
    }
    false
}

/// Parse and execute a command-mode string. Returns true if the app should quit.
fn execute_command(app: &mut App, input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return false;
    }

    let mut parts = trimmed.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();

    match cmd {
        "q" | "quit" => return true,

        "connect" => {
            if arg.is_empty() {
                app.conversation.push_system("usage: :connect <addr>");
            } else if let Some(tx) = app.connection_tx.clone() {
                clear_daemon_authority(app);
                if tx.send(ConnectionControl::Connect(std::path::PathBuf::from(arg))).is_err() {
                    app.conversation.push_system("⬡ connection manager unavailable");
                } else {
                    app.conversation.push_system(&format!("⬡ connecting to {arg}"));
                }
            } else {
                app.conversation.push_system("⬡ connection manager unavailable");
            }
        }

        "disconnect" => {
            if app.daemon_connected {
                clear_daemon_authority(app);
                if let Some(tx) = &app.connection_tx {
                    let _ = tx.send(ConnectionControl::Disconnect);
                }
                app.conversation.push_system("⬡ disconnected from daemon");
                app.activity.push(crate::mesh_state::ActivityEntry::new(
                    crate::mesh_state::ActivityKind::LinkDown,
                    "daemon",
                    "disconnected by operator",
                ));
            } else {
                app.conversation.push_system("⬡ not connected");
            }
        }

        "settings" => {
            app.settings_open = !app.settings_open;
            if app.settings_open {
                app.conversation.push_system("⬡ settings panel opened (Esc to close)");
            }
        }

        "announce" => {
            if let Err(reason) = app.mutation_availability("network.announce") {
                app.conversation.push_system(&format!("⬡ announce disabled — {reason}"));
            } else {
                app.send_daemon_cmd(daemon::DaemonCmd::Announce);
                app.conversation.push_system("⬡ announce submitted; awaiting daemon observation");
            }
        }

        "path" | "link-open" => {
            let kind = match cmd {
                "path" => styrene_ipc::types::NetworkOperationKind::PathRequest,
                _ => styrene_ipc::types::NetworkOperationKind::LinkOpen,
            };
            let capability = format!("network.{}", kind.as_str());
            if arg.is_empty() {
                app.conversation.push_system(&format!("usage: :{cmd} <destination_hash>"));
            } else if let Err(reason) = app.mutation_availability(&capability) {
                app.conversation.push_system(&format!("⬡ {cmd} disabled — {reason}"));
            } else {
                let mut request = styrene_ipc::types::StartNetworkOperationInfo::default();
                request.kind = kind;
                request.destination_hash = Some(arg.to_string());
                request.timeout_ms = 15_000;
                app.send_daemon_cmd(daemon::DaemonCmd::StartNetworkOperation(request));
                app.conversation.push_system("⬡ operation submitted; awaiting daemon observation");
            }
        }

        "probe" => {
            if arg.is_empty() {
                app.conversation.push_system("usage: :probe <link_id>");
            } else if let Err(reason) = app.mutation_availability("network.probe") {
                app.conversation.push_system(&format!("⬡ probe disabled — {reason}"));
            } else {
                let mut request = styrene_ipc::types::StartNetworkOperationInfo::default();
                request.kind = styrene_ipc::types::NetworkOperationKind::Probe;
                request.link_id = Some(arg.to_string());
                request.timeout_ms = 15_000;
                app.send_daemon_cmd(daemon::DaemonCmd::StartNetworkOperation(request));
            }
        }

        "link-close" => {
            if arg.is_empty() {
                app.conversation.push_system("usage: :link-close <link_id>");
            } else if let Err(reason) = app.mutation_availability("network.link_close") {
                app.conversation.push_system(&format!("⬡ link-close disabled — {reason}"));
            } else {
                let mut request = styrene_ipc::types::StartNetworkOperationInfo::default();
                request.kind = styrene_ipc::types::NetworkOperationKind::LinkClose;
                request.link_id = Some(arg.to_string());
                request.timeout_ms = 15_000;
                app.send_daemon_cmd(daemon::DaemonCmd::StartNetworkOperation(request));
            }
        }

        "request" => {
            let mut fields = arg.splitn(3, ' ');
            let link_id = fields.next().unwrap_or("");
            let path = fields.next().unwrap_or("");
            let data = fields.next().unwrap_or("");
            if link_id.is_empty() || path.is_empty() {
                app.conversation.push_system("usage: :request <link_id> <path> [data]");
            } else if let Err(reason) = app.mutation_availability("network.request") {
                app.conversation.push_system(&format!("⬡ request disabled — {reason}"));
            } else {
                let mut request = styrene_ipc::types::StartRequestInfo::default();
                request.link_id = link_id.to_string();
                request.path = path.to_string();
                request.data = data.as_bytes().to_vec();
                request.timeout_ms = 15_000;
                request.max_response_size = 4 * 1024 * 1024;
                app.send_daemon_cmd(daemon::DaemonCmd::StartRequest(request));
            }
        }

        "cancel" => {
            if let Some(operation) =
                app.network_operations.iter().find(|item| item.operation_id == arg)
            {
                let capability = format!("network.{}", operation.kind.as_str());
                if let Err(reason) = app.mutation_availability(&capability) {
                    app.conversation.push_system(&format!("⬡ cancellation disabled — {reason}"));
                } else {
                    app.send_daemon_cmd(daemon::DaemonCmd::CancelNetworkOperation {
                        operation_id: arg.to_string(),
                    });
                }
            } else if app.request_observations.iter().any(|item| item.request_id == arg) {
                if let Err(reason) = app.mutation_availability("network.request_cancel") {
                    app.conversation.push_system(&format!("⬡ cancellation disabled — {reason}"));
                } else {
                    app.send_daemon_cmd(daemon::DaemonCmd::CancelRequest {
                        request_id: arg.to_string(),
                    });
                }
            } else {
                app.conversation.push_system(
                    "⬡ cancellation rejected — unknown current-generation operation or request",
                );
            }
        }

        "resource-cancel" => {
            if arg.is_empty() {
                app.conversation.push_system("usage: :resource-cancel <resource_hash>");
            } else {
                app.send_daemon_cmd(daemon::DaemonCmd::CancelResource {
                    resource_hash: arg.to_string(),
                });
            }
        }

        "routes" => app.send_daemon_cmd(daemon::DaemonCmd::InspectRoutes),
        "interfaces" => app.send_daemon_cmd(daemon::DaemonCmd::InspectInterfaces),
        "links" => app.send_daemon_cmd(daemon::DaemonCmd::InspectLinks),
        "requests" => app.send_daemon_cmd(daemon::DaemonCmd::InspectRequests),
        "resources" => app.send_daemon_cmd(daemon::DaemonCmd::InspectResources),

        "exec" => {
            let mut fields = arg.split_whitespace();
            let destination = fields.next().unwrap_or("");
            let command = fields.next().unwrap_or("");
            let args = fields.map(ToOwned::to_owned).collect::<Vec<_>>();
            if destination.is_empty() || command.is_empty() {
                app.conversation.push_system("usage: :exec <destination_hash> <command> [args]");
            } else {
                app.send_daemon_cmd(daemon::DaemonCmd::Exec {
                    dest_hash: destination.into(),
                    command: command.into(),
                    args,
                });
            }
        }

        "block" => {
            if arg.is_empty() {
                app.conversation.push_system("usage: :block <identity_hash>");
            } else if let Err(reason) = app.mutation_availability("policy.update") {
                app.conversation.push_system(&format!("⬡ block disabled — {reason}"));
            } else {
                app.send_daemon_cmd(daemon::DaemonCmd::BlockPeer {
                    identity_hash: arg.to_string(),
                });
                app.conversation
                    .push_system(&format!("⬡ blocking peer: {}...", &arg[..12.min(arg.len())]));
            }
        }

        "unblock" => {
            if arg.is_empty() {
                app.conversation.push_system("usage: :unblock <identity_hash>");
            } else if let Err(reason) = app.mutation_availability("policy.update") {
                app.conversation.push_system(&format!("⬡ unblock disabled — {reason}"));
            } else {
                app.send_daemon_cmd(daemon::DaemonCmd::UnblockPeer {
                    identity_hash: arg.to_string(),
                });
                app.conversation
                    .push_system(&format!("⬡ unblocking peer: {}...", &arg[..12.min(arg.len())]));
            }
        }

        "alias" => {
            let mut alias_parts = arg.splitn(2, ' ');
            let peer = alias_parts.next().unwrap_or("");
            let name = alias_parts.next().unwrap_or("").trim();
            if peer.is_empty() || name.is_empty() {
                app.conversation.push_system("usage: :alias <peer_hash> <display_name>");
            } else {
                // Update local peer record
                if let Some(p) = app.peers.iter_mut().find(|p| p.hash.starts_with(peer)) {
                    p.name = Some(name.to_string());
                    app.conversation.push_system(&format!(
                        "⬡ alias set: {} → {}",
                        &peer[..8.min(peer.len())],
                        name
                    ));
                } else {
                    app.conversation
                        .push_system(&format!("⬡ peer not found: {}", &peer[..12.min(peer.len())]));
                }
            }
        }

        "help" => {
            app.conversation.push_system(
                "⬡ commands:\n\n  \
                 :q, :quit        exit\n  \
                 :settings        toggle settings panel\n  \
                 :announce        broadcast mesh announce\n  \
                 :path <hash>     request route discovery\n  \
                 :probe <link_id> probe an active link\n  \
                 :link-open <h>   establish link\n  \
                 :link-close <id> close active link\n  \
                 :request <id> <path> [data]\n  \
                 :cancel <id>     cancel operation/request\n  \
                 :resource-cancel <hash>\n  \
                 :routes, :interfaces, :links\n  \
                 :requests, :resources\n  \
                 :exec <hash> <command> [args]\n  \
                 :block <hash>    block a peer\n  \
                 :unblock <hash>  unblock a peer\n  \
                 :alias <h> <n>   set peer display name\n  \
                 :connect <addr>  connect to daemon\n  \
                 :disconnect      disconnect from daemon\n  \
                 :help            show this message",
            );
        }

        other => {
            app.conversation.push_system(&format!("unknown command: {other}"));
        }
    }

    false
}

fn handle_search_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.focus = Focus::Sidebar;
        }
        KeyCode::Enter => {
            app.input_mode = InputMode::Normal;
            app.focus = Focus::Sidebar;
        }
        KeyCode::Char(c) => {
            if let InputMode::Search { ref mut query } = app.input_mode {
                query.push(c);
            }
        }
        KeyCode::Backspace => {
            if let InputMode::Search { ref mut query } = app.input_mode {
                query.pop();
            }
        }
        _ => {}
    }
    false
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    fn negotiated(capability: &str) -> App {
        let mut app = App::new();
        app.daemon_connected = true;
        app.connection_generation = Some(7);
        let mut active = styrene_ipc::types::ActiveCapabilitiesInfo::default();
        active.version = styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION;
        active.authorized_operations = vec![capability.into()];
        app.active_capabilities = Some(active);
        app
    }

    #[test]
    fn mutations_fail_closed_for_disconnected_unknown_stale_and_denied_sessions() {
        let mut app = App::new();
        assert!(app.mutation_availability("network.probe").unwrap_err().contains("disconnected"));

        app.daemon_connected = true;
        assert!(app.mutation_availability("network.probe").unwrap_err().contains("unknown"));

        app.connection_generation = Some(1);
        app.active_capabilities = Some(Default::default());
        assert!(app.mutation_availability("network.probe").unwrap_err().contains("stale"));

        let app = negotiated("network.announce");
        assert!(app.mutation_availability("network.probe").unwrap_err().contains("denied"));
        assert!(app.mutation_availability("network.announce").is_ok());
    }

    #[test]
    fn operator_fixture_catalog_exercises_each_universal_execution_gate() {
        use styrene_ipc::operator_fixtures::{
            OPERATOR_FIXTURE_OPERATIONS, OperatorFixtureState, operator_fixture_evidence,
        };

        for operation in OPERATOR_FIXTURE_OPERATIONS {
            let disconnected = App::new();
            assert!(
                disconnected
                    .mutation_availability(operation.capability)
                    .unwrap_err()
                    .contains("disconnected"),
                "{} disconnected fixture",
                operation.id
            );

            let mut stale = negotiated(operation.capability);
            stale.active_capabilities.as_mut().unwrap().version = 0;
            assert!(
                stale.mutation_availability(operation.capability).unwrap_err().contains("stale"),
                "{} stale fixture",
                operation.id
            );

            let denied = negotiated("fixture.unrelated");
            assert!(
                denied
                    .mutation_availability(operation.capability)
                    .unwrap_err()
                    .contains("permission denied"),
                "{} denied fixture",
                operation.id
            );

            let mut unsupported = negotiated(operation.capability);
            let mut degraded = styrene_ipc::types::DegradedCapabilityInfo::default();
            degraded.id = operation.capability.into();
            degraded.reason = "fixture: operation unsupported".into();
            unsupported.active_capabilities.as_mut().unwrap().degraded.push(degraded);
            assert!(
                unsupported
                    .mutation_availability(operation.capability)
                    .unwrap_err()
                    .contains("operation unsupported"),
                "{} unsupported fixture",
                operation.id
            );

            for state in [
                OperatorFixtureState::TimedOut,
                OperatorFixtureState::Cancelled,
                OperatorFixtureState::PartialFailure,
            ] {
                match operator_fixture_evidence(*operation, state) {
                    Some(evidence) => {
                        assert_eq!(evidence.source, styrene_ipc::types::ObservationSource::Fixture);
                        assert_eq!(evidence.connection_generation, 7);
                        assert!(evidence.terminal_outcome.is_some());
                        assert!(evidence.correlation_id.starts_with("fixture:"));
                    }
                    None => assert!(operation.not_applicable_reason(state).is_some()),
                }
            }
        }
    }

    #[test]
    fn mutation_surfaces_render_disabled_state_before_activation() {
        let source = include_str!("app.rs");
        for label in [
            "Page browsing disabled",
            "Terminal disabled",
            "Command disabled",
            "Chat input disabled",
        ] {
            assert!(source.contains(label), "missing pre-activation disabled state: {label}");
        }
    }

    #[test]
    fn disconnect_revokes_old_task_authority_and_connect_requests_a_fresh_session() {
        let mut app = negotiated("network.announce");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        app.connection_tx = Some(tx);

        assert!(!execute_command(&mut app, "disconnect"));
        assert!(matches!(rx.try_recv(), Ok(ConnectionControl::Disconnect)));
        assert!(!app.daemon_session_accepting_events);
        assert!(app.cmd_tx.is_none());

        let mut stale = styrene_ipc::types::DaemonStatusInfo::default();
        stale.connection_generation = Some(7);
        stale.rns_initialized = true;
        daemon::apply_event(&mut app, daemon::TuiEvent::Status(stale));
        assert_eq!(app.connection_generation, None);
        assert!(!app.rns_initialized, "stale poll task restored disconnected authority");

        assert!(!execute_command(&mut app, "connect /tmp/new-styrene.sock"));
        assert!(matches!(
            rx.try_recv(),
            Ok(ConnectionControl::Connect(path)) if path == std::path::Path::new("/tmp/new-styrene.sock")
        ));
        assert_eq!(app.connection_generation, None);

        app.daemon_session_accepting_events = true;
        app.daemon_connected = true;
        let mut fresh = styrene_ipc::types::DaemonStatusInfo::default();
        fresh.connection_generation = Some(8);
        fresh.rns_initialized = true;
        daemon::apply_event(&mut app, daemon::TuiEvent::Status(fresh));
        assert_eq!(app.connection_generation, Some(8));
        assert!(app.rns_initialized);
    }

    #[test]
    fn timeout_and_cancelled_outcomes_are_not_promoted_to_success() {
        let mut app = negotiated("network.probe");
        for (id, outcome) in [
            ("timeout", styrene_ipc::types::NetworkOperationOutcome::TimedOut),
            ("cancel", styrene_ipc::types::NetworkOperationOutcome::Cancelled),
        ] {
            let mut operation = styrene_ipc::types::NetworkOperationInfo::default();
            operation.operation_id = id.into();
            operation.kind = styrene_ipc::types::NetworkOperationKind::Probe;
            operation.outcome = Some(outcome);
            operation.observation.connection_generation = Some(7);
            crate::daemon::apply_event(
                &mut app,
                crate::daemon::TuiEvent::NetworkOperation(operation),
            );
        }
        assert_eq!(
            app.network_operations[0].outcome,
            Some(styrene_ipc::types::NetworkOperationOutcome::TimedOut)
        );
        assert_eq!(
            app.network_operations[1].outcome,
            Some(styrene_ipc::types::NetworkOperationOutcome::Cancelled)
        );
    }

    #[test]
    fn page_form_and_navigation_controls_emit_authoritative_daemon_commands() {
        let mut app = negotiated("page.browse");
        app.workspace = Workspace::Peers;
        app.peer_tab = app::PeerTab::Pages;
        app.focus = Focus::Main;
        app.selected_peer = Some("peer".into());
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        app.cmd_tx = Some(tx);
        let mut page = styrene_ipc::types::PageContent::default();
        page.navigation.session_id = "session".into();
        page.navigation.address = "/page/index.mu".into();
        page.navigation.can_back = true;
        let mut password = styrene_ipc::types::PageFormField::default();
        password.name = "password".into();
        password.kind = styrene_ipc::types::PageFormFieldKind::Password;
        page.fields.push(password);
        let mut link = styrene_ipc::types::PageLinkTarget::default();
        link.target = "next.mu".into();
        link.submitted_fields = vec!["password".into()];
        page.link_targets.push(link);
        let mut ordinary = styrene_ipc::types::PageLinkTarget::default();
        ordinary.target = "ordinary.mu".into();
        page.link_targets.push(ordinary);
        crate::daemon::apply_event(
            &mut app,
            crate::daemon::TuiEvent::PageLoaded {
                host: "local".into(),
                path: "/page/index.mu".into(),
                page: Box::new(page),
                generation: 7,
            },
        );

        handle_key(&mut app, KeyEvent::from(KeyCode::Char('e')));
        for character in "secret".chars() {
            handle_key(&mut app, KeyEvent::from(KeyCode::Char(character)));
        }
        assert!(!format!("{:?}", app.input_mode).contains("secret"));
        handle_key(&mut app, KeyEvent::from(KeyCode::Enter));
        handle_key(&mut app, KeyEvent::from(KeyCode::Enter));
        let command = rx.try_recv().expect("navigate command");
        assert_eq!(command.origin_generation, 7);
        assert_eq!(command.capability, "page.browse");
        let daemon::DaemonCmd::NavigatePage(request) = command.command else {
            panic!("unexpected page command")
        };
        assert_eq!(request.session_id.as_deref(), Some("session"));
        assert_eq!(request.submission.unwrap().values["password"], ["secret"]);

        handle_key(&mut app, KeyEvent::from(KeyCode::Char('b')));
        assert!(matches!(
            rx.try_recv().expect("back command").command,
            daemon::DaemonCmd::NavigatePage(request)
                if request.action == styrene_ipc::types::PageNavigationAction::Back
        ));

        app.page_link_selection = 1;
        handle_key(&mut app, KeyEvent::from(KeyCode::Enter));
        let daemon::DaemonCmd::NavigatePage(request) =
            rx.try_recv().expect("ordinary navigation").command
        else {
            panic!("unexpected ordinary page command")
        };
        assert!(request.submission.is_none(), "ordinary links must remain cacheable");
    }

    #[test]
    fn verified_page_host_opens_native_index_without_remote_listing() {
        let mut app = negotiated("page.browse");
        app.workspace = Workspace::Peers;
        app.peer_tab = app::PeerTab::Pages;
        app.focus = Focus::Main;
        app.selected_peer = Some("peer".into());
        let mut peer = crate::mesh_state::PeerRecord::new("peer".into(), None, 1);
        peer.native_page_host = true;
        app.peers.push(peer);
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        app.cmd_tx = Some(tx);

        handle_key(&mut app, KeyEvent::from(KeyCode::Enter));

        assert!(matches!(
            rx.try_recv().expect("native index browse").command,
            daemon::DaemonCmd::BrowsePage { host, path }
                if host == "peer" && path == "/page/index.mu"
        ));
    }

    #[test]
    fn page_close_waits_for_confirmation_and_save_uses_operator_path() {
        let mut app = negotiated("page.browse");
        app.workspace = Workspace::Peers;
        app.peer_tab = app::PeerTab::Pages;
        app.focus = Focus::Main;
        app.selected_peer = Some("peer".into());
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        app.cmd_tx = Some(tx);
        let mut page = styrene_ipc::types::PageContent::default();
        page.navigation.session_id = "session".into();
        page.navigation.address = "/page/index.mu".into();
        crate::daemon::apply_event(
            &mut app,
            crate::daemon::TuiEvent::PageLoaded {
                host: "local".into(),
                path: "/page/index.mu".into(),
                page: Box::new(page),
                generation: 7,
            },
        );

        handle_key(&mut app, KeyEvent::from(KeyCode::Char('x')));
        assert!(matches!(
            rx.try_recv().expect("close command").command,
            daemon::DaemonCmd::ClosePage { session_id } if session_id == "session"
        ));
        assert!(app.page_content.is_some(), "optimistic close discarded the page");
        crate::daemon::apply_event(
            &mut app,
            crate::daemon::TuiEvent::CommandResult {
                action: "close page".into(),
                success: false,
                detail: "link teardown failed".into(),
                generation: 7,
            },
        );
        assert!(app.page_content.is_some(), "failed close discarded the page");

        let mut download = styrene_ipc::types::FileDownloadInfo::default();
        download.download_id = "download".into();
        download.state = styrene_ipc::types::FileDownloadState::Completed;
        download.integrity_verified = true;
        app.page_download = Some(download);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('s')));
        for character in "/tmp/operator-selected.bin".chars() {
            handle_key(&mut app, KeyEvent::from(KeyCode::Char(character)));
        }
        handle_key(&mut app, KeyEvent::from(KeyCode::Enter));
        assert!(matches!(
            rx.try_recv().expect("save command").command,
            daemon::DaemonCmd::SaveFileDownload { download_id, destination }
                if download_id == "download" && destination == "/tmp/operator-selected.bin"
        ));

        crate::daemon::apply_event(
            &mut app,
            crate::daemon::TuiEvent::PageClosed { session_id: "session".into(), generation: 7 },
        );
        assert!(app.page_content.is_none());
        assert!(app.page_path.is_none());
    }

    fn app_with_open_page() -> (App, tokio::sync::mpsc::Receiver<daemon::QueuedDaemonCmd>) {
        let mut app = negotiated("page.browse");
        app.workspace = Workspace::Peers;
        app.peer_tab = app::PeerTab::Pages;
        app.focus = Focus::Main;
        app.selected_peer = Some("old-peer".into());
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        app.cmd_tx = Some(tx);
        let mut page = styrene_ipc::types::PageContent::default();
        page.navigation.session_id = "old-session".into();
        page.navigation.address = "/page/index.mu".into();
        app.page_content = Some(page);
        app.page_path = Some("/page/index.mu".into());
        app.page_field_values.insert("password".into(), vec!["secret".into()]);
        app.page_download = Some(styrene_ipc::types::FileDownloadInfo::default());
        (app, rx)
    }

    fn assert_close_requested(
        app: &App,
        rx: &mut tokio::sync::mpsc::Receiver<daemon::QueuedDaemonCmd>,
    ) {
        assert!(matches!(
            rx.try_recv().expect("authoritative close request").command,
            daemon::DaemonCmd::ClosePage { session_id } if session_id == "old-session"
        ));
        assert!(app.page_content.is_some(), "transition cleared before PageClosed");
        assert!(app.page_download.is_some(), "download cleared before PageClosed");
    }

    #[test]
    fn page_exit_transitions_wait_for_authoritative_close() {
        let (mut app, mut rx) = app_with_open_page();
        app.dispatch(Action::Back);
        assert_close_requested(&app, &mut rx);
        crate::daemon::apply_event(
            &mut app,
            crate::daemon::TuiEvent::PageClosed { session_id: "old-session".into(), generation: 7 },
        );
        assert!(app.page_content.is_none());

        let (mut app, mut rx) = app_with_open_page();
        app.set_peer_tab(app::PeerTab::Chat);
        assert_close_requested(&app, &mut rx);
        assert_eq!(app.peer_tab, app::PeerTab::Pages);
        crate::daemon::apply_event(
            &mut app,
            crate::daemon::TuiEvent::PageClosed { session_id: "old-session".into(), generation: 7 },
        );
        assert_eq!(app.peer_tab, app::PeerTab::Chat);

        let (mut app, mut rx) = app_with_open_page();
        app.select_peer("new-peer".into());
        assert_close_requested(&app, &mut rx);
        assert_eq!(app.selected_peer.as_deref(), Some("old-peer"));
        crate::daemon::apply_event(
            &mut app,
            crate::daemon::TuiEvent::PageClosed { session_id: "old-session".into(), generation: 7 },
        );
        assert_eq!(app.selected_peer.as_deref(), Some("new-peer"));

        let (mut app, mut rx) = app_with_open_page();
        app.set_workspace(Workspace::Home);
        assert_close_requested(&app, &mut rx);
        assert_eq!(app.workspace, Workspace::Peers);
        crate::daemon::apply_event(
            &mut app,
            crate::daemon::TuiEvent::PageClosed { session_id: "old-session".into(), generation: 7 },
        );
        assert_eq!(app.workspace, Workspace::Home);
    }

    #[test]
    fn physical_disconnect_discards_owned_page_state_before_reconnect() {
        let (mut app, mut rx) = app_with_open_page();
        crate::daemon::apply_event(
            &mut app,
            crate::daemon::TuiEvent::Disconnected("socket closed".into()),
        );
        assert!(app.page_content.is_none());
        assert!(app.page_path.is_none());
        assert!(app.page_field_values.is_empty());
        assert!(app.page_download.is_none());

        let mut status = styrene_ipc::types::DaemonStatusInfo::default();
        status.connection_generation = Some(8);
        let mut capabilities = styrene_ipc::types::ActiveCapabilitiesInfo::default();
        capabilities.version = styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION;
        capabilities.authorized_operations = vec!["page.browse".into()];
        status.active_capabilities = Some(capabilities);
        crate::daemon::apply_event(&mut app, crate::daemon::TuiEvent::Status(status));
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('b')));
        assert!(rx.try_recv().is_err(), "old page session was reused after reconnect");
    }
}
