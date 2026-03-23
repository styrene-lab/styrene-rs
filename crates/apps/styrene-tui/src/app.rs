//! Application state — owns all TUI state, wires theme/conversation/footer.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Padding};

use crate::tui::conversation::ConversationView;
use crate::tui::conv_widget::ConversationWidget;
use crate::tui::editor::Editor;
use crate::tui::effects::Effects;
use crate::tui::footer::FooterData;
use crate::tui::segments::{DeliveryStatus, ProtocolEventKind};
use crate::tui::theme::{self, Theme};

// ─── Tab ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Messages,
    Peers,
    Links,
    Config,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Messages, Tab::Peers, Tab::Links, Tab::Config];

    pub fn title(&self) -> &'static str {
        match self {
            Tab::Messages => "Messages",
            Tab::Peers => "Peers",
            Tab::Links => "Links",
            Tab::Config => "Config",
        }
    }
}

// ─── App ────────────────────────────────────────────────────────────────

pub struct App {
    pub theme: Box<dyn Theme>,
    pub conversation: ConversationView,
    pub editor: Editor,
    pub footer: FooterData,
    pub effects: Effects,
    pub composing: bool,
    pub active_tab: Tab,
    pub last_ctrl_c: Option<std::time::Instant>,

    // Demo state for the fake peer/announce feed
    announce_counter: usize,
    link_counter: usize,
}

impl App {
    pub fn new() -> Self {
        let theme = theme::default_theme();
        let mut editor = Editor::new();
        editor.apply_theme(theme.as_ref());
        let mut footer = FooterData::default();

        // Demo local node identity
        let id = rns_core::identity::PrivateIdentity::new_from_rand(&mut rand_core::OsRng);
        footer.node_hash = hex::encode(id.as_identity().address_hash.as_slice());
        footer.node_name = "local-node".to_string();
        footer.transport_active = true;
        footer.active_links = 0;
        footer.link_quality = 0.0;
        footer.known_peers = 0;

        Self {
            theme,
            conversation: ConversationView::new(),
            editor,
            footer,
            effects: Effects::new(),
            composing: false,
            active_tab: Tab::Messages,
            last_ctrl_c: None,
            announce_counter: 0,
            link_counter: 0,
        }
    }

    pub fn push_welcome(&mut self) {
        let hash_short = &self.footer.node_hash[..8.min(self.footer.node_hash.len())];
        let msg = format!(
            "⬡ Styrene mesh TUI\n\
             \n  Local node: {hash_short}…\n\
             \n  Tab / Shift+Tab  switch panels\n\
             \n  Ctrl+N           compose message\n\
             \n  r                demo announce\n\
             \n  l                demo link\n\
             \n  q / Ctrl+C×2     quit"
        );
        self.conversation.push_system(&msg);
    }

    pub fn next_tab(&mut self) {
        let idx = Tab::ALL.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        self.active_tab = Tab::ALL[(idx + 1) % Tab::ALL.len()];
    }

    pub fn prev_tab(&mut self) {
        let idx = Tab::ALL.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        self.active_tab = Tab::ALL[(idx + Tab::ALL.len() - 1) % Tab::ALL.len()];
    }

    pub fn tick(&mut self) {
        self.footer.tick_flash();
    }

    pub fn handle_compose_submit(&mut self, text: String) {
        // For now, just push it as a sent message to the demo peer
        self.conversation.push_sent(
            "demonode00",
            Some("Demo Node"),
            &text,
            DeliveryStatus::Sent,
        );
        self.footer.unread_messages = self.footer.unread_messages.saturating_add(0); // outbound
        self.footer.total_messages += 1;
    }

    pub fn demo_announce(&mut self) {
        self.announce_counter += 1;
        let hash = format!("peer{:06x}", self.announce_counter * 0xf1a7b3);
        let name = format!("node-{}", self.announce_counter);
        self.conversation.push_protocol_event(
            ProtocolEventKind::Announce,
            Some(&hash[..8]),
            Some(&name),
            "announce received",
        );
        self.footer.known_peers += 1;
        self.footer.last_announce_secs = Some(0);
        self.footer.trigger_flash();
    }

    pub fn demo_link(&mut self) {
        self.link_counter += 1;
        let hash = format!("peer{:06x}", self.link_counter * 0xa3b7c1);
        let name = format!("node-{}", self.link_counter);
        self.conversation.push_protocol_event(
            ProtocolEventKind::LinkEstablished,
            Some(&hash[..8]),
            Some(&name),
            "link established",
        );
        // Then drop a demo received message
        self.conversation.push_received(
            &hash[..16.min(hash.len())],
            Some(&name),
            Some("Hello"),
            "This is a demo inbound LXMF message over the new link.",
            1_700_000_000 + self.link_counter as i64 * 30,
        );
        self.footer.active_links += 1;
        self.footer.link_quality = (self.footer.active_links as f32 * 0.3).min(1.0);
        self.footer.unread_messages += 1;
        self.footer.total_messages += 1;
        self.footer.trigger_flash();
    }

