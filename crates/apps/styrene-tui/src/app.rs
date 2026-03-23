//! Application state — owns all TUI state, wires theme/panels/data model.

use std::time::Instant;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Padding};

use crate::mesh_state::{
    ActivityEntry, ActivityKind, ActivityLog, LinkRecord, LinkStatus, PeerRecord, PeerStatus,
    epoch_secs,
};
use crate::tui::conversation::ConversationView;
use crate::tui::conv_widget::ConversationWidget;
use crate::tui::editor::Editor;
use crate::tui::effects::Effects;
use crate::tui::footer::FooterData;
use crate::tui::segments::{DeliveryStatus, ProtocolEventKind};
use crate::tui::signal::{self, SignalState};
use crate::tui::theme::{self, Theme};
use crate::tui::topology::{self, TopologyState};

// ─── Minimum width for the topology sidebar ──────────────────────────────────
const SIDEBAR_MIN_WIDTH: u16 = 120;
const SIDEBAR_WIDTH: u16 = 36;

// ─── Tab ─────────────────────────────────────────────────────────────────────

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

// ─── App ─────────────────────────────────────────────────────────────────────

pub struct App {
    pub theme: Box<dyn Theme>,

    // Data model
    pub peers: Vec<PeerRecord>,
    pub links: Vec<LinkRecord>,
    pub activity: ActivityLog,

    // Panels
    pub conversation: ConversationView,
    pub editor: Editor,
    pub footer: FooterData,
    pub effects: Effects,
    pub topology: TopologyState,
    pub signal: SignalState,

    // UI state
    pub composing: bool,
    pub active_tab: Tab,
    pub last_ctrl_c: Option<Instant>,
    pub last_tick: Instant,

    // Demo counters
    announce_counter: usize,
    link_counter: usize,
}

impl App {
    pub fn new() -> Self {
        let theme = theme::default_theme();
        let mut editor = Editor::new();
        editor.apply_theme(theme.as_ref());

        // Generate a demo local identity for display
        let id = rns_core::identity::PrivateIdentity::new_from_rand(&mut rand_core::OsRng);
        let node_hash = hex::encode(id.as_identity().address_hash.as_slice());

        let mut footer = FooterData::default();
        footer.node_hash = node_hash;
        footer.node_name = "local-node".to_string();
        footer.transport_active = true;

        Self {
            theme,
            peers: Vec::new(),
            links: Vec::new(),
            activity: ActivityLog::new(),
            conversation: ConversationView::new(),
            editor,
            footer,
            effects: Effects::new(),
            topology: TopologyState::new(),
            signal: SignalState::new(),
            composing: false,
            active_tab: Tab::Messages,
            last_ctrl_c: None,
            last_tick: Instant::now(),
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
             \n  Ctrl+D           toggle topology sidebar\n\
             \n  Ctrl+N           compose message\n\
             \n  r                demo announce\n\
             \n  l                demo link\n\
             \n  q / Ctrl+C×2     quit"
        );
        self.conversation.push_system(&msg);
    }

    // ─── Navigation ──────────────────────────────────────────────────────────

    pub fn next_tab(&mut self) {
        let idx = Tab::ALL.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        self.active_tab = Tab::ALL[(idx + 1) % Tab::ALL.len()];
    }

    pub fn prev_tab(&mut self) {
        let idx = Tab::ALL.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        self.active_tab = Tab::ALL[(idx + Tab::ALL.len() - 1) % Tab::ALL.len()];
    }

    pub fn toggle_sidebar(&mut self) {
        self.topology.toggle_active();
    }

    pub fn sidebar_visible(&self, terminal_width: u16) -> bool {
        terminal_width >= SIDEBAR_MIN_WIDTH
    }

    // ─── Tick — called every frame ────────────────────────────────────────────

    pub fn tick(&mut self) {
        let dt = self.last_tick.elapsed().as_secs_f64();
        self.last_tick = Instant::now();

        self.footer.tick_flash();
        self.signal.tick(dt);

        // Advance wave animation on all links
        for link in &mut self.links {
            link.tick_wave(dt);
        }

        // Age peer status: online → stale after 5 minutes
        let now = epoch_secs();
        for peer in &mut self.peers {
            if peer.status == PeerStatus::Online && peer.age_secs(now) > 300 {
                peer.status = PeerStatus::Stale;
            }
        }

        self.sync_footer();
    }

