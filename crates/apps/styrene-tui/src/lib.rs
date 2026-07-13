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
mod daemon;
mod ghost;
mod ghost_preferences;
mod mesh_state;
mod micron_widget;
mod onboarding;
mod runtime;
mod tui;

pub use onboarding::paths::{StyrenePaths, TuiOptions};
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
pub async fn run(mut options: TuiOptions) -> Result<()> {
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

async fn connect_with_retry(
    socket: &std::path::Path,
) -> Result<(daemon::DaemonHandle, tokio::sync::mpsc::Receiver<daemon::TuiEvent>), String> {
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
            if event::poll(interval)? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press
                        && (splash.ready_to_dismiss()
                            || start.elapsed() > Duration::from_millis(300))
                    {
                        break;
                    }
                }
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
            if event::poll(Duration::from_millis(16))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
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
    let (daemon_tx, mut daemon_rx) = tokio::sync::mpsc::channel::<daemon::TuiEvent>(128);

    let embedded_daemon = if daemon_mode == onboarding::setup::DaemonMode::Embedded {
        match styrened::daemon::start(styrened::daemon::DaemonConfig2 {
            db: Some(options.paths.data_dir.join("messages.db")),
            config: options.paths.config_path().exists().then(|| options.paths.config_path()),
            identity: (!options.runtime_profile.is_ephemeral())
                .then(|| options.paths.identity_path()),
            socket: Some(options.paths.daemon_socket.clone()),
            ephemeral: options.runtime_profile.is_ephemeral(),
        })
        .await
        {
            Ok(handle) => {
                app.conversation.push_system("⬡ embedded runtime ready");
                Some(handle)
            }
            Err(error) => {
                app.conversation.push_system(&format!("⬡ embedded runtime failed: {error}"));
                None
            }
        }
    } else {
        None
    };

    let connect_result = match daemon_mode {
        onboarding::setup::DaemonMode::Embedded => {
            if embedded_daemon.is_none() {
                Err("embedded runtime failed to start".into())
            } else {
                connect_with_retry(&options.paths.daemon_socket).await
            }
        }
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

    match connect_result {
        Ok((handle, mut event_rx)) => {
            app.daemon_connected = true;
            app.conversation.push_system("⬡ daemon connected");
            let handle = Arc::new(Mutex::new(handle));

            // Forward daemon events to TUI event channel
            let tx_clone = daemon_tx.clone();
            tokio::spawn(async move {
                while let Some(ev) = event_rx.recv().await {
                    if tx_clone.send(ev).await.is_err() {
                        break;
                    }
                }
            });

            // Command queue: key handlers queue commands, executor dispatches async
            let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<daemon::DaemonCmd>(64);
            app.cmd_tx = Some(cmd_tx);
            daemon::spawn_command_executor(handle.clone(), cmd_rx, daemon_tx.clone());

            // Periodic poll task (status + devices every 10s)
            daemon::spawn_poll_task(handle, daemon_tx, 10);
        }
        Err(e) => {
            app.conversation.push_system(&format!("⬡ daemon unavailable ({e}) — demo mode"));
        }
    }

    // Keep the embedded runtime alive for the full terminal session.
    let _embedded_daemon = embedded_daemon;

    // ── Main event loop — 60fps ──────────────────────────────────────────────
    loop {
        // Drain daemon events
        loop {
            match daemon_rx.try_recv() {
                Ok(ev) => daemon::apply_event(&mut app, ev),
                Err(_) => break,
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
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if handle_key(&mut app, key) {
                        break;
                    }
                }
                _ => {}
            }
        }

        app.tick();
    }

    Ok(())
}

/// Returns true if the app should quit.
fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    // ── Input mode routing ──────────────────────────────────────────────────
    match &app.input_mode {
        InputMode::Compose => return handle_compose_key(app, key),
        InputMode::Command { .. } => return handle_command_key(app, key),
        InputMode::Search { .. } => return handle_search_key(app, key),
        InputMode::Normal => {}
    }

    if let Some(action) = action_for_key(app, key) {
        if action == Action::Quit {
            let now = std::time::Instant::now();
            if let Some(last) = app.last_ctrl_c {
                if now.duration_since(last) < Duration::from_secs(1) {
                    return true;
                }
            }
            app.last_ctrl_c = Some(now);
            app.conversation.push_system("Press Ctrl+C again to quit");
            return false;
        }
        return app.dispatch(action);
    }

    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::NONE) if app.focus != Focus::Input => {
            return true;
        }

        // Workspace navigation accelerators
        (KeyCode::Char('1'), _) => app.set_workspace(Workspace::Home),
        (KeyCode::Char('2'), _) => app.set_workspace(Workspace::Peers),
        (KeyCode::Char('3'), _) => app.set_workspace(Workspace::Messages),

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
                        app.selected_peer = Some(hash);
                        app.set_workspace(Workspace::Peers);
                        app.focus = Focus::Main;
                    }
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
                    app::CommandAction::RemoteExec => {
                        // TODO: prompt for command input. For now, run `uptime`.
                        app.send_daemon_cmd(DaemonCmd::Exec {
                            dest_hash: peer_hash,
                            command: "uptime".into(),
                            args: vec![],
                        });
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

        // Browse pages in Pages tab
        (KeyCode::Enter, _)
            if app.focus == Focus::Main
                && app.workspace == Workspace::Peers
                && app.peer_tab == app::PeerTab::Pages =>
        {
            if let Some(peer_hash) = app.selected_peer.clone() {
                if app.page_source.is_some() {
                    app.page_source = None;
                    app.page_path = None;
                } else if app.page_index.is_empty() {
                    app.send_daemon_cmd(daemon::DaemonCmd::ListPages { host: peer_hash });
                    app.conversation.push_system("⬡ loading page index from peer...");
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
                        app.selected_peer = None;
                        app.selected_conversation = None;
                    }
                }
            }
        }

        // Peer tab switching (in Peers workspace)
        (KeyCode::Char(n @ '4'..='5'), _) if app.workspace == Workspace::Peers => {
            let idx = (n as u8 - b'1') as usize;
            if let Some(tab) = app::PeerTab::ALL.get(idx) {
                app.peer_tab = *tab;
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

fn handle_compose_key(app: &mut App, key: KeyEvent) -> bool {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => {
            app.input_mode = InputMode::Normal;
            app.focus = Focus::Sidebar;
            app.editor.clear_line();
        }
        (KeyCode::Enter, _) => {
            let text = app.editor.take_text();
            if !text.is_empty() {
                app.handle_compose_submit(text);
            }
            app.input_mode = InputMode::Normal;
            app.focus = Focus::Sidebar;
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
            } else {
                app.conversation.push_system(&format!(
                    "⬡ connect to {arg} — not yet wired (daemon reconnect TODO)"
                ));
            }
        }

        "disconnect" => {
            if app.daemon_connected {
                app.daemon_connected = false;
                app.rns_initialized = false;
                app.transport_active = false;
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
            if app.daemon_connected {
                app.send_daemon_cmd(daemon::DaemonCmd::Announce);
                app.conversation.push_system("⬡ mesh announce queued");
            } else {
                app.conversation.push_system("⬡ not connected — cannot announce");
            }
        }

        "block" => {
            if arg.is_empty() {
                app.conversation.push_system("usage: :block <identity_hash>");
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
