//! Footer bar — 4-card mesh telemetry strip at the bottom of the TUI.
//!
//! Cards (left → right):
//!   Node     — local identity hash + display name
//!   Links    — active link count + quality gauge
//!   Peers    — known peer count + most recent announce age
//!   Messages — unread / total in store

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Padding, Paragraph};

use super::theme::Theme;
use super::widgets::{self, GaugeConfig};

/// Footer telemetry — updated from daemon RPC events.
#[derive(Default, Clone)]
pub struct FooterData {
    // Node card
    pub node_hash: String,
    pub node_name: String,
    pub transport_active: bool,

    // Links card
    pub active_links: usize,
    pub link_quality: f32,   // 0.0–1.0 average

    // Peers card
    pub known_peers: usize,
    pub last_announce_secs: Option<u64>,  // seconds since last announce seen

    // Messages card
    pub unread_messages: usize,
    pub total_messages: usize,
    #[allow(dead_code)]
    pub store_size_kb: usize, // Phase 5: wire from MessagesStore

    // Status flash
    pub flash_ticks: u8,
}

impl FooterData {
    pub fn trigger_flash(&mut self) { self.flash_ticks = 3; }
    pub fn tick_flash(&mut self) {
        if self.flash_ticks > 0 { self.flash_ticks = self.flash_ticks.saturating_sub(1); }
    }

    pub fn render(&self, area: Rect, frame: &mut Frame, t: &dyn Theme) {
        // Fill the footer zone with footer background
        let bg = Block::default().style(t.style_footer_bg());
        frame.render_widget(bg, area);

        if area.width < 50 {
            self.render_narrow(area, frame, t);
            return;
        }

        let [node_a, links_a, peers_a, msgs_a] = Layout::horizontal([
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
        ])
        .areas(area);

        self.render_node_card(node_a, frame, t);
        self.render_links_card(links_a, frame, t);
        self.render_peers_card(peers_a, frame, t);
        self.render_msgs_card(msgs_a, frame, t);
    }

    fn render_node_card(&self, area: Rect, frame: &mut Frame, t: &dyn Theme) {
        let hash_short = if self.node_hash.len() >= 8 {
            &self.node_hash[..8]
        } else {
            &self.node_hash
        };
        let name = if self.node_name.is_empty() { "unnamed" } else { &self.node_name };
        let transport_color = if self.transport_active { t.success() } else { t.muted() };
        let transport_icon = if self.transport_active { "◉" } else { "○" };

        let lines = vec![
            Line::from(vec![
                Span::styled("⬡ ", Style::default().fg(t.accent())),
                Span::styled(name, Style::default().fg(t.fg()).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled(format!("{hash_short}…"), Style::default().fg(t.dim())),
                Span::styled(format!("  {transport_icon}"), Style::default().fg(transport_color)),
            ]),
        ];

        let block = Block::default()
            .title(Span::styled(" Node ", Style::default().fg(t.muted())))
            .borders(Borders::RIGHT)
            .border_style(t.style_border_dim())
            .style(t.style_footer_bg())
            .padding(Padding::new(1, 1, 0, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_links_card(&self, area: Rect, frame: &mut Frame, t: &dyn Theme) {
        let bar_w = area.width.saturating_sub(6) as usize;
        let bar_spans = widgets::gauge_bar(
            &GaugeConfig { percent: self.link_quality * 100.0, bar_width: bar_w.max(4), memory_blocks: 0 },
            t,
        );
        let lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("{}", self.active_links),
                    Style::default().fg(t.accent_bright()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" active", Style::default().fg(t.muted())),
            ]),
            Line::from(bar_spans),
        ];

        let block = Block::default()
            .title(Span::styled(" Links ", Style::default().fg(t.muted())))
            .borders(Borders::RIGHT)
            .border_style(t.style_border_dim())
            .style(t.style_footer_bg())
            .padding(Padding::new(1, 1, 0, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_peers_card(&self, area: Rect, frame: &mut Frame, t: &dyn Theme) {
        let announce_age = match self.last_announce_secs {
            Some(s) if s < 60 => format!("{s}s ago"),
            Some(s) => format!("{}m ago", s / 60),
            None => "none".to_string(),
        };

        let lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("{}", self.known_peers),
                    Style::default().fg(t.accent_bright()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" known", Style::default().fg(t.muted())),
            ]),
            Line::from(vec![
                Span::styled("last ann ", Style::default().fg(t.dim())),
                Span::styled(announce_age, Style::default().fg(t.muted())),
            ]),
        ];

        let block = Block::default()
            .title(Span::styled(" Peers ", Style::default().fg(t.muted())))
            .borders(Borders::RIGHT)
            .border_style(t.style_border_dim())
            .style(t.style_footer_bg())
            .padding(Padding::new(1, 1, 0, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_msgs_card(&self, area: Rect, frame: &mut Frame, t: &dyn Theme) {
        let unread_color = if self.unread_messages > 0 { t.accent_bright() } else { t.muted() };

        let lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("{}", self.unread_messages),
                    Style::default().fg(unread_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" unread", Style::default().fg(t.muted())),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("{}", self.total_messages),
                    Style::default().fg(t.dim()),
                ),
                Span::styled(" total", Style::default().fg(t.dim())),
            ]),
        ];

        let border_color = if self.flash_ticks > 0 { t.accent() } else { t.border_dim() };

        let block = Block::default()
            .title(Span::styled(" Messages ", Style::default().fg(t.muted())))
            .borders(Borders::NONE)
            .border_style(Style::default().fg(border_color))
            .style(t.style_footer_bg())
            .padding(Padding::new(1, 1, 0, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_narrow(&self, area: Rect, frame: &mut Frame, t: &dyn Theme) {
        let hash_short = if self.node_hash.len() >= 6 { &self.node_hash[..6] } else { &self.node_hash };
        let line = Line::from(vec![
            Span::styled(format!("⬡ {hash_short}…"), Style::default().fg(t.accent())),
            Span::styled(format!("  {} links", self.active_links), Style::default().fg(t.muted())),
            Span::styled(format!("  {} peers", self.known_peers), Style::default().fg(t.muted())),
        ]);
        frame.render_widget(
            Paragraph::new(line).style(t.style_footer_bg()),
            area,
        );
    }
}
