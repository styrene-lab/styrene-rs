//! Application state — three-workspace shell with persistent sidebar and input bar.
//!
//! Layout:
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │  ■ styrene    Home · Peers · Messages       3↑  ●12  ◐2  ○1│
//! ├──────────┬───────────────────────────────────────────────────┤
//! │ SIDEBAR  │                MAIN PANE                          │
//! │          │                                                   │
//! ├──────────┴───────────────────────────────────────────────────┤
//! │ > _                                                          │
//! └──────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::time::Instant;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};

use crate::mesh_state::{ActivityLog, LinkRecord, PeerRecord, PeerStatus, epoch_secs};
use crate::tui::conv_widget::ConversationWidget;
use crate::tui::conversation::ConversationView;
use crate::tui::editor::Editor;
use crate::tui::effects::Effects;
use crate::tui::segments::{DeliveryStatus, ProtocolEventKind};
use crate::tui::signal::{self, SignalState};
use crate::tui::theme::{self, Theme};
use crate::tui::topology::TopologyState;

// ─── Layout constants ────────────────────────────────────────────────────────

const SIDEBAR_WIDTH: u16 = 28;
const SIDEBAR_COLLAPSE_THRESHOLD: u16 = 60;

// ─── Workspace ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workspace {
    Home,
    Peers,
    Messages,
}

impl Workspace {
    pub const ALL: [Workspace; 3] = [Workspace::Home, Workspace::Peers, Workspace::Messages];

    pub fn title(&self) -> &'static str {
        match self {
            Workspace::Home => "Home",
            Workspace::Peers => "Peers",
            Workspace::Messages => "Messages",
        }
    }
}

// ─── Input mode ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputMode {
    /// Default — input bar shows status
    Normal,
    /// Command mode — `:` prefix, buffer holds typed text after `:`
    Command { buffer: String },
    /// Search mode — `/` prefix, filters sidebar
    Search { query: String },
    /// Compose mode — writing a chat message
    Compose,
}

// ─── Focus ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Main,
    Input,
}

// ─── App ─────────────────────────────────────────────────────────────────────

pub struct App {
    pub theme: Box<dyn Theme>,

    // Navigation
    pub workspace: Workspace,
    pub focus: Focus,
    pub input_mode: InputMode,
    pub sidebar_visible: bool,

    // Data model
    pub peers: Vec<PeerRecord>,
    pub links: Vec<LinkRecord>,
    pub activity: ActivityLog,

    // Panels
    pub conversation: ConversationView, // system/global view (Home workspace)
    pub conversations: HashMap<String, ConversationView>, // per-peer (Messages/Chat)
    pub editor: Editor,
    pub effects: Effects,
    #[allow(dead_code)] // used by future tree-mode sidebar
    pub topology: TopologyState,
    pub signal: SignalState,

    // Sidebar state
    pub sidebar_selection: usize,

    // Peers workspace: selected peer + active tab
    pub selected_peer: Option<String>,
    pub peer_tab: PeerTab,

    // Messages workspace: selected conversation
    pub selected_conversation: Option<String>,

    // Commands tab state
    pub command_tab: CommandTabState,

    // Terminal tab state
    pub terminal_tab: TerminalTabState,

    // Pages tab state
    pub page_source: Option<String>,
    pub page_path: Option<String>,
    pub page_index: Vec<String>,

    // Daemon state (populated from IPC events)
    pub node_hash: String,
    pub node_name: String,
    pub daemon_connected: bool,
    pub daemon_version: String,
    pub rns_initialized: bool,
    pub transport_active: bool,
    pub propagation_enabled: bool,
    pub interface_count: u32,

    // Mesh badges (computed each tick)
    pub badge_online: usize,
    pub badge_stale: usize,
    pub badge_lost: usize,
    pub unread_count: usize,

    // Daemon command queue (None in demo mode)
    pub cmd_tx: Option<tokio::sync::mpsc::Sender<crate::daemon::DaemonCmd>>,

    // Settings panel
    pub settings_open: bool,

    // UI state
    pub last_ctrl_c: Option<Instant>,
    pub last_tick: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerTab {
    Status,
    Chat,
    Pages,
    Terminal,
    Commands,
}

// ─── Commands Tab State ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandAction {
    QueryStatus,
    RemoteExec,
    Reboot,
    ConfigPush,
}

impl CommandAction {
    pub const ALL: [CommandAction; 4] = [
        CommandAction::QueryStatus,
        CommandAction::RemoteExec,
        CommandAction::Reboot,
        CommandAction::ConfigPush,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            CommandAction::QueryStatus => "Query Status",
            CommandAction::RemoteExec => "Execute Command",
            CommandAction::Reboot => "Reboot Device",
            CommandAction::ConfigPush => "Push Config",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            CommandAction::QueryStatus => {
                "Query remote device status (uptime, version, mesh state)"
            }
            CommandAction::RemoteExec => "Execute a shell command on the remote device",
            CommandAction::Reboot => "Reboot the remote device (with optional delay)",
            CommandAction::ConfigPush => "Push a signed configuration profile to the remote node",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            CommandAction::QueryStatus => "?",
            CommandAction::RemoteExec => ">",
            CommandAction::Reboot => "!",
            CommandAction::ConfigPush => "^",
        }
    }
}

// ─── Terminal Tab State ──────────────────────────────────────────────────────