    /// Keep footer counts in sync with the data model.
    fn sync_footer(&mut self) {
        self.footer.known_peers = self.peers.len();
        self.footer.active_links = self.links.iter()
            .filter(|l| l.status.is_active())
            .count();

        // Average RTT quality for the gauge (0.0 = no links, 1.0 = perfect)
        let active_rtts: Vec<f64> = self.links.iter()
            .filter(|l| l.status == LinkStatus::Active && l.rtt_ms > 0.0)
            .map(|l| l.rtt_ms)
            .collect();
        self.footer.link_quality = if active_rtts.is_empty() {
            0.0
        } else {
            let avg_rtt = active_rtts.iter().sum::<f64>() / active_rtts.len() as f64;
            // Map 0–2000ms to 1.0–0.0 (lower RTT = higher quality)
            (1.0 - (avg_rtt / 2000.0).min(1.0)) as f32
        };

        // Last announce age
        if self.peers.is_empty() {
            self.footer.last_announce_secs = None;
        } else {
            let now = epoch_secs();
            let most_recent = self.peers.iter()
                .map(|p| p.last_seen)
                .max()
                .unwrap_or(0);
            self.footer.last_announce_secs = Some(now.saturating_sub(most_recent));
        }
    }

    // ─── Demo event generators ────────────────────────────────────────────────

    pub fn demo_announce(&mut self) {
        self.announce_counter += 1;
        let hash = format!("{:032x}", self.announce_counter as u128 * 0xf1a7b3cafe01_u128);
        let name = format!("node-{}", self.announce_counter);
        let now = epoch_secs();

        // Upsert peer record
        if let Some(existing) = self.peers.iter_mut().find(|p| p.hash == hash) {
            existing.touch(now, 1);
        } else {
            self.peers.push(PeerRecord::new(hash.clone(), Some(name.clone()), now));
        }

        // Activity log
        self.activity.push(ActivityEntry::new(
            ActivityKind::Announce,
            &name,
            "announce received",
        ));

        // Conversation event
        self.conversation.push_protocol_event(
            ProtocolEventKind::Announce,
            Some(&hash[..8]),
            Some(&name),
            "announce received",
        );

        self.footer.trigger_flash();
    }

    pub fn demo_link(&mut self) {
        self.link_counter += 1;
        let peer_hash = format!("{:032x}", self.link_counter as u128 * 0xa3b7c1d5e2f0_u128);
        let link_id = format!("{:016x}", self.link_counter as u64 * 0xdeadbeef_u64);
        let name = format!("node-{}", self.link_counter + 100);
        let now = epoch_secs();

        // Ensure peer exists
        if !self.peers.iter().any(|p| p.hash == peer_hash) {
            let mut peer = PeerRecord::new(peer_hash.clone(), Some(name.clone()), now);
            peer.link_ids.push(link_id.clone());
            self.peers.push(peer);
        } else if let Some(peer) = self.peers.iter_mut().find(|p| p.hash == peer_hash) {
            if !peer.link_ids.contains(&link_id) {
                peer.link_ids.push(link_id.clone());
            }
        }

        // Create link record with a fake initial RTT
        let mut link = LinkRecord::new(link_id.clone(), peer_hash.clone(), Some(name.clone()), now);
        link.rtt_ms = 20.0 + (self.link_counter as f64 * 7.3) % 180.0;
        link.pluck();
        self.links.push(link);

        // Activity log
        self.activity.push(ActivityEntry::new(
            ActivityKind::LinkUp,
            &name,
            "link established",
        ));

        // Conversation events
        self.conversation.push_protocol_event(
            ProtocolEventKind::LinkEstablished,
            Some(&peer_hash[..8]),
            Some(&name),
            "link established",
        );
        self.conversation.push_received(
            &peer_hash[..16.min(peer_hash.len())],
            Some(&name),
            Some("Hello"),
            "This is a demo inbound LXMF message over the new link.",
            now as i64,
        );
        self.activity.push(ActivityEntry::new(
            ActivityKind::InboundMessage,
            &name,
            "Hello",
        ));

        self.footer.unread_messages += 1;
        self.footer.total_messages += 1;
        self.footer.trigger_flash();
    }

