//! Styrene TUI — three-workspace terminal UI for the Styrene mesh daemon.
//!
//! Workspaces:
//!   - Home:     Activity feed, node status, signal waveform
//!   - Peers:    Peer browser with Status/Chat/Pages/Terminal/Commands tabs
//!   - Messages: Conversation threads
//!
//! Run: `cargo run -p styrene-tui`

mod app;
mod daemon;
mod mesh_state;
mod micron_widget;
mod onboarding;
mod tui;

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

use app::{App, Focus, InputMode, Workspace};
use tui::splash;

#[tokio::main]
async fn main() -> Result<()> {
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

    let result = run(&mut terminal).await;

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    result
}

async fn run(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
) -> Result<()> {
    let mut app = App::new();

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
    let env = onboarding::detect::scan_environment();
    let daemon_mode = if env.needs_wizard() {
        let mut wizard = onboarding::WizardState::new(env);
        loop {
            terminal.draw(|f| wizard.draw(f, app.theme.as_ref()))?;
            if event::poll(Duration::from_millis(16))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        match wizard.handle_key(k) {
                            onboarding::WizardAction::Complete(result) => {
                                if let Err(e) = result.apply() {
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
        onboarding::load_tui_prefs().daemon_mode_or_default()
    };

    // ── Welcome + effects ────────────────────────────────────────────────────
    {
        let t = app.theme.as_ref();
        app.effects.queue_startup(t);
    }
    app.push_welcome();

    // ── Daemon connection (mode-aware) ──────────────────────────────────────
    let (daemon_tx, mut daemon_rx) = tokio::sync::mpsc::channel::<daemon::TuiEvent>(128);

    let connect_result = match daemon_mode {
        onboarding::setup::DaemonMode::Embedded => {
            // TODO: embedded daemon bootstrap (Step 6)
            // For now, fall back to standard socket connect
            app.conversation.push_system("⬡ embedded daemon not yet implemented — trying socket");
            daemon::connect(None).await
        }
        onboarding::setup::DaemonMode::Background => {
            // Try to spawn styrened as a child process, then connect
            match std::process::Command::new("styrened")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(_child) => {
                    app.conversation.push_system("⬡ starting daemon...");
                    // Give the daemon a moment to create its socket
                    let mut connected = Err("timeout".to_string());
                    for _ in 0..30 {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        match daemon::connect(None).await {
                            Ok(result) => {
                                connected = Ok(result);
                                break;
                            }
                            Err(_) => continue,
                        }
                    }
                    connected
                }
                Err(e) => {
                    app.conversation
                        .push_system(&format!("⬡ failed to start daemon: {e} — trying socket"));
                    daemon::connect(None).await
                }
            }
        }
        onboarding::setup::DaemonMode::ConnectExisting => daemon::connect(None).await,
    };

    match connect_result {
        Ok((handle, mut event_rx)) => {
            app.daemon_connected = true;
            app.conversation.push_system("⬡ daemon connected");
            let handle = Arc::new(Mutex::new(handle));
            let tx_clone = daemon_tx.clone();
            tokio::spawn(async move {
                while let Some(ev) = event_rx.recv().await {
                    if tx_clone.send(ev).await.is_err() {
                        break;
                    }
                }
            });
            daemon::spawn_poll_task(handle, daemon_tx, 10);
        }
        Err(e) => {
            app.conversation.push_system(&format!("⬡ daemon unavailable ({e}) — demo mode"));
        }
    }

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

    // ── Global keys (Normal mode) ───────────────────────────────────────────
    match (key.code, key.modifiers) {
        // Quit
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            let now = std::time::Instant::now();
            if let Some(last) = app.last_ctrl_c {
                if now.duration_since(last) < Duration::from_secs(1) {
                    return true;
                }
            }
            app.last_ctrl_c = Some(now);
            app.conversation.push_system("Press Ctrl+C again to quit");
        }
        (KeyCode::Char('q'), KeyModifiers::NONE) if app.focus != Focus::Input => {
            return true;
        }

        // Workspace navigation
        (KeyCode::Tab, _) => app.next_workspace(),
        (KeyCode::BackTab, _) => app.prev_workspace(),
        (KeyCode::Char('1'), _) => app.set_workspace(Workspace::Home),
        (KeyCode::Char('2'), _) => app.set_workspace(Workspace::Peers),
        (KeyCode::Char('3'), _) => app.set_workspace(Workspace::Messages),

        // Mode triggers
        (KeyCode::Char(':'), _) => {
            app.input_mode = InputMode::Command { buffer: String::new() };
            app.focus = Focus::Input;
        }
        (KeyCode::Char('/'), _) => {
            app.input_mode = InputMode::Search { query: String::new() };
            app.focus = Focus::Input;
        }
        (KeyCode::Char('i'), _) => {
            app.input_mode = InputMode::Compose;
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
            // Select peer from sidebar
            if let Some(peer) = app.peers.get(app.sidebar_selection) {
                let hash = peer.hash.clone();
                match app.workspace {
                    Workspace::Peers => {
                        app.selected_peer = Some(hash);
                        app.focus = Focus::Main;
                    }
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
            app.active_conversation_mut().scroll_down(3);
        }
        (KeyCode::Char('k') | KeyCode::Up, _) if app.focus == Focus::Main => {
            app.active_conversation_mut().scroll_up(3);
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
                app.conversation
                    .push_system(&format!("⬡ connect to {arg} — not yet wired (daemon reconnect TODO)"));
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

        "help" => {
            app.conversation.push_system(
                "⬡ commands:\n\n  \
                 :q, :quit        exit\n  \
                 :connect <addr>  connect to daemon\n  \
                 :disconnect      disconnect from daemon\n  \
                 :help            show this message",
            );
        }

        other => {
            app.conversation
                .push_system(&format!("unknown command: {other}"));
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
