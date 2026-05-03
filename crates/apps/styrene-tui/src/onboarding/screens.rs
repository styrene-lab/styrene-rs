//! Wizard screen rendering — ratatui draw functions for each wizard step.
//!
//! All screens render inside a centered content block (max 70 cols wide).
//! Progress dots at top show the current step position.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::tui::theme::Theme;

use super::setup::{DaemonMode, IdentitySource};
use super::wizard::{WizardState, WizardStep};

/// Maximum width for the wizard content area.
const MAX_WIDTH: u16 = 90;
/// Minimum horizontal margin on each side.
const H_MARGIN: u16 = 4;
/// Top padding before content.
const TOP_PAD: u16 = 3;

/// Draw the current wizard step.
pub fn draw(state: &WizardState, f: &mut Frame, theme: &dyn Theme) {
    let area = f.area();

    // Full-screen background
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(Style::default().bg(theme.bg())), area);

    // Horizontally centered, top-aligned with padding — uses up to 80% of width
    let max_w = MAX_WIDTH.min(area.width.saturating_sub(H_MARGIN * 2));
    let x = area.x + (area.width.saturating_sub(max_w)) / 2;

    // Progress dots — pinned near the top
    let progress = progress_line(state, theme);
    let dots_y = area.y + TOP_PAD;
    if dots_y < area.height {
        let dots_area = Rect { x, y: dots_y, width: max_w, height: 1 };
        f.render_widget(Paragraph::new(progress).alignment(Alignment::Center), dots_area);
    }

    // Body — starts below the progress dots with a gap
    let body_y = dots_y + 2;
    let body_h = area.height.saturating_sub(body_y + 3); // leave room for nav hint
    let body = Rect { x, y: body_y, width: max_w, height: body_h };

    match state.step {
        WizardStep::Welcome => draw_welcome(state, f, body, theme),
        WizardStep::Identity => draw_identity(state, f, body, theme),
        WizardStep::Profile => draw_profile(state, f, body, theme),
        WizardStep::Network => draw_network(state, f, body, theme),
        WizardStep::ImportContacts => draw_import_contacts(state, f, body, theme),
        WizardStep::DaemonStart => draw_daemon_start(state, f, body, theme),
        WizardStep::Summary => draw_summary(state, f, body, theme),
    }

    // Navigation hint — pinned to bottom
    let nav = nav_hint(state, theme);
    let nav_y = area.height.saturating_sub(1);
    if nav_y > body_y {
        let nav_area = Rect { x, y: nav_y, width: max_w, height: 1 };
        f.render_widget(Paragraph::new(nav).alignment(Alignment::Center), nav_area);
    }
}

// ─── Individual screens ─────────────────────────────────────────────────────