    pub fn handle_compose_submit(&mut self, text: String) {
        let dest_hash = "demonode00000000000000000000000000";
        let dest_name = "Demo Node";
        self.conversation.push_sent(dest_hash, Some(dest_name), &text, DeliveryStatus::Sent);
        self.activity.push(ActivityEntry::new(
            ActivityKind::OutboundMessage,
            dest_name,
            text.chars().take(32).collect::<String>(),
        ));
        self.footer.total_messages += 1;
    }

    // ─── Draw ─────────────────────────────────────────────────────────────────

    pub fn draw(&mut self, f: &mut Frame) {
        let full = f.area();
        let bg_color = self.theme.bg();
        f.render_widget(Block::default().style(Style::default().bg(bg_color)), full);

        let compose_height = if self.composing { 3u16 } else { 0 };
        let [header_a, tabs_a, body_a, compose_a, footer_a] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(compose_height),
            Constraint::Length(2),
        ])
        .areas(full);

        {
            let t = self.theme.as_ref();
            draw_header(f, header_a, t, &self.footer.node_hash);
            draw_tabs(f, tabs_a, t, self.active_tab);
        }

        // Body: optionally split into [main | sidebar]
        let show_sidebar = self.sidebar_visible(full.width);
        let (main_a, sidebar_a) = if show_sidebar {
            let [m, s] = Layout::horizontal([
                Constraint::Min(0),
                Constraint::Length(SIDEBAR_WIDTH),
            ])
            .areas(body_a);
            (m, Some(s))
        } else {
            (body_a, None)
        };

        self.draw_main(f, main_a);

        if let Some(s_area) = sidebar_a {
            // Borrow split: extract what topology needs, then render
            let (peers_snap, links_snap) = (self.peers.clone(), self.links.clone());
            let t = self.theme.as_ref();
            topology::render(s_area, f, &mut self.topology, &peers_snap, &links_snap, t);
        }

        if self.composing {
            self.draw_compose(f, compose_a);
        }

        {
            let footer = self.footer.clone();
            let t = self.theme.as_ref();
            footer.render(footer_a, f, t);
        }

        self.effects.process(f.buffer_mut(), main_a, footer_a, compose_a);
    }

    fn draw_main(&mut self, f: &mut Frame, area: Rect) {
        match self.active_tab {
            Tab::Messages => self.draw_messages(f, area),
            Tab::Peers => self.draw_peers(f, area),
            Tab::Links => self.draw_links(f, area),
            Tab::Config => {
                let t = self.theme.as_ref();
                draw_placeholder(f, area, t, "Config");
            }
        }
    }

    fn draw_messages(&mut self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();
        let (segments, state) = self.conversation.segments_and_state();
        f.render_stateful_widget(ConversationWidget::new(segments, t), area, state);
    }

    fn draw_peers(&mut self, f: &mut Frame, area: Rect) {
        // Full-width peer tree when sidebar isn't visible (narrow terminal)
        let (peers_snap, links_snap) = (self.peers.clone(), self.links.clone());
        let t = self.theme.as_ref();
        topology::render(area, f, &mut self.topology, &peers_snap, &links_snap, t);
    }

    fn draw_links(&mut self, f: &mut Frame, area: Rect) {
        let links_snap = self.links.clone();
        let t = self.theme.as_ref();
        signal::render(area, f, &mut self.signal, &links_snap, &self.activity, t);
    }

    fn draw_compose(&mut self, f: &mut Frame, area: Rect) {
        let (border_color, surface_bg, muted) = {
            let t = self.theme.as_ref();
            (t.border(), t.surface_bg(), t.muted())
        };
        let block = Block::default()
            .title(Span::styled(
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

// ─── Free-function renderers ──────────────────────────────────────────────────

fn draw_header(f: &mut Frame, area: Rect, t: &dyn Theme, node_hash: &str) {
    use ratatui::text::Line;
    use ratatui::widgets::Paragraph;
    let hash_short = &node_hash[..8.min(node_hash.len())];
    let header = Paragraph::new(Line::from(vec![
        Span::styled("⬡ ", Style::default().fg(t.accent())),
        Span::styled("Styrene", Style::default().fg(t.accent()).add_modifier(Modifier::BOLD)),
        Span::styled("  mesh communications", Style::default().fg(t.muted())),
        Span::styled(format!("  {hash_short}…"), Style::default().fg(t.dim())),
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
