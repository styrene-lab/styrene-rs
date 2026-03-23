//! Styrene TUI — Ratatui terminal UI for the Styrene mesh daemon.
//!
//! Architecture:
//!   - `tui/`       — all ratatui rendering code (theme, widgets, segments)
//!   - `app.rs`     — AppState and event handling
//!   - `daemon.rs`  — RPC channel bridge (feeds DaemonEvents into App)
//!
//! Run: `cargo run -p styrene-tui`
//! Quit: `q`, `Esc`, or Ctrl+C twice

mod app;
mod daemon;
mod micron_widget; // Micron markup widget — used by Config tab in Phase 4
mod tui;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::event::{EnableMouseCapture, DisableMouseCapture};
use std::io;
use std::time::Duration;

use app::App;
use tui::splash;

fn main() -> Result<()> {
    // Seed spinner from process start
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

    // Panic hook that restores terminal
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));

    let result = run(&mut terminal);

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    result
}

fn run(terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();

    // Splash
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

    // Queue startup reveal effects, populate welcome message
    {
        let t = app.theme.as_ref();
        app.effects.queue_startup(t);
    }
    app.push_welcome();

    // Main event loop — 60fps
    loop {
        terminal.draw(|f| app.draw(f))?;

        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Mouse(m) => {
                    use crossterm::event::MouseEventKind;
                    match m.kind {
                        MouseEventKind::ScrollUp => app.conversation.scroll_up(3),
                        MouseEventKind::ScrollDown => app.conversation.scroll_down(3),
                        _ => {}
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if handle_key(&mut app, key) {
                        break; // quit
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
        (KeyCode::Char('q'), KeyModifiers::NONE) if !app.composing => {
            return true;
        }

        // Scroll
        (KeyCode::PageUp, _) => app.conversation.scroll_up(20),
        (KeyCode::PageDown, _) => app.conversation.scroll_down(20),
        (KeyCode::Up, KeyModifiers::SHIFT) => app.conversation.scroll_up(3),
        (KeyCode::Down, KeyModifiers::SHIFT) => app.conversation.scroll_down(3),
        (KeyCode::Up, KeyModifiers::NONE) if !app.composing => app.conversation.scroll_up(3),
        (KeyCode::Down, KeyModifiers::NONE) if !app.composing => app.conversation.scroll_down(3),

        // Tab navigation
        (KeyCode::Tab, _) => app.next_tab(),
        (KeyCode::BackTab, _) => app.prev_tab(),

        // Compose mode: Enter starts, Esc cancels, second Enter submits
        (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
            app.composing = true;
            app.conversation.push_system("Compose mode — type your message, Enter to send, Esc to cancel");
        }
        (KeyCode::Esc, _) if app.composing => {
            app.composing = false;
            app.editor.clear_line();
        }
        (KeyCode::Enter, _) if app.composing => {
            let text = app.editor.take_text();
            if !text.is_empty() {
                app.handle_compose_submit(text);
            }
            app.composing = false;
        }
        // Editor input while composing
        (KeyCode::Char(c), mods) if app.composing && !mods.contains(KeyModifiers::CONTROL) => {
            app.editor.insert(c);
        }
        (KeyCode::Backspace, _) if app.composing => { app.editor.backspace(); }
        (KeyCode::Char('w'), KeyModifiers::CONTROL) if app.composing => {
            app.editor.delete_word_backward();
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) if app.composing => {
            app.editor.clear_line();
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) if app.composing => {
            app.editor.kill_to_end();
        }

        // Quick demo triggers
        (KeyCode::Char('r'), KeyModifiers::NONE) => app.demo_announce(),
        (KeyCode::Char('l'), KeyModifiers::NONE) => app.demo_link(),

        _ => {}
    }
    false
}