fn draw_welcome(state: &WizardState, f: &mut Frame, area: Rect, theme: &dyn Theme) {
    let mut lines: Vec<Line> = Vec::new();

    // Compact sigil
    lines.push(Line::from(Span::styled("    /\\ /\\", theme.style_accent())));
    lines.push(Line::from(Span::styled("   | X | X |    S T Y R E N E", theme.style_accent())));
    lines.push(Line::from(Span::styled(
        "    \\/ \\/     mesh communications",
        theme.style_accent(),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(""));

    // Environment summary
    lines.push(Line::from(Span::styled("Detected environment:", theme.style_accent())));
    lines.push(Line::from(""));

    for (found, description) in state.env.summary_lines() {
        let icon = if found { "  [x] " } else { "  [ ] " };
        let style = if found { theme.style_fg() } else { theme.style_dim() };
        lines.push(Line::from(Span::styled(format!("{icon}{description}"), style)));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(""));

    // Continue button
    let btn_style = if state.focus == 0 {
        Style::default().fg(theme.bg()).bg(theme.accent()).add_modifier(Modifier::BOLD)
    } else {
        theme.style_accent()
    };
    lines.push(Line::from(Span::styled("  [ Continue → ]  ", btn_style)));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_identity(state: &WizardState, f: &mut Frame, area: Rect, theme: &dyn Theme) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled("Identity", theme.style_heading())));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Your identity is a cryptographic keypair that uniquely identifies",
        theme.style_muted(),
    )));
    lines.push(Line::from(Span::styled("your node on the mesh network.", theme.style_muted())));
    lines.push(Line::from(""));

    // Radio: Create new
    let create_selected = matches!(state.identity_source, IdentitySource::CreateNew);
    let radio_new =
        radio_line("Create a new Styrene identity", create_selected, state.focus == 0, theme);
    lines.push(radio_new);

    // Radio: Import from Reticulum (only if detected)
    if let Some(ref rns) = state.env.reticulum {
        lines.push(Line::from(""));
        let import_selected = matches!(state.identity_source, IdentitySource::ImportReticulum(_));
        let label = if let Some(ref hash) = rns.identity_hash {
            format!("Import from Reticulum identity ({hash})")
        } else {
            "Import from Reticulum identity".into()
        };
        let radio_import = radio_line(&label, import_selected, state.focus == 1, theme);
        lines.push(radio_import);

        lines.push(Line::from(Span::styled(
            format!("        {}", rns.identity_path.display()),
            theme.style_dim(),
        )));
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_profile(state: &WizardState, f: &mut Frame, area: Rect, theme: &dyn Theme) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled("Profile", theme.style_heading())));
    lines.push(Line::from(""));

    // Display name input
    let name_focused = state.focus == 0;
    let label_style = if name_focused { theme.style_accent() } else { theme.style_fg() };
    lines.push(Line::from(Span::styled("Display name:", label_style)));

    let input_content = if state.display_name.is_empty() && !name_focused {
        Span::styled("(optional)", theme.style_dim())
    } else {
        let mut display = state.display_name.clone();
        if name_focused {
            display.push('_');
        }
        Span::styled(display, theme.style_user_input())
    };

    let border = if name_focused { theme.style_accent() } else { theme.style_border_dim() };
    lines.push(Line::from(vec![
        Span::styled("  [ ", border),
        input_content,
        Span::styled(" ]", border),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(""));

    // Node role
    lines.push(Line::from(Span::styled("Node role:", theme.style_fg())));
    lines.push(Line::from(""));

    let roles = [
        (
            styrened::config::NodeRole::FullNode,
            "Full Node",
            "Routes packets, maintains announce tables",
        ),
        (styrened::config::NodeRole::PropagationClient, "Client", "Connects to a hub, no routing"),
        (styrened::config::NodeRole::Hub, "Hub", "Stores and relays messages for clients"),
    ];

    for (i, (role, name, desc)) in roles.iter().enumerate() {
        let selected = state.node_role == *role;
        let focused = state.focus == i + 1;
        let radio = radio_line(&format!("{name} — {desc}"), selected, focused, theme);
        lines.push(radio);
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_network(state: &WizardState, f: &mut Frame, area: Rect, theme: &dyn Theme) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled("Network", theme.style_heading())));
    lines.push(Line::from(""));

    // Auto-discover checkbox
    let check =
        checkbox_line("Auto-discover LAN peers", state.auto_discover, state.focus == 0, theme);
    lines.push(check);
    lines.push(Line::from(Span::styled(
        "      TCP + UDP broadcast on local network",
        theme.style_dim(),
    )));

    // Imported Reticulum interfaces
    if !state.imported_interfaces.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Import from Reticulum config:", theme.style_accent())));
        lines.push(Line::from(""));

        for (i, (iface, selected)) in state.imported_interfaces.iter().enumerate() {
            let label = format!(
                "{} — {}{}",
                iface.name,
                iface.host.as_deref().unwrap_or("*"),
                iface.port.map(|p| format!(":{p}")).unwrap_or_default()
            );
            let focused = state.focus == i + 1;
            lines.push(checkbox_line(&label, *selected, focused, theme));
        }
    }

    // Hub address input
    lines.push(Line::from(""));
    let hub_idx = 1 + state.imported_interfaces.len();
    let hub_focused = state.focus == hub_idx;
    let label_style = if hub_focused { theme.style_accent() } else { theme.style_fg() };
    lines.push(Line::from(Span::styled("Hub address (optional):", label_style)));

    let hub_content = if state.hub_address.is_empty() && !hub_focused {
        Span::styled("host:port", theme.style_dim())
    } else {
        let mut display = state.hub_address.clone();
        if hub_focused {
            display.push('_');
        }
        Span::styled(display, theme.style_user_input())
    };

    let border = if hub_focused { theme.style_accent() } else { theme.style_border_dim() };
    lines.push(Line::from(vec![
        Span::styled("  [ ", border),
        hub_content,
        Span::styled(" ]", border),
    ]));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_import_contacts(state: &WizardState, f: &mut Frame, area: Rect, theme: &dyn Theme) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled("Import Contacts", theme.style_heading())));
    lines.push(Line::from(""));

    let count = state.imported_contacts.len();
    let noun = if count == 1 { "contact" } else { "contacts" };
    lines.push(Line::from(Span::styled(
        format!("Found {count} {noun} to import:"),
        theme.style_muted(),
    )));
    lines.push(Line::from(""));

    // Select all toggle
    let all_selected = state.imported_contacts.iter().all(|(_, _, s)| *s);
    lines.push(checkbox_line("Select all", all_selected, state.focus == 0, theme));
    lines.push(Line::from(""));

    // Individual contacts
    for (i, (hash, name, selected)) in state.imported_contacts.iter().enumerate() {
        let label = if name.is_empty() { hash.clone() } else { format!("{name} ({hash})") };
        let focused = state.focus == i + 1;
        lines.push(checkbox_line(&label, *selected, focused, theme));
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_daemon_start(state: &WizardState, f: &mut Frame, area: Rect, theme: &dyn Theme) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled("Daemon", theme.style_heading())));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "The Styrene daemon manages your mesh connection.",
        theme.style_muted(),
    )));
    lines.push(Line::from(""));

    let modes = [
        (DaemonMode::Embedded, "Embedded", "Run inside this TUI process"),
        (DaemonMode::Background, "Background", "Start as a separate process"),
        (DaemonMode::ConnectExisting, "Connect", "Use an already-running daemon"),
    ];

    for (i, (mode, name, desc)) in modes.iter().enumerate() {
        let selected = state.daemon_mode == *mode;
        let focused = state.focus == i;
        lines.push(radio_line(&format!("{name} — {desc}"), selected, focused, theme));
    }

    lines.push(Line::from(""));
    if state.daemon_mode == DaemonMode::Embedded {
        lines.push(Line::from(Span::styled(
            "  The daemon will start and stop with this TUI.",
            theme.style_dim(),
        )));
    } else if state.daemon_mode == DaemonMode::Background {
        lines.push(Line::from(Span::styled(
            "  The daemon will continue running after this TUI exits.",
            theme.style_dim(),
        )));
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_summary(state: &WizardState, f: &mut Frame, area: Rect, theme: &dyn Theme) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled("Summary", theme.style_heading())));
    lines.push(Line::from(""));

    // Identity
    let identity_desc = match &state.identity_source {
        IdentitySource::CreateNew => "Create new identity".to_string(),
        IdentitySource::ImportReticulum(p) => format!("Import from {}", p.display()),
    };
    lines.push(summary_row("Identity", &identity_desc, theme));

    // Display name
    if !state.display_name.is_empty() {
        lines.push(summary_row("Name", &state.display_name, theme));
    }

    // Role
    lines.push(summary_row("Role", &state.node_role.to_string(), theme));

    // Network
    if state.auto_discover {
        lines.push(summary_row("Network", "Auto-discover LAN", theme));
    }
    let iface_count = state.imported_interfaces.iter().filter(|(_, s)| *s).count();
    if iface_count > 0 {
        lines.push(summary_row(
            "Imported",
            &format!("{iface_count} interface(s) from Reticulum"),
            theme,
        ));
    }
    if !state.hub_address.is_empty() {
        lines.push(summary_row("Hub", &state.hub_address, theme));
    }

    // Contacts
    let contact_count = state.imported_contacts.iter().filter(|(_, _, s)| *s).count();
    if contact_count > 0 {
        lines.push(summary_row("Contacts", &format!("{contact_count} to import"), theme));
    }

    // Daemon
    let daemon_desc = match state.daemon_mode {
        DaemonMode::Embedded => "Embedded (in-process)",
        DaemonMode::Background => "Background service",
        DaemonMode::ConnectExisting => "Connect to existing",
    };
    lines.push(summary_row("Daemon", daemon_desc, theme));

    lines.push(Line::from(""));
    lines.push(Line::from(""));

    // Launch button
    let btn_style = if state.focus == 0 {
        Style::default().fg(theme.bg()).bg(theme.accent()).add_modifier(Modifier::BOLD)
    } else {
        theme.style_accent()
    };
    lines.push(Line::from(Span::styled("  [ Launch Styrene → ]  ", btn_style)));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

