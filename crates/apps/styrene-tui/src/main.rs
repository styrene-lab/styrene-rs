//! Styrene TUI — Ratatui terminal UI spike
//!
//! Validates:
//! 1. Protocol crates compile into a Ratatui app
//! 2. Shared state types (IdentityInfo, MeshState) render in terminal
//! 3. Event loop + keyboard navigation pattern
//!
//! Run: `cargo run -p styrene-tui`
//! Quit: `q` or `Esc`

mod micron_widget;
mod state;
mod ui;

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;

use state::AppState;

fn main() -> Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut state = AppState::new();

    loop {
        terminal.draw(|frame| ui::render(frame, &state))?;

        // Poll for events with 250ms tick rate
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => state.regenerate_identity(),
                    KeyCode::Tab => state.next_tab(),
                    KeyCode::BackTab => state.prev_tab(),
                    _ => {}
                }
            }
        }
    }
}