pub struct TerminalTabState {
    pub session_id: Option<String>,
    pub scrollback: Vec<String>,
    pub scroll_offset: usize,
    pub status: TerminalStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

impl Default for TerminalTabState {
    fn default() -> Self {
        Self {
            session_id: None,
            scrollback: Vec::new(),
            scroll_offset: 0,
            status: TerminalStatus::Disconnected,
        }
    }
}

impl TerminalTabState {
    pub fn push_output(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(data);
        for line in text.split('\n') {
            let clean = strip_ansi_escapes(line);
            self.scrollback.push(clean);
        }
        // Cap scrollback at 10K lines
        if self.scrollback.len() > 10_000 {
            let excess = self.scrollback.len() - 10_000;
            self.scrollback.drain(..excess);
        }
    }
}

fn strip_ansi_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next(); // consume '['
                    // CSI: skip until final byte [A-Za-z@]
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() || c == '@' {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next(); // consume ']'
                    // OSC: skip until BEL (\x07) or ST (ESC\)
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                                break;
                            }
                        }
                    }
                }
                _ => {
                    chars.next(); // skip one char for other escape types
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub struct CommandTabState {
    pub selected: usize,
    pub result_text: String,
    pub is_executing: bool,
}

impl Default for CommandTabState {
    fn default() -> Self {
        Self { selected: 0, result_text: String::new(), is_executing: false }
    }
}

impl PeerTab {
    pub const ALL: [PeerTab; 5] =
        [PeerTab::Status, PeerTab::Chat, PeerTab::Pages, PeerTab::Terminal, PeerTab::Commands];

    pub fn title(&self) -> &'static str {
        match self {
            PeerTab::Status => "Status",
            PeerTab::Chat => "Chat",
            PeerTab::Pages => "Pages",
            PeerTab::Terminal => "Terminal",
            PeerTab::Commands => "Commands",
        }
    }
}

impl App {
    pub fn new() -> Self {
        let theme = theme::default_theme();
        let mut editor = Editor::new();
        editor.apply_theme(theme.as_ref());

        Self {
            theme,
            workspace: Workspace::Home,
            focus: Focus::Main,
            input_mode: InputMode::Normal,
            sidebar_visible: true,
            peers: Vec::new(),
            links: Vec::new(),
            activity: ActivityLog::new(),
            conversation: ConversationView::new(),
            conversations: HashMap::new(),
            editor,
            effects: Effects::new(),
            topology: TopologyState::new(),
            signal: SignalState::new(),
            node_hash: String::new(),
            node_name: String::new(),
            daemon_connected: false,
            daemon_version: String::new(),
            rns_initialized: false,
            transport_active: false,
            propagation_enabled: false,
            interface_count: 0,
            sidebar_selection: 0,
            selected_peer: None,
            peer_tab: PeerTab::Status,
            selected_conversation: None,
            command_tab: CommandTabState::default(),
            terminal_tab: TerminalTabState::default(),
            page_source: None,
            page_path: None,
            page_index: Vec::new(),
            cmd_tx: None,
            settings_open: false,
            badge_online: 0,
            badge_stale: 0,
            badge_lost: 0,
            unread_count: 0,
            last_ctrl_c: None,
            last_tick: Instant::now(),
        }
    }

    // ─── Navigation ──────────────────────────────────────────────────────────

    pub fn set_workspace(&mut self, ws: Workspace) {
        self.workspace = ws;
        self.sidebar_selection = 0;
        self.focus = Focus::Sidebar;
    }

    pub fn next_workspace(&mut self) {
        let idx = Workspace::ALL.iter().position(|w| *w == self.workspace).unwrap_or(0);
        self.set_workspace(Workspace::ALL[(idx + 1) % Workspace::ALL.len()]);
    }

    pub fn prev_workspace(&mut self) {
        let idx = Workspace::ALL.iter().position(|w| *w == self.workspace).unwrap_or(0);
        self.set_workspace(Workspace::ALL[(idx + Workspace::ALL.len() - 1) % Workspace::ALL.len()]);
    }

