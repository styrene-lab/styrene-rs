//! TUI Theme — Alpharius-derived color system for the Styrene mesh UI.
//!
//! Trait-based so the same rendering code works against any theme.
//! The `StyreneTheme` default uses the navy → teal → amber ramp from
//! the Alpharius system, adjusted for a darker mesh-terminal aesthetic.

use ratatui::style::{Color, Modifier, Style};

/// Semantic color slots for the Styrene TUI.
pub trait Theme: Send + Sync {
    // ─── Core palette ─────────────────────────────────────────────
    fn bg(&self) -> Color;
    fn card_bg(&self) -> Color;
    fn surface_bg(&self) -> Color;
    fn border(&self) -> Color;
    fn border_dim(&self) -> Color;

    // ─── Text ─────────────────────────────────────────────────────
    fn fg(&self) -> Color;
    fn muted(&self) -> Color;
    fn dim(&self) -> Color;

    // ─── Brand ────────────────────────────────────────────────────
    fn accent(&self) -> Color;
    fn accent_muted(&self) -> Color;
    fn accent_bright(&self) -> Color;

    // ─── Signal ───────────────────────────────────────────────────
    fn success(&self) -> Color;
    fn error(&self) -> Color;
    fn warning(&self) -> Color;
    fn caution(&self) -> Color;

    // ─── Extended (semantic mesh-specific colors) ─────────────────
    fn footer_bg(&self) -> Color { Color::Rgb(1, 3, 6) }
    fn sent_msg_bg(&self) -> Color { self.card_bg() }
    fn received_msg_bg(&self) -> Color { Color::Rgb(3, 12, 20) }
    fn event_bg(&self) -> Color { self.surface_bg() }
    fn link_active(&self) -> Color { self.success() }
    fn link_stale(&self) -> Color { self.warning() }
    fn link_closed(&self) -> Color { self.muted() }
    fn peer_online(&self) -> Color { self.success() }
    fn peer_offline(&self) -> Color { self.muted() }
    fn signal_strong(&self) -> Color { self.success() }
    fn signal_weak(&self) -> Color { self.caution() }
    fn signal_none(&self) -> Color { self.error() }

    // ─── Derived styles ───────────────────────────────────────────
    fn style_fg(&self) -> Style { Style::default().fg(self.fg()) }
    fn style_muted(&self) -> Style { Style::default().fg(self.muted()) }
    fn style_dim(&self) -> Style { Style::default().fg(self.dim()) }
    fn style_accent(&self) -> Style { Style::default().fg(self.accent()) }
    fn style_accent_bold(&self) -> Style {
        Style::default().fg(self.accent()).add_modifier(Modifier::BOLD)
    }
    fn style_success(&self) -> Style { Style::default().fg(self.success()) }
    fn style_error(&self) -> Style { Style::default().fg(self.error()) }
    fn style_warning(&self) -> Style { Style::default().fg(self.warning()) }
    fn style_heading(&self) -> Style {
        Style::default().fg(self.accent_bright()).add_modifier(Modifier::BOLD)
    }
    fn style_user_input(&self) -> Style {
        Style::default().fg(self.fg()).add_modifier(Modifier::BOLD)
    }
    fn style_footer_bg(&self) -> Style {
        Style::default().bg(self.footer_bg())
    }
    fn style_card(&self) -> Style {
        Style::default().bg(self.card_bg())
    }
    fn style_surface(&self) -> Style {
        Style::default().bg(self.surface_bg())
    }
    fn style_border(&self) -> Style { Style::default().fg(self.border()) }
    fn style_border_dim(&self) -> Style { Style::default().fg(self.border_dim()) }
}

// ═══════════════════════════════════════════════════════════════════
// Default theme — Styrene dark (navy → teal → amber)
// ═══════════════════════════════════════════════════════════════════

/// The default Styrene theme. Dark navy base, teal accent, amber signal.
pub struct StyreneTheme;

impl Theme for StyreneTheme {
    // Very dark navy base — evokes deep mesh / night sky
    fn bg(&self) -> Color { Color::Rgb(0, 1, 3) }
    fn card_bg(&self) -> Color { Color::Rgb(3, 8, 15) }
    fn surface_bg(&self) -> Color { Color::Rgb(6, 14, 24) }
    fn border(&self) -> Color { Color::Rgb(20, 50, 80) }
    fn border_dim(&self) -> Color { Color::Rgb(10, 25, 40) }

    fn fg(&self) -> Color { Color::Rgb(200, 220, 235) }
    fn muted(&self) -> Color { Color::Rgb(90, 120, 150) }
    fn dim(&self) -> Color { Color::Rgb(45, 65, 85) }

    // Teal accent — Reticulum mesh color identity
    fn accent(&self) -> Color { Color::Rgb(30, 160, 180) }
    fn accent_muted(&self) -> Color { Color::Rgb(20, 90, 110) }
    fn accent_bright(&self) -> Color { Color::Rgb(80, 210, 230) }

    fn success(&self) -> Color { Color::Rgb(40, 180, 100) }
    fn error(&self) -> Color { Color::Rgb(210, 60, 70) }
    fn warning(&self) -> Color { Color::Rgb(220, 160, 30) }
    fn caution(&self) -> Color { Color::Rgb(200, 120, 20) }
}

/// Convenience: boxed default theme for storage in App.
pub fn default_theme() -> Box<dyn Theme> {
    Box::new(StyreneTheme)
}