// ─── Shared primitives ──────────────────────────────────────────────────────

fn radio_line<'a>(label: &str, selected: bool, focused: bool, theme: &dyn Theme) -> Line<'a> {
    let bullet = if selected { "(o) " } else { "( ) " };
    let style = if focused {
        theme.style_accent_bold()
    } else if selected {
        theme.style_fg()
    } else {
        theme.style_muted()
    };
    Line::from(Span::styled(format!("  {bullet}{label}"), style))
}

fn checkbox_line<'a>(label: &str, checked: bool, focused: bool, theme: &dyn Theme) -> Line<'a> {
    let box_str = if checked { "[x] " } else { "[ ] " };
    let style = if focused {
        theme.style_accent_bold()
    } else if checked {
        theme.style_fg()
    } else {
        theme.style_muted()
    };
    Line::from(Span::styled(format!("  {box_str}{label}"), style))
}

fn summary_row<'a>(label: &str, value: &str, theme: &dyn Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {label:<12} "), theme.style_muted()),
        Span::styled(value.to_string(), theme.style_fg()),
    ])
}

fn progress_line<'a>(state: &WizardState, theme: &dyn Theme) -> Line<'a> {
    let total = state.step_count();
    let current = state.step_number() - 1;
    let dots: Vec<Span> = (0..total)
        .map(|i| {
            if i == current {
                Span::styled(" ● ", theme.style_accent_bold())
            } else {
                Span::styled(" ○ ", theme.style_dim())
            }
        })
        .collect();
    Line::from(dots)
}

fn nav_hint<'a>(state: &WizardState, theme: &dyn Theme) -> Line<'a> {
    let hint = match state.step {
        WizardStep::Welcome => "Esc quit  ·  Enter continue",
        WizardStep::Summary => "Esc back  ·  Enter launch",
        _ => "Esc back  ·  Tab next field  ·  Space toggle  ·  Enter continue",
    };
    Line::from(Span::styled(hint, theme.style_dim()))
}

/// Center a rect horizontally and vertically within `outer`.
fn centered_rect(outer: Rect, max_w: u16, max_h: u16) -> Rect {
    let w = max_w.min(outer.width);
    let h = max_h.min(outer.height);
    let x = outer.x + (outer.width.saturating_sub(w)) / 2;
    let y = outer.y + (outer.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}