    #[allow(dead_code)] // available for keybind wiring
    pub fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
    }

    pub fn next_peer_tab(&mut self) {
        let idx = PeerTab::ALL.iter().position(|t| *t == self.peer_tab).unwrap_or(0);
        self.peer_tab = PeerTab::ALL[(idx + 1) % PeerTab::ALL.len()];
    }

    /// Get the currently active conversation for scrolling (workspace-aware).
    pub fn active_conversation_mut(&mut self) -> &mut ConversationView {
        match self.workspace {
            Workspace::Messages => {
                if let Some(ref hash) = self.selected_conversation {
                    let hash = hash.clone();
                    return self.conversations.entry(hash).or_insert_with(ConversationView::new);
                }
                &mut self.conversation
            }
            Workspace::Peers if self.peer_tab == PeerTab::Chat => {
                if let Some(ref hash) = self.selected_peer {
                    let hash = hash.clone();
                    return self.conversations.entry(hash).or_insert_with(ConversationView::new);
                }
                &mut self.conversation
            }
            _ => &mut self.conversation,
        }
    }

    /// Get or create a per-peer conversation view.
    pub fn peer_conversation(&mut self, peer_hash: &str) -> &mut ConversationView {
        self.conversations.entry(peer_hash.to_string()).or_insert_with(ConversationView::new)
    }

    // ─── Sidebar data ────────────────────────────────────────────────────────

    pub fn sidebar_items(&self) -> Vec<(String, String, Option<usize>)> {
        // Returns (hash, display_name, unread_count) for the current workspace,
        // filtered by search query when in Search mode.
        let search = match &self.input_mode {
            InputMode::Search { query } if !query.is_empty() => Some(query.to_lowercase()),
            _ => None,
        };

        let matches_search = |hash: &str, name: &str| -> bool {
            match &search {
                Some(q) => name.to_lowercase().contains(q) || hash.to_lowercase().contains(q),
                None => true,
            }
        };

        match self.workspace {
            Workspace::Home | Workspace::Peers => self
                .peers
                .iter()
                .filter_map(|p| {
                    let name = p.name.clone().unwrap_or_else(|| p.hash[..8].to_string());
                    if matches_search(&p.hash, &name) {
                        Some((p.hash.clone(), name, None))
                    } else {
                        None
                    }
                })
                .collect(),
            Workspace::Messages => {
                // Show peers that have conversations, sorted by most recent message
                let mut convos: Vec<_> = self
                    .conversations
                    .keys()
                    .filter_map(|hash| {
                        let conv = self.conversations.get(hash)?;
                        if conv.segments().is_empty() {
                            return None;
                        }
                        let name = self
                            .peers
                            .iter()
                            .find(|p| p.hash == *hash)
                            .and_then(|p| p.name.clone())
                            .unwrap_or_else(|| hash[..8.min(hash.len())].to_string());
                        // Count unread (received messages — simplified)
                        let unread = conv
                            .segments()
                            .iter()
                            .filter(|s| {
                                matches!(s, crate::tui::segments::Segment::ReceivedMessage { .. })
                            })
                            .count();
                        if !matches_search(hash, &name) {
                            return None;
                        }
                        Some((hash.clone(), name, Some(unread)))
                    })
                    .collect();
                // Most recently active first (by segment count as proxy)
                convos.sort_by(|a, b| {
                    let a_count =
                        self.conversations.get(&a.0).map(|c| c.segments().len()).unwrap_or(0);
                    let b_count =
                        self.conversations.get(&b.0).map(|c| c.segments().len()).unwrap_or(0);
                    b_count.cmp(&a_count)
                });
                convos
            }
        }
    }

    // ─── Tick ────────────────────────────────────────────────────────────────

    pub fn tick(&mut self) {
        let dt = self.last_tick.elapsed().as_secs_f64();
        self.last_tick = Instant::now();

        self.signal.tick(dt);
        for link in &mut self.links {
            link.tick_wave(dt);
        }

        // Age peer status
        let now = epoch_secs();
        for peer in &mut self.peers {
            if peer.status == PeerStatus::Online && peer.age_secs(now) > 300 {
                peer.status = PeerStatus::Stale;
            }
        }

        // Compute badges
        self.badge_online = self.peers.iter().filter(|p| p.status == PeerStatus::Online).count();
        self.badge_stale = self.peers.iter().filter(|p| p.status == PeerStatus::Stale).count();
        self.badge_lost = self.peers.iter().filter(|p| p.status == PeerStatus::Offline).count();
    }

    // ─── Draw ────────────────────────────────────────────────────────────────

    pub fn draw(&mut self, f: &mut Frame) {
        let full = f.area();
        f.render_widget(Block::default().style(Style::default().bg(self.theme.bg())), full);

        let input_height = match self.input_mode {
            InputMode::Compose => 3u16,
            InputMode::Command { .. } | InputMode::Search { .. } => 1,
            InputMode::Normal => 1,
        };

        let [top_bar, body, input_bar] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(input_height),
        ])
        .areas(full);

        self.draw_top_bar(f, top_bar);
        self.draw_body(f, body);
        self.draw_input_bar(f, input_bar);

        // Settings panel overlay (right side)
        if self.settings_open {
            self.draw_settings_overlay(f, body);
        }

        // Post-process effects
        self.effects.process(f.buffer_mut(), body, input_bar, input_bar);
    }

    fn draw_top_bar(&self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();

        // Left: brand + workspace tabs
        let tabs: Vec<Span> = Workspace::ALL
            .iter()
            .flat_map(|ws| {
                let style = if *ws == self.workspace {
                    Style::default().fg(t.accent()).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.muted())
                };
                vec![
                    Span::styled(ws.title(), style),
                    Span::styled(" · ", Style::default().fg(t.dim())),
                ]
            })
            .collect();

        // Right: badges
        let badges = vec![
            Span::styled(
                format!("{}↑", self.unread_count),
                if self.unread_count > 0 {
                    Style::default().fg(t.accent()).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.dim())
                },
            ),
            Span::styled("  ", Style::default()),
            Span::styled(format!("●{}", self.badge_online), Style::default().fg(t.success())),
            Span::styled("  ", Style::default()),
            Span::styled(format!("◐{}", self.badge_stale), Style::default().fg(t.warning())),
            Span::styled("  ", Style::default()),
            Span::styled(format!("○{}", self.badge_lost), Style::default().fg(t.dim())),
        ];

        // Compose the line with brand on left, badges on right
        let hash_short = if self.node_hash.is_empty() {
            String::new()
        } else {
            format!("  {}…", &self.node_hash[..8.min(self.node_hash.len())])
        };
        let mut left_spans = vec![
            Span::styled("⬡ ", Style::default().fg(t.accent())),
            Span::styled("styrene", Style::default().fg(t.accent()).add_modifier(Modifier::BOLD)),
            Span::styled(&hash_short, Style::default().fg(t.dim())),
            Span::styled("   ", Style::default()),
        ];
        left_spans.extend(tabs);

        // Calculate right-side width for padding
        let right_text: String = badges.iter().map(|s| s.content.as_ref()).collect();
        let right_width = right_text.len() as u16;
        let left_text: String = left_spans.iter().map(|s| s.content.as_ref()).collect();
        let left_width = left_text.len() as u16;
        let pad = area.width.saturating_sub(left_width + right_width);

        left_spans.push(Span::styled(" ".repeat(pad as usize), Style::default()));
        left_spans.extend(badges);

        let bar = Paragraph::new(Line::from(left_spans)).style(Style::default().bg(t.bg()));
        f.render_widget(bar, area);
    }

    fn draw_body(&mut self, f: &mut Frame, area: Rect) {
        let show_sidebar = self.sidebar_visible && area.width >= SIDEBAR_COLLAPSE_THRESHOLD;

        if show_sidebar {
            let [sidebar_area, main_area] =
                Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)])
                    .areas(area);

            self.draw_sidebar(f, sidebar_area);
            self.draw_main(f, main_area);
        } else {
            self.draw_main(f, area);
        }
    }

    fn draw_sidebar(&mut self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();
        let items = self.sidebar_items();

        // Clamp selection to valid range when search filters reduce item count
        if !items.is_empty() && self.sidebar_selection >= items.len() {
            self.sidebar_selection = items.len() - 1;
        }

        let title = match self.workspace {
            Workspace::Home => " Peers ",
            Workspace::Peers => " Peers ",
            Workspace::Messages => " Conversations ",
        };

        let block = Block::default()
            .title(Span::styled(title, Style::default().fg(t.muted())))
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(t.border_dim()))
            .style(Style::default().bg(t.bg()));

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Render sidebar items
        let visible_height = inner.height as usize;
        let scroll_offset = if self.sidebar_selection >= visible_height {
            self.sidebar_selection - visible_height + 1
        } else {
            0
        };

        for (i, (hash, name, unread)) in
            items.iter().enumerate().skip(scroll_offset).take(visible_height)
        {
            let y = inner.y + (i - scroll_offset) as u16;
            if y >= inner.y + inner.height {
                break;
            }

            let is_selected = i == self.sidebar_selection && self.focus == Focus::Sidebar;

            // Status icon
            let peer = self.peers.iter().find(|p| p.hash == *hash);
            let (icon, icon_color) = match peer.map(|p| &p.status) {
                Some(PeerStatus::Online) => ("● ", t.success()),
                Some(PeerStatus::Stale) => ("◐ ", t.warning()),
                Some(PeerStatus::Offline) | None => ("○ ", t.dim()),
            };

            let name_style = if is_selected {
                Style::default().fg(t.accent()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.fg())
            };

            let mut spans = vec![
                Span::styled(icon, Style::default().fg(icon_color)),
                Span::styled(truncate_to(name, (SIDEBAR_WIDTH - 4) as usize), name_style),
            ];

            if let Some(count) = unread {
                if *count > 0 {
                    spans.push(Span::styled(format!(" {count}"), Style::default().fg(t.accent())));
                }
            }

            let line_area = Rect { x: inner.x, y, width: inner.width, height: 1 };

            if is_selected {
                f.render_widget(
                    Block::default().style(Style::default().bg(t.surface_bg())),
                    line_area,
                );
            }

            f.render_widget(Paragraph::new(Line::from(spans)), line_area);
        }
    }

    fn draw_main(&mut self, f: &mut Frame, area: Rect) {
        match self.workspace {
            Workspace::Home => self.draw_home(f, area),
            Workspace::Peers => self.draw_peers_workspace(f, area),
            Workspace::Messages => self.draw_messages_workspace(f, area),
        }
    }

    fn draw_home(&mut self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();

        // Split: activity feed (top) + node status (bottom)
        let [feed_area, status_area] =
            Layout::vertical([Constraint::Min(6), Constraint::Length(8)]).areas(area);

        // ── Activity feed ────────────────────────────────────────────────────
        let feed_block = Block::default()
            .title(Span::styled(" Activity ", Style::default().fg(t.muted())))
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(t.border_dim()))
            .style(Style::default().bg(t.bg()));
        let feed_inner = feed_block.inner(feed_area);
        f.render_widget(feed_block, feed_area);

        let entries: Vec<_> = self.activity.entries().take(feed_inner.height as usize).collect();
        for (i, entry) in entries.iter().enumerate() {
            let y = feed_inner.y + i as u16;
            if y >= feed_inner.y + feed_inner.height {
                break;
            }
            let age = entry.age_secs();
            let time_str = if age < 60.0 {
                format!("{:>3.0}s", age)
            } else if age < 3600.0 {
                format!("{:>3.0}m", age / 60.0)
            } else {
                format!("{:>3.0}h", age / 3600.0)
            };
            let icon = entry.kind.icon();
            let line = Line::from(vec![
                Span::styled(time_str, Style::default().fg(t.dim())),
                Span::styled("  ", Style::default()),
                Span::styled(icon, Style::default().fg(t.accent())),
                Span::styled(" ", Style::default()),
                Span::styled(&entry.peer_label, Style::default().fg(t.fg())),
                Span::styled(": ", Style::default().fg(t.dim())),
                Span::styled(&entry.detail, Style::default().fg(t.muted())),
            ]);
            f.render_widget(
                Paragraph::new(line),
                Rect { x: feed_inner.x, y, width: feed_inner.width, height: 1 },
            );
        }

        if entries.is_empty() {
            f.render_widget(
                Paragraph::new("  No activity yet — waiting for mesh events...")
                    .style(Style::default().fg(t.dim())),
                feed_inner,
            );
        }

        // ── Node status panel ────────────────────────────────────────────────
        let status_block = Block::default()
            .title(Span::styled(" Node ", Style::default().fg(t.muted())))
            .borders(Borders::TOP)
            .border_style(Style::default().fg(t.border_dim()))
            .style(Style::default().bg(t.bg()));
        let status_inner = status_block.inner(status_area);
        f.render_widget(status_block, status_area);

        // Node identity + status lines
        let hash_display = if self.node_hash.is_empty() {
            "not connected".to_string()
        } else {
            format!("{}…", &self.node_hash[..12.min(self.node_hash.len())])
        };

        let connection_color = if self.daemon_connected { t.success() } else { t.dim() };
        let connection_icon = if self.daemon_connected { "●" } else { "○" };

        let rns_status = if self.rns_initialized { "active" } else { "inactive" };
        let rns_color = if self.rns_initialized { t.success() } else { t.warning() };

        // Left column: identity + mesh status
        // Right column: signal waveform
        let [info_area, wave_area] =
            Layout::horizontal([Constraint::Length(40), Constraint::Min(12)]).areas(status_inner);

        let info_lines = vec![
            Line::from(vec![
                Span::styled("  Identity  ", Style::default().fg(t.muted())),
                Span::styled(&hash_display, Style::default().fg(t.fg())),
            ]),
            Line::from(vec![
                Span::styled("  Name      ", Style::default().fg(t.muted())),
                Span::styled(
                    if self.node_name.is_empty() { "—" } else { &self.node_name },
                    Style::default().fg(t.fg()),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Daemon    ", Style::default().fg(t.muted())),
                Span::styled(connection_icon, Style::default().fg(connection_color)),
                Span::styled(
                    if self.daemon_version.is_empty() {
                        " not connected".to_string()
                    } else {
                        format!(" v{}", self.daemon_version)
                    },
                    Style::default().fg(t.fg()),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Mesh      ", Style::default().fg(t.muted())),
                Span::styled(rns_status, Style::default().fg(rns_color)),
                Span::styled(
                    format!(
                        "  {} iface  {} peers  {} links",
                        self.interface_count,
                        self.peers.len(),
                        self.links.iter().filter(|l| l.status.is_active()).count(),
                    ),
                    Style::default().fg(t.dim()),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Propagation ", Style::default().fg(t.muted())),
                Span::styled(
                    if self.propagation_enabled { "enabled" } else { "disabled" },
                    Style::default().fg(if self.propagation_enabled {
                        t.success()
                    } else {
                        t.dim()
                    }),
                ),
            ]),
        ];

        f.render_widget(Paragraph::new(info_lines).style(Style::default().bg(t.bg())), info_area);

        // Signal waveform in the right column
        let links_snap = self.links.clone();
        signal::render(wave_area, f, &mut self.signal, &links_snap, &self.activity, t);
    }

    fn draw_peers_workspace(&mut self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();

        if self.selected_peer.is_none() {
            // No peer selected — show prompt
            f.render_widget(
                Paragraph::new("  Select a peer from the sidebar to view details")
                    .style(Style::default().fg(t.muted()).bg(t.bg())),
                area,
            );
            return;
        }

        let peer_hash = self.selected_peer.clone().unwrap_or_default();
        let peer_name = self
            .peers
            .iter()
            .find(|p| p.hash == peer_hash)
            .and_then(|p| p.name.clone())
            .unwrap_or_else(|| peer_hash[..8.min(peer_hash.len())].to_string());

        // Header with peer name + tabs
        let [header_area, content_area] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(area);

        // Peer header
        let header_line = Line::from(vec![
            Span::styled("  ⬡ ", Style::default().fg(t.accent())),
            Span::styled(&peer_name, Style::default().fg(t.fg()).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("  {}", &peer_hash[..12.min(peer_hash.len())]),
                Style::default().fg(t.dim()),
            ),
        ]);
        f.render_widget(
            Paragraph::new(header_line).style(Style::default().bg(t.bg())),
            Rect { x: header_area.x, y: header_area.y, width: header_area.width, height: 1 },
        );

        // Tab bar
        let tab_titles: Vec<&str> = PeerTab::ALL.iter().map(|t| t.title()).collect();
        let selected = PeerTab::ALL.iter().position(|tab| *tab == self.peer_tab).unwrap_or(0);
        let tabs = Tabs::new(tab_titles)
            .select(selected)
            .highlight_style(Style::default().fg(t.accent()).add_modifier(Modifier::BOLD))
            .style(Style::default().fg(t.muted()).bg(t.bg()))
            .divider("│");
        f.render_widget(
            tabs,
            Rect { x: header_area.x, y: header_area.y + 1, width: header_area.width, height: 1 },
        );

        // Tab content
        match self.peer_tab {
            PeerTab::Status => self.draw_peer_status(f, content_area, &peer_hash),
            PeerTab::Chat => self.draw_peer_chat(f, content_area),
            PeerTab::Commands => self.draw_peer_commands(f, content_area, &peer_hash),
            PeerTab::Terminal => self.draw_peer_terminal(f, content_area),
            PeerTab::Pages => self.draw_peer_pages(f, content_area, &peer_hash),
        }
    }

    fn draw_peer_pages(&self, f: &mut Frame, area: Rect, _peer_hash: &str) {
        let t = self.theme.as_ref();

        if let Some(source) = &self.page_source {
            // Render Micron page content
            let doc = styrene_micron::parse(source);
            let rendered = crate::micron_widget::render_document(&doc, area.width);
            let path_display = self.page_path.as_deref().unwrap_or("/");
            let block = Block::default()
                .title(Span::styled(format!(" {} ", path_display), Style::default().fg(t.muted())))
                .borders(Borders::TOP)
                .border_style(Style::default().fg(t.border_dim()))
                .style(Style::default().bg(t.bg()));
            f.render_widget(
                Paragraph::new(rendered).block(block).wrap(ratatui::widgets::Wrap { trim: false }),
                area,
            );
        } else if !self.page_index.is_empty() {
            // Show page index
            let mut lines = vec![
                Line::from(Span::styled(
                    "  Pages served by this node:",
                    Style::default().fg(t.fg()).add_modifier(Modifier::BOLD),
                )),
                Line::default(),
            ];
            for path in &self.page_index {
                lines.push(Line::from(vec![
                    Span::styled("  > ", Style::default().fg(t.accent())),
                    Span::styled(path.as_str(), Style::default().fg(t.fg())),
                ]));
            }
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                "  Press Enter to browse selected page.",
                Style::default().fg(t.dim()),
            )));
            f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.bg())), area);
        } else {
            let lines = vec![
                Line::default(),
                Line::from(Span::styled(
                    "  Page Browser",
                    Style::default().fg(t.fg()).add_modifier(Modifier::BOLD),
                )),
                Line::default(),
                Line::from(Span::styled(
                    "  Press Enter to load pages from this node.",
                    Style::default().fg(t.dim()),
                )),
            ];
            f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.bg())), area);
        }
    }

    fn draw_peer_status(&self, f: &mut Frame, area: Rect, peer_hash: &str) {
        let t = self.theme.as_ref();
        let peer = self.peers.iter().find(|p| p.hash == peer_hash);

        let mut lines = Vec::new();
        if let Some(p) = peer {
            let status_str = match p.status {
                PeerStatus::Online => "● ACTIVE",
                PeerStatus::Stale => "◐ STALE",
                PeerStatus::Offline => "○ LOST",
            };
            let status_color = match p.status {
                PeerStatus::Online => t.success(),
                PeerStatus::Stale => t.warning(),
                PeerStatus::Offline => t.dim(),
            };

            lines.push(Line::from(vec![
                Span::styled("  Status:  ", Style::default().fg(t.muted())),
                Span::styled(status_str, Style::default().fg(status_color)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  First:   ", Style::default().fg(t.muted())),
                Span::styled(format!("{}", p.first_seen), Style::default().fg(t.fg())),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Last:    ", Style::default().fg(t.muted())),
                Span::styled(format!("{}", p.last_seen), Style::default().fg(t.fg())),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Hops:    ", Style::default().fg(t.muted())),
                Span::styled(format!("{}", p.hop_count), Style::default().fg(t.fg())),
            ]));

            // Show links for this peer
            let peer_links: Vec<_> = self.links.iter().filter(|l| l.peer_hash == p.hash).collect();
            if !peer_links.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("  Links:", Style::default().fg(t.muted()))));
                for link in peer_links {
                    lines.push(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(
                            format!("{}  RTT: {:.1}ms", &link.id[..8], link.rtt_ms),
                            Style::default().fg(t.fg()),
                        ),
                    ]));
                }
            }
        } else {
            lines.push(Line::from(Span::styled(
                "  No data available",
                Style::default().fg(t.dim()),
            )));
        }

        f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.bg())), area);
    }

    fn draw_peer_chat(&mut self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();
        let peer_hash = self.selected_peer.clone().unwrap_or_default();

        if let Some(conv) = self.conversations.get_mut(&peer_hash) {
            let (segments, state) = conv.segments_and_state();
            f.render_stateful_widget(ConversationWidget::new(segments, t), area, state);
        } else {
            f.render_widget(
                Paragraph::new("  No messages with this peer")
                    .style(Style::default().fg(t.dim()).bg(t.bg())),
                area,
            );
        }
    }

    fn draw_peer_terminal(&self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();

        let status_line = match &self.terminal_tab.status {
            TerminalStatus::Disconnected => Line::from(vec![
                Span::styled("  Terminal session not connected. ", Style::default().fg(t.dim())),
                Span::styled("Press Enter to open session.", Style::default().fg(t.muted())),
            ]),
            TerminalStatus::Connecting => {
                Line::from(Span::styled("  Connecting...", Style::default().fg(t.warning())))
            }
            TerminalStatus::Connected => Line::from(vec![
                Span::styled("  Terminal: ", Style::default().fg(t.dim())),
                Span::styled("connected", Style::default().fg(t.success())),
                Span::styled("  |  Ctrl+\\ to exit", Style::default().fg(t.dim())),
            ]),
            TerminalStatus::Error(msg) => Line::from(vec![
                Span::styled("  Error: ", Style::default().fg(t.error())),
                Span::styled(msg.as_str(), Style::default().fg(t.dim())),
            ]),
        };

        let [status_area, content_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

        f.render_widget(
            Paragraph::new(status_line).style(Style::default().bg(t.bg())),
            status_area,
        );

        // Scrollback content — scroll_offset=0 means bottom (most recent)
        let visible_height = content_area.height.saturating_sub(1) as usize; // -1 for border
        let total_lines = self.terminal_tab.scrollback.len();
        let max_scroll = total_lines.saturating_sub(visible_height);
        let user_offset = self.terminal_tab.scroll_offset.min(max_scroll);
        let skip_count = max_scroll.saturating_sub(user_offset);

        let lines: Vec<Line> = self
            .terminal_tab
            .scrollback
            .iter()
            .skip(skip_count)
            .take(visible_height)
            .map(|s| Line::from(Span::raw(s.as_str())))
            .collect();

        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(t.border_dim()))
            .style(Style::default().bg(Color::Black));

        f.render_widget(Paragraph::new(lines).block(block), content_area);
    }

    fn draw_peer_commands(&self, f: &mut Frame, area: Rect, peer_hash: &str) {
        let t = self.theme.as_ref();
        let peer_name = self
            .peers
            .iter()
            .find(|p| p.hash == peer_hash)
            .and_then(|p| p.name.clone())
            .unwrap_or_else(|| peer_hash[..8.min(peer_hash.len())].to_string());

        let [actions_area, result_area] = Layout::vertical([
            Constraint::Length(CommandAction::ALL.len() as u16 * 3 + 2),
            Constraint::Min(3),
        ])
        .areas(area);

        // Action cards
        let mut lines = vec![
            Line::from(vec![
                Span::styled("  Target: ", Style::default().fg(t.dim())),
                Span::styled(&peer_name, Style::default().fg(t.fg()).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("  ({})", &peer_hash[..12.min(peer_hash.len())]),
                    Style::default().fg(t.dim()),
                ),
            ]),
            Line::default(),
        ];

        for (i, action) in CommandAction::ALL.iter().enumerate() {
            let is_selected = i == self.command_tab.selected;
            let marker = if is_selected { ">" } else { " " };
            let style = if is_selected {
                Style::default().fg(t.accent()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.fg())
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {marker} "), style),
                Span::styled(format!("[{}] ", action.icon()), Style::default().fg(t.muted())),
                Span::styled(action.title(), style),
            ]));
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(action.description(), Style::default().fg(t.dim())),
            ]));
            lines.push(Line::default());
        }

        f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.bg())), actions_area);

        // Result area
        let result_block = Block::default()
            .title(Span::styled(" Result ", Style::default().fg(t.muted())))
            .borders(Borders::TOP)
            .border_style(Style::default().fg(t.border_dim()))
            .style(Style::default().bg(t.bg()));

        let result_text = if self.command_tab.is_executing {
            "  Executing...".to_string()
        } else if self.command_tab.result_text.is_empty() {
            "  Select an action and press Enter to execute".to_string()
        } else {
            self.command_tab.result_text.clone()
        };

        let result_style = if self.command_tab.is_executing {
            Style::default().fg(t.warning())
        } else if self.command_tab.result_text.is_empty() {
            Style::default().fg(t.dim())
        } else {
            Style::default().fg(t.fg())
        };

        f.render_widget(
            Paragraph::new(result_text)
                .style(result_style)
                .block(result_block)
                .wrap(ratatui::widgets::Wrap { trim: false }),
            result_area,
        );
    }

    fn draw_messages_workspace(&mut self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();

        let peer_hash = match &self.selected_conversation {
            Some(h) => h.clone(),
            None => {
                f.render_widget(
                    Paragraph::new("  Select a conversation from the sidebar")
                        .style(Style::default().fg(t.muted()).bg(t.bg())),
                    area,
                );
                return;
            }
        };

        if let Some(conv) = self.conversations.get_mut(&peer_hash) {
            let (segments, state) = conv.segments_and_state();
            f.render_stateful_widget(ConversationWidget::new(segments, t), area, state);
        } else {
            f.render_widget(
                Paragraph::new("  No messages yet").style(Style::default().fg(t.dim()).bg(t.bg())),
                area,
            );
        }
    }

    fn draw_settings_overlay(&self, f: &mut Frame, body_area: Rect) {
        let t = self.theme.as_ref();
        let panel_width = 34u16.min(body_area.width.saturating_sub(4));
        let panel_area = Rect {
            x: body_area.x + body_area.width - panel_width,
            y: body_area.y,
            width: panel_width,
            height: body_area.height,
        };

        // Dim the area behind the panel
        let dim_area = Rect {
            x: body_area.x,
            y: body_area.y,
            width: body_area.width.saturating_sub(panel_width),
            height: body_area.height,
        };
        f.render_widget(Block::default().style(Style::default().bg(Color::Black)), dim_area);

        // Panel background
        let block = Block::default()
            .title(Span::styled(
                " Settings ",
                Style::default().fg(t.accent()).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM)
            .border_style(Style::default().fg(t.border_dim()))
            .style(Style::default().bg(t.surface_bg()));
        let inner = block.inner(panel_area);
        f.render_widget(block, panel_area);

        let mut lines = Vec::new();

        // Identity section
        lines.push(Line::from(Span::styled(
            " Identity",
            Style::default().fg(t.accent()).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("  Name: ", Style::default().fg(t.dim())),
            Span::styled(
                if self.node_name.is_empty() { "(not set)" } else { &self.node_name },
                Style::default().fg(t.fg()),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Hash: ", Style::default().fg(t.dim())),
            Span::styled(
                &self.node_hash[..16.min(self.node_hash.len())],
                Style::default().fg(t.muted()),
            ),
        ]));
        lines.push(Line::default());

        // Network section
        lines.push(Line::from(Span::styled(
            " Network",
            Style::default().fg(t.accent()).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("  Transport: ", Style::default().fg(t.dim())),
            Span::styled(
                if self.transport_active { "active" } else { "inactive" },
                Style::default().fg(if self.transport_active { t.success() } else { t.error() }),
            ),
        ]));
        let iface_str = self.interface_count.to_string();
        lines.push(Line::from(vec![
            Span::styled("  Interfaces: ", Style::default().fg(t.dim())),
            Span::styled(iface_str, Style::default().fg(t.fg())),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Propagation: ", Style::default().fg(t.dim())),
            Span::styled(
                if self.propagation_enabled { "enabled" } else { "disabled" },
                Style::default().fg(if self.propagation_enabled { t.success() } else { t.muted() }),
            ),
        ]));
        let links_str = self.links.len().to_string();
        lines.push(Line::from(vec![
            Span::styled("  Links: ", Style::default().fg(t.dim())),
            Span::styled(links_str, Style::default().fg(t.fg())),
        ]));
        lines.push(Line::default());

        // Daemon section
        lines.push(Line::from(Span::styled(
            " Daemon",
            Style::default().fg(t.accent()).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("  Status: ", Style::default().fg(t.dim())),
            Span::styled(
                if self.daemon_connected { "connected" } else { "disconnected" },
                Style::default().fg(if self.daemon_connected { t.success() } else { t.error() }),
            ),
        ]));
        if !self.daemon_version.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  Version: ", Style::default().fg(t.dim())),
                Span::styled(self.daemon_version.clone(), Style::default().fg(t.fg())),
            ]));
        }
        lines.push(Line::default());

        // Mesh stats
        let peers_str = self.peers.len().to_string();
        let mesh_summary =
            format!(" ({}↑ {}? {}×)", self.badge_online, self.badge_stale, self.badge_lost);
        lines.push(Line::from(Span::styled(
            " Mesh",
            Style::default().fg(t.accent()).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("  Peers: ", Style::default().fg(t.dim())),
            Span::styled(peers_str, Style::default().fg(t.fg())),
            Span::styled(mesh_summary, Style::default().fg(t.dim())),
        ]));
        let unread_str = self.unread_count.to_string();
        lines.push(Line::from(vec![
            Span::styled("  Unread: ", Style::default().fg(t.dim())),
            Span::styled(
                unread_str,
                Style::default().fg(if self.unread_count > 0 { t.accent() } else { t.fg() }),
            ),
        ]));
        lines.push(Line::default());

        // Footer hint
        lines.push(Line::from(Span::styled(
            " Esc or :settings to close",
            Style::default().fg(t.dim()),
        )));

        f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.surface_bg())), inner);
    }

    fn draw_input_bar(&self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();

        match &self.input_mode {
            InputMode::Normal => {
                let status = match self.workspace {
                    Workspace::Home => "Tab: switch workspace  /: search  ?: help",
                    Workspace::Peers => "j/k: navigate  Enter: select  t: tree mode",
                    Workspace::Messages => "j/k: navigate  i: compose  /: search",
                };
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(" ", Style::default()),
                        Span::styled(status, Style::default().fg(t.dim())),
                    ]))
                    .style(Style::default().bg(t.bg())),
                    area,
                );
            }
            InputMode::Command { buffer } => {
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(" :", Style::default().fg(t.accent())),
                        Span::styled(buffer, Style::default().fg(t.fg())),
                        Span::styled("_", Style::default().fg(t.muted())),
                    ]))
                    .style(Style::default().bg(t.surface_bg())),
                    area,
                );
            }
            InputMode::Search { query } => {
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(" /", Style::default().fg(t.accent())),
                        Span::styled(query, Style::default().fg(t.fg())),
                        Span::styled("_", Style::default().fg(t.muted())),
                    ]))
                    .style(Style::default().bg(t.surface_bg())),
                    area,
                );
            }
            InputMode::Compose => {
                let block = Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(t.border_dim()))
                    .style(Style::default().bg(t.surface_bg()));
                let block_inner = block.inner(area);
                f.render_widget(block, area);
                f.render_widget(&self.editor.textarea, block_inner);
            }
        }
    }

    // ─── Demo / data injection ───────────────────────────────────────────────

    pub fn push_welcome(&mut self) {
        self.conversation.push_system(
            "⬡ Styrene mesh TUI\n\n  \
             Tab          switch workspace\n  \
             j/k          navigate sidebar\n  \
             Enter        select\n  \
             i            compose message\n  \
             /            search\n  \
             :            command mode\n  \
             Ctrl+C x2    quit",
        );
    }

    pub fn demo_announce(&mut self) {
        use crate::mesh_state::{ActivityEntry, ActivityKind, PeerRecord as MeshPeer};

        let idx = self.peers.len() + 1;
        let hash = format!("{:032x}", idx as u128 * 0xf1a7b3cafe01_u128);
        let name = format!("node-{idx}");
        let now = epoch_secs();

        if let Some(existing) = self.peers.iter_mut().find(|p| p.hash == hash) {
            existing.touch(now, 1);
        } else {
            self.peers.push(MeshPeer::new(hash.clone(), Some(name.clone()), now));
        }

        self.activity.push(ActivityEntry::new(ActivityKind::Announce, &name, "announce received"));
        self.conversation.push_protocol_event(
            ProtocolEventKind::Announce,
            Some(&hash[..8]),
            Some(&name),
            "announce received",
        );
    }

    pub fn demo_link(&mut self) {
        use crate::mesh_state::{
            ActivityEntry, ActivityKind, LinkRecord as MeshLink, PeerRecord as MeshPeer,
        };

        let idx = self.links.len() + 1;
        let peer_hash = format!("{:032x}", idx as u128 * 0xa3b7c1d5e2f0_u128);
        let link_id = format!("{:016x}", idx as u64 * 0xdeadbeef_u64);
        let name = format!("node-{}", idx + 100);
        let now = epoch_secs();

        if !self.peers.iter().any(|p| p.hash == peer_hash) {
            let mut peer = MeshPeer::new(peer_hash.clone(), Some(name.clone()), now);
            peer.link_ids.push(link_id.clone());
            self.peers.push(peer);
        }

        let mut link = MeshLink::new(link_id, peer_hash.clone(), Some(name.clone()), now);
        link.rtt_ms = 20.0 + (idx as f64 * 7.3) % 180.0;
        link.pluck();
        self.links.push(link);

        self.activity.push(ActivityEntry::new(ActivityKind::LinkUp, &name, "link established"));
        self.conversation.push_protocol_event(
            ProtocolEventKind::LinkEstablished,
            Some(&peer_hash[..8]),
            Some(&name),
            "link established",
        );

        // Push demo message to per-peer conversation
        let peer_key = peer_hash[..16.min(peer_hash.len())].to_string();
        let conv = self.peer_conversation(&peer_key);
        conv.push_received(
            &peer_key,
            Some(&name),
            Some("Hello"),
            "Demo inbound LXMF message over the new link.",
            now as i64,
        );
        self.unread_count += 1;
    }

    /// Queue a daemon command for async execution. No-op in demo mode.
    pub fn send_daemon_cmd(&self, cmd: crate::daemon::DaemonCmd) {
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.try_send(cmd);
        }
    }

    pub fn handle_compose_submit(&mut self, text: String) {
        let dest = self
            .selected_peer
            .clone()
            .or_else(|| self.selected_conversation.clone())
            .unwrap_or_else(|| "demo".to_string());
        let name = self
            .peers
            .iter()
            .find(|p| p.hash == dest)
            .and_then(|p| p.name.clone())
            .unwrap_or_else(|| "peer".to_string());

        // Push to per-peer conversation (optimistic UI)
        let conv = self.peer_conversation(&dest);
        conv.push_sent(&dest, Some(&name), &text, DeliveryStatus::Sending);

        // Activity log
        self.activity.push(crate::mesh_state::ActivityEntry::new(
            crate::mesh_state::ActivityKind::OutboundMessage,
            &name,
            &text[..text.len().min(32)],
        ));

        // Queue actual send via daemon
        self.send_daemon_cmd(crate::daemon::DaemonCmd::SendChat { peer_hash: dest, content: text });
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn truncate_to(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else if max > 1 {
        format!("{}…", &s[..max - 1])
    } else {
        s[..max].to_string()
    }
}