    // ─── Draw ──────────────────────────────────────────────────────

    pub fn draw(&mut self, f: &mut Frame) {
        // Fill entire screen with bg
        let full = f.area();
        let bg_color = self.theme.bg();
        f.render_widget(Block::default().style(Style::default().bg(bg_color)), full);

        let compose_height = if self.composing { 3 } else { 0 };
        let [header_a, tabs_a, body_a, compose_a, footer_a] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(compose_height),
            Constraint::Length(2),
        ])
        .areas(full);

        // Draw each zone — reborrow theme each time to avoid long-lived borrow
        {
            let t = self.theme.as_ref();
            draw_header(f, header_a, t, &self.footer.node_hash);
            draw_tabs(f, tabs_a, t, self.active_tab);
        }

        match self.active_tab {
            Tab::Messages => self.draw_messages(f, body_a),
            Tab::Peers => {
                let t = self.theme.as_ref();
                draw_placeholder(f, body_a, t, "Peers");
            }
            Tab::Links => {
                let t = self.theme.as_ref();
                draw_placeholder(f, body_a, t, "Links");
            }
            Tab::Config => {
                let t = self.theme.as_ref();
                draw_placeholder(f, body_a, t, "Config");
            }
        }

        if self.composing {
            self.draw_compose(f, compose_a);
        }

        {
            let footer = self.footer.clone();
            let t = self.theme.as_ref();
            footer.render(footer_a, f, t);
        }

        self.effects.process(f.buffer_mut(), body_a, footer_a, compose_a);
    }

    fn draw_messages(&mut self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();
        let (segments, state) = self.conversation.segments_and_state();
        let widget = ConversationWidget::new(segments, t);
        f.render_stateful_widget(widget, area, state);
    }

    fn draw_compose(&mut self, f: &mut Frame, area: Rect) {
        let (border_color, surface_bg, muted) = {
            let t = self.theme.as_ref();
            (t.border(), t.surface_bg(), t.muted())
        };
        let block = Block::default()
            .title(ratatui::text::Span::styled(
                " Compose  Enter to send  Esc to cancel ",
                Style::default().fg(muted),
            ))
            .borders(Borders::TOP)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(surface_bg));
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(&self.editor.textarea, inner);
    }
}

// ─── Free-function renderers (no &mut self needed) ──────────────────────

fn draw_header(f: &mut Frame, area: Rect, t: &dyn Theme, node_hash: &str) {
    use ratatui::text::Line;
    use ratatui::widgets::Paragraph;
    let hash_short = &node_hash[..8.min(node_hash.len())];
    let header = Paragraph::new(Line::from(vec![
        ratatui::text::Span::styled("⬡ ", Style::default().fg(t.accent())),
        ratatui::text::Span::styled(
            "Styrene",
            Style::default().fg(t.accent()).add_modifier(Modifier::BOLD),
        ),
        ratatui::text::Span::styled("  mesh communications", Style::default().fg(t.muted())),
        ratatui::text::Span::styled(format!("  {hash_short}…"), Style::default().fg(t.dim())),
    ]))
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(t.style_border_dim())
            .style(Style::default().bg(t.bg()))
            .padding(Padding::new(1, 0, 0, 0)),
    );
    f.render_widget(header, area);
}

fn draw_tabs(f: &mut Frame, area: Rect, t: &dyn Theme, active: Tab) {
    use ratatui::widgets::Tabs;
    let titles: Vec<&str> = Tab::ALL.iter().map(|t| t.title()).collect();
    let selected = Tab::ALL.iter().position(|tab| *tab == active).unwrap_or(0);
    let tabs = Tabs::new(titles)
        .select(selected)
        .highlight_style(Style::default().fg(t.accent()).add_modifier(Modifier::BOLD))
        .style(Style::default().fg(t.muted()).bg(t.bg()))
        .divider("│");
    f.render_widget(tabs, area);
}

fn draw_placeholder(f: &mut Frame, area: Rect, t: &dyn Theme, label: &str) {
    use ratatui::widgets::Paragraph;
    f.render_widget(
        Paragraph::new(format!("  {label} — coming soon"))
            .style(Style::default().fg(t.muted()).bg(t.bg())),
        area,
    );
}
