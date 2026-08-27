//! Application state with persistent navigation and input bar.
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
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs, Wrap};

use crate::action::Action;
use crate::mesh_state::{ActivityLog, LinkRecord, PeerRecord, PeerStatus, epoch_secs};
use crate::tui::conv_widget::ConversationWidget;
use crate::tui::conversation::ConversationView;
use crate::tui::editor::Editor;
use crate::tui::effects::Effects;
use crate::tui::segments::ProtocolEventKind;
use crate::tui::signal::{self, SignalState};
use crate::tui::theme::{self, Theme};
use crate::tui::topology::TopologyState;

// ─── Layout constants ────────────────────────────────────────────────────────

const SIDEBAR_WIDTH: u16 = 28;
const SIDEBAR_COLLAPSE_THRESHOLD: u16 = 60;

#[derive(Clone, PartialEq, Eq)]
pub struct PaperExportState {
    pub message_id: String,
    pub uri: String,
}

impl std::fmt::Debug for PaperExportState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PaperExportState")
            .field("message_id", &self.message_id)
            .field("uri", &"[REDACTED]")
            .finish()
    }
}

// ─── Workspace ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workspace {
    Home,
    Peers,
    Messages,
    Propagation,
}

impl Workspace {
    pub const ALL: [Workspace; 4] =
        [Workspace::Home, Workspace::Peers, Workspace::Messages, Workspace::Propagation];

    pub fn title(&self) -> &'static str {
        match self {
            Workspace::Home => "Home",
            Workspace::Peers => "Peers",
            Workspace::Messages => "Messages",
            Workspace::Propagation => "Propagation",
        }
    }
}

// ─── Input mode ──────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Eq)]
pub enum InputMode {
    /// Default — input bar shows status
    Normal,
    /// Command mode — `:` prefix, buffer holds typed text after `:`
    Command {
        buffer: String,
    },
    /// Search mode — `/` prefix, filters sidebar
    Search {
        query: String,
    },
    /// Compose mode — writing a chat message
    Compose,
    PageField {
        name: String,
        password: bool,
        buffer: String,
    },
    SavePath {
        download_id: String,
        buffer: String,
    },
}

impl std::fmt::Debug for InputMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => formatter.write_str("Normal"),
            Self::Command { buffer } => {
                formatter.debug_struct("Command").field("buffer", buffer).finish()
            }
            Self::Search { query } => {
                formatter.debug_struct("Search").field("query", query).finish()
            }
            Self::Compose => formatter.write_str("Compose"),
            Self::PageField { name, password, buffer } => formatter
                .debug_struct("PageField")
                .field("name", name)
                .field("password", password)
                .field("buffer", if *password { &"[REDACTED]" } else { buffer })
                .finish(),
            Self::SavePath { download_id, buffer } => formatter
                .debug_struct("SavePath")
                .field("download_id", download_id)
                .field("buffer", buffer)
                .finish(),
        }
    }
}

// ─── Focus ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Main,
    Input,
}

fn offset_index(current: usize, delta: isize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    current.saturating_add_signed(delta).min(count - 1)
}

// ─── App ─────────────────────────────────────────────────────────────────────

pub struct App {
    pub theme: Box<dyn Theme>,
    pub runtime_profile: crate::RuntimeProfile,

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
    pub conversation_summaries: HashMap<String, styrene_ipc::types::ConversationInfo>,
    pub conversation_cursor: Option<String>,
    pub conversation_page_loaded: bool,
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
    pub compose_delivery_method: String,
    pub compose_pending: Option<(String, String)>,
    pub paper_export: Option<PaperExportState>,
    pub message_cursors: HashMap<String, String>,
    pub loaded_message_ids: std::collections::HashSet<String>,
    pub history_message_ids: HashMap<String, std::collections::HashSet<String>>,
    pub live_messages: HashMap<String, styrene_ipc::types::MessageInfo>,
    pub message_page_live_baselines: HashMap<String, std::collections::HashSet<String>>,

    // Commands tab state
    pub command_tab: CommandTabState,

    // Terminal tab state
    pub terminal_tab: TerminalTabState,

    // Pages tab state
    pub page_content: Option<styrene_ipc::types::PageContent>,
    pub page_path: Option<String>,
    pub page_index: Vec<String>,
    pub page_selection: usize,
    pub page_link_selection: usize,
    pub page_field_selection: usize,
    pub page_field_values: std::collections::BTreeMap<String, Vec<String>>,
    pub page_download: Option<styrene_ipc::types::FileDownloadInfo>,
    pub pending_page_transition: Option<PendingPageTransition>,

    // Daemon state (populated from IPC events)
    pub node_hash: String,
    pub node_name: String,
    pub daemon_connected: bool,
    pub daemon_session_accepting_events: bool,
    pub daemon_version: String,
    pub rns_initialized: bool,
    pub transport_active: bool,
    pub propagation_enabled: bool,
    pub interface_count: u32,
    pub connection_generation: Option<u64>,
    pub event_connection_generation: Option<u64>,
    pub active_capabilities: Option<styrene_ipc::types::ActiveCapabilitiesInfo>,
    pub network_operations: Vec<styrene_ipc::types::NetworkOperationInfo>,
    pub request_observations: Vec<styrene_ipc::types::RequestObservationInfo>,
    pub resource_transfers: Vec<styrene_ipc::types::ResourceTransferInfo>,
    pub route_observations: Vec<styrene_ipc::types::PathInfo>,
    pub interface_observations: Vec<styrene_ipc::types::InterfaceDetail>,
    pub standard_propagation: Option<styrene_ipc::types::StandardPropagationSnapshot>,
    pub standard_propagation_error: Option<String>,
    pub propagation_scroll: u16,

    // Mesh badges (computed each tick)
    pub badge_online: usize,
    pub badge_stale: usize,
    pub badge_lost: usize,
    pub unread_count: usize,

    // Daemon command queue (None in demo mode)
    pub cmd_tx: Option<tokio::sync::mpsc::Sender<crate::daemon::QueuedDaemonCmd>>,
    pub(crate) connection_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::ConnectionControl>>,

    // Settings panel
    pub settings_open: bool,
    pub help_open: bool,
    pub palette_open: bool,
    pub palette_selection: usize,

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingPageTransition {
    Dismiss,
    Workspace(Workspace),
    Peer(String),
    PeerTab(PeerTab),
    Deselect,
}

// ─── Commands Tab State ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandAction {
    QueryStatus,
    Reboot,
    ConfigPush,
}

impl CommandAction {
    pub const ALL: [CommandAction; 3] =
        [CommandAction::QueryStatus, CommandAction::Reboot, CommandAction::ConfigPush];

    pub fn title(&self) -> &'static str {
        match self {
            CommandAction::QueryStatus => "Query Status",
            CommandAction::Reboot => "Reboot Device",
            CommandAction::ConfigPush => "Push Config",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            CommandAction::QueryStatus => {
                "Query remote device status (uptime, version, mesh state)"
            }
            CommandAction::Reboot => "Reboot the remote device (with optional delay)",
            CommandAction::ConfigPush => "Push a signed configuration profile to the remote node",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            CommandAction::QueryStatus => "?",
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
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
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

#[derive(Default)]
pub struct CommandTabState {
    pub selected: usize,
    pub result_text: String,
    pub is_executing: bool,
}

impl PeerTab {
    pub const ALL: [PeerTab; 3] = [PeerTab::Chat, PeerTab::Status, PeerTab::Pages];

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
            runtime_profile: crate::RuntimeProfile::Standard,
            workspace: Workspace::Home,
            focus: Focus::Main,
            input_mode: InputMode::Normal,
            sidebar_visible: true,
            peers: Vec::new(),
            links: Vec::new(),
            activity: ActivityLog::new(),
            conversation: ConversationView::new(),
            conversations: HashMap::new(),
            conversation_summaries: HashMap::new(),
            conversation_cursor: None,
            conversation_page_loaded: false,
            editor,
            effects: Effects::new(),
            topology: TopologyState::new(),
            signal: SignalState::new(),
            node_hash: String::new(),
            node_name: String::new(),
            daemon_connected: false,
            daemon_session_accepting_events: true,
            daemon_version: String::new(),
            rns_initialized: false,
            transport_active: false,
            propagation_enabled: false,
            interface_count: 0,
            connection_generation: None,
            event_connection_generation: None,
            active_capabilities: None,
            network_operations: Vec::new(),
            request_observations: Vec::new(),
            resource_transfers: Vec::new(),
            route_observations: Vec::new(),
            interface_observations: Vec::new(),
            standard_propagation: None,
            standard_propagation_error: None,
            propagation_scroll: 0,
            sidebar_selection: 0,
            selected_peer: None,
            peer_tab: PeerTab::Chat,
            selected_conversation: None,
            compose_delivery_method: "direct".into(),
            compose_pending: None,
            paper_export: None,
            message_cursors: HashMap::new(),
            loaded_message_ids: std::collections::HashSet::new(),
            history_message_ids: HashMap::new(),
            live_messages: HashMap::new(),
            message_page_live_baselines: HashMap::new(),
            command_tab: CommandTabState::default(),
            terminal_tab: TerminalTabState::default(),
            page_content: None,
            page_path: None,
            page_index: Vec::new(),
            page_selection: 0,
            page_link_selection: 0,
            page_field_selection: 0,
            page_field_values: std::collections::BTreeMap::new(),
            page_download: None,
            pending_page_transition: None,
            cmd_tx: None,
            connection_tx: None,
            settings_open: false,
            help_open: false,
            palette_open: false,
            palette_selection: 0,
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
        if self.workspace == Workspace::Peers
            && self.peer_tab == PeerTab::Pages
            && self.page_content.is_some()
            && ws != self.workspace
        {
            self.request_page_transition(PendingPageTransition::Workspace(ws));
            return;
        }
        self.apply_workspace(ws);
    }

    fn apply_workspace(&mut self, ws: Workspace) {
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

    pub fn focus_next(&mut self) {
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Main,
            Focus::Main => Focus::Input,
            Focus::Input => Focus::Sidebar,
        };
    }

    pub fn focus_previous(&mut self) {
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Input,
            Focus::Main => Focus::Sidebar,
            Focus::Input => Focus::Main,
        };
    }

    pub fn dispatch(&mut self, action: Action) -> bool {
        match action {
            Action::WorkspaceNext => self.next_workspace(),
            Action::WorkspacePrevious => self.prev_workspace(),
            Action::FocusNext => self.focus_next(),
            Action::FocusPrevious => self.focus_previous(),
            Action::MoveUp => self.move_selection(-1),
            Action::MoveDown => self.move_selection(1),
            Action::MoveLeft => {
                if self.workspace == Workspace::Peers && self.focus == Focus::Main {
                    self.prev_peer_tab();
                }
            }
            Action::MoveRight => {
                if self.workspace == Workspace::Peers && self.focus == Focus::Main {
                    self.next_peer_tab();
                }
            }
            Action::Toggle => {}
            Action::PageUp if self.workspace == Workspace::Propagation => {
                self.propagation_scroll = self.propagation_scroll.saturating_sub(10);
            }
            Action::PageDown if self.workspace == Workspace::Propagation => {
                self.propagation_scroll = self.propagation_scroll.saturating_add(10);
            }
            Action::PageUp => self.active_conversation_mut().scroll_up(10),
            Action::PageDown => self.active_conversation_mut().scroll_down(10),
            Action::Activate => self.activate_focused(),
            Action::Back => {
                if self.workspace == Workspace::Peers
                    && self.peer_tab == PeerTab::Pages
                    && self.page_content.is_some()
                {
                    self.request_page_transition(PendingPageTransition::Dismiss);
                } else if self.help_open {
                    self.help_open = false;
                } else if self.palette_open {
                    self.palette_open = false;
                } else if self.focus != Focus::Sidebar {
                    self.focus = Focus::Sidebar;
                } else {
                    if self.page_content.is_some() {
                        self.request_page_transition(PendingPageTransition::Deselect);
                    } else {
                        self.selected_peer = None;
                        self.selected_conversation = None;
                        self.clear_page_state();
                    }
                }
            }
            Action::Compose => {
                if self.selected_peer.is_some() || self.selected_conversation.is_some() {
                    self.input_mode = InputMode::Compose;
                    self.focus = Focus::Input;
                    if let Some(peer_hash) = self.compose_peer() {
                        self.send_daemon_cmd(crate::daemon::DaemonCmd::LoadDraft { peer_hash });
                    }
                }
            }
            Action::Search => {
                self.input_mode = InputMode::Search { query: String::new() };
                self.focus = Focus::Input;
            }
            Action::OpenHelp => self.help_open = !self.help_open,
            Action::OpenPalette => {
                self.palette_open = !self.palette_open;
                self.palette_selection = 0;
            }
            Action::Quit => return true,
        }
        false
    }

    fn move_selection(&mut self, delta: isize) {
        if self.focus == Focus::Sidebar {
            let count = self.sidebar_items().len();
            self.sidebar_selection = offset_index(self.sidebar_selection, delta, count);
        } else if self.workspace == Workspace::Peers && self.peer_tab == PeerTab::Commands {
            self.command_tab.selected =
                offset_index(self.command_tab.selected, delta, CommandAction::ALL.len());
        } else if self.workspace == Workspace::Peers && self.peer_tab == PeerTab::Pages {
            if self.page_content.is_none() && !self.page_index.is_empty() {
                self.page_selection =
                    offset_index(self.page_selection, delta, self.page_index.len());
            }
        } else if self.workspace == Workspace::Propagation && delta < 0 {
            self.propagation_scroll = self.propagation_scroll.saturating_sub(1);
        } else if self.workspace == Workspace::Propagation {
            self.propagation_scroll = self.propagation_scroll.saturating_add(1);
        } else if delta < 0 {
            self.active_conversation_mut().scroll_up(3);
        } else {
            self.active_conversation_mut().scroll_down(3);
        }
    }

    fn activate_focused(&mut self) {
        if self.focus == Focus::Input {
            self.dispatch(Action::Compose);
            return;
        }
        if self.focus == Focus::Sidebar {
            let items = self.sidebar_items();
            if let Some((hash, _, _)) = items.get(self.sidebar_selection) {
                let hash = hash.clone();
                match self.workspace {
                    Workspace::Peers | Workspace::Home => {
                        self.select_peer(hash);
                        if self.workspace == Workspace::Home {
                            self.workspace = Workspace::Peers;
                        }
                    }
                    Workspace::Messages => self.selected_conversation = Some(hash),
                    Workspace::Propagation => {}
                }
                self.focus = Focus::Main;
            }
        } else if self.workspace == Workspace::Peers && self.peer_tab == PeerTab::Pages {
            if self.page_content.is_some() {
                self.request_page_transition(PendingPageTransition::Dismiss);
            } else if !self.page_index.is_empty() {
                let index = self.page_selection.min(self.page_index.len() - 1);
                let path = self.page_index[index].clone();
                if let Some(peer_hash) = self.selected_peer.clone() {
                    self.send_daemon_cmd(crate::daemon::DaemonCmd::BrowsePage {
                        host: peer_hash,
                        path,
                    });
                }
            } else if let Some(peer_hash) = self.selected_peer.clone()
                && self.peers.iter().any(|peer| peer.hash == peer_hash && peer.native_page_host)
            {
                self.send_daemon_cmd(crate::daemon::DaemonCmd::BrowsePage {
                    host: peer_hash,
                    path: "/page/index.mu".into(),
                });
            }
        } else if self.workspace == Workspace::Messages
            || (self.workspace == Workspace::Peers && self.peer_tab == PeerTab::Chat)
        {
            self.dispatch(Action::Compose);
        }
    }

    fn clear_page_state(&mut self) {
        self.page_index.clear();
        self.page_selection = 0;
        self.page_content = None;
        self.page_path = None;
        self.page_field_values.clear();
        self.page_download = None;
    }

    pub fn select_peer(&mut self, hash: String) {
        let changed_peer = self.selected_peer.as_deref() != Some(hash.as_str());
        if changed_peer && self.page_content.is_some() {
            self.request_page_transition(PendingPageTransition::Peer(hash));
            return;
        }
        self.apply_peer(hash, changed_peer);
    }

    fn apply_peer(&mut self, hash: String, changed_peer: bool) {
        self.selected_peer = Some(hash);
        self.peer_tab = PeerTab::Chat;
        if changed_peer {
            self.clear_page_state();
        }
        self.focus = Focus::Main;
    }

    #[allow(dead_code)] // available for keybind wiring
    pub fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
    }

    pub fn next_peer_tab(&mut self) {
        let idx = PeerTab::ALL.iter().position(|t| *t == self.peer_tab).unwrap_or(0);
        self.set_peer_tab(PeerTab::ALL[(idx + 1) % PeerTab::ALL.len()]);
    }

    pub fn prev_peer_tab(&mut self) {
        let idx = PeerTab::ALL.iter().position(|t| *t == self.peer_tab).unwrap_or(0);
        self.set_peer_tab(PeerTab::ALL[(idx + PeerTab::ALL.len() - 1) % PeerTab::ALL.len()]);
    }

    pub fn set_peer_tab(&mut self, tab: PeerTab) {
        if self.peer_tab == PeerTab::Pages && tab != PeerTab::Pages && self.page_content.is_some() {
            self.request_page_transition(PendingPageTransition::PeerTab(tab));
        } else {
            self.peer_tab = tab;
        }
    }

    pub fn request_page_transition(&mut self, transition: PendingPageTransition) {
        let Some(session_id) =
            self.page_content.as_ref().map(|page| page.navigation.session_id.clone())
        else {
            self.apply_page_transition(transition);
            return;
        };
        if self.pending_page_transition.is_some() {
            return;
        }
        self.pending_page_transition = Some(transition);
        self.send_daemon_cmd(crate::daemon::DaemonCmd::ClosePage { session_id });
    }

    pub fn confirm_page_closed(&mut self) {
        self.clear_page_state();
        if let Some(transition) = self.pending_page_transition.take() {
            self.apply_page_transition(transition);
        }
    }

    fn apply_page_transition(&mut self, transition: PendingPageTransition) {
        match transition {
            PendingPageTransition::Dismiss => self.clear_page_state(),
            PendingPageTransition::Workspace(workspace) => {
                self.clear_page_state();
                self.apply_workspace(workspace);
            }
            PendingPageTransition::Peer(peer) => {
                self.clear_page_state();
                self.apply_peer(peer, true);
            }
            PendingPageTransition::PeerTab(tab) => {
                self.clear_page_state();
                self.peer_tab = tab;
            }
            PendingPageTransition::Deselect => {
                self.clear_page_state();
                self.selected_peer = None;
                self.selected_conversation = None;
            }
        }
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
                let mut hashes: std::collections::HashSet<_> =
                    self.conversations.keys().cloned().collect();
                hashes.extend(self.conversation_summaries.keys().cloned());
                let mut convos: Vec<_> = hashes
                    .iter()
                    .filter_map(|hash| {
                        let conv = self.conversations.get(hash);
                        let summary = self.conversation_summaries.get(hash);
                        if conv.is_none_or(|value| value.segments().is_empty()) && summary.is_none()
                        {
                            return None;
                        }
                        let name = summary
                            .and_then(|value| value.peer_name.clone())
                            .or_else(|| {
                                self.peers
                                    .iter()
                                    .find(|p| p.hash == *hash)
                                    .and_then(|p| p.name.clone())
                            })
                            .unwrap_or_else(|| hash[..8.min(hash.len())].to_string());
                        let unread = summary.map_or_else(
                            || {
                                conv.into_iter()
                                    .flat_map(|value| value.segments())
                                    .filter(|s| {
                                        matches!(
                                            s,
                                            crate::tui::segments::Segment::ReceivedMessage { .. }
                                        )
                                    })
                                    .count()
                            },
                            |value| value.unread_count as usize,
                        );
                        if !matches_search(hash, &name) {
                            return None;
                        }
                        Some((hash.clone(), name, Some(unread)))
                    })
                    .collect();
                convos.sort_by(|a, b| {
                    let timestamp = |hash: &String| {
                        self.conversation_summaries
                            .get(hash)
                            .and_then(|value| value.last_message_timestamp)
                            .unwrap_or_default()
                    };
                    timestamp(&b.0).cmp(&timestamp(&a.0))
                });
                convos
            }
            Workspace::Propagation => Vec::new(),
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
            InputMode::PageField { .. } | InputMode::SavePath { .. } => 1u16,
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
        if let Some(export) = &self.paper_export {
            let width = full.width.saturating_sub(8).min(96);
            let height = full.height.saturating_sub(6).min(14);
            let area = Rect::new(
                full.x + full.width.saturating_sub(width) / 2,
                full.y + full.height.saturating_sub(height) / 2,
                width,
                height,
            );
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(format!(
                    "Paper LXMF export\n\n{}\n\n[s] save to paper-{}.lxm   [Esc] dismiss\nThe URI remains selectable in this panel.",
                    export.uri, export.message_id
                ))
                .block(Block::default().borders(Borders::ALL).title(" Paper Export "))
                .wrap(Wrap { trim: false }),
                area,
            );
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
            Span::styled(
                format!(" [{}]", self.runtime_profile.label()),
                Style::default().fg(if self.runtime_profile.is_ephemeral() {
                    t.warning()
                } else {
                    t.dim()
                }),
            ),
            Span::styled(
                format!(
                    " v{}+{}",
                    env!("CARGO_PKG_VERSION"),
                    option_env!("STYRENE_BUILD_SHA").unwrap_or("unknown")
                ),
                Style::default().fg(t.dim()),
            ),
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
        let show_sidebar = self.workspace != Workspace::Propagation
            && self.sidebar_visible
            && area.width >= SIDEBAR_COLLAPSE_THRESHOLD;

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
            Workspace::Propagation => " Propagation ",
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

            if let Some(count) = unread
                && *count > 0
            {
                spans.push(Span::styled(format!(" {count}"), Style::default().fg(t.accent())));
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
            Workspace::Propagation => self.draw_propagation_workspace(f, area),
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

    fn draw_propagation_workspace(&self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();
        let block = Block::default()
            .title(Span::styled(
                " Standard LXMF Propagation ",
                Style::default().fg(t.accent()).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border_dim()));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let rows = wrap_propagation_rows(
            standard_propagation_rows(
                self.standard_propagation.as_ref(),
                self.standard_propagation_error.as_deref(),
            ),
            inner.width,
        );
        let max_scroll = rows.len().saturating_sub(inner.height as usize);
        let max_scroll = u16::try_from(max_scroll).unwrap_or(u16::MAX);
        let paragraph = Paragraph::new(rows).style(Style::default().fg(t.fg()).bg(t.bg()));
        f.render_widget(paragraph.scroll((self.propagation_scroll.min(max_scroll), 0)), inner);
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

    fn draw_peer_pages(&self, f: &mut Frame, area: Rect, peer_hash: &str) {
        let t = self.theme.as_ref();
        let unavailable =
            if self.peers.iter().any(|peer| peer.hash == peer_hash && peer.native_page_host) {
                self.mutation_availability("page.browse").err()
            } else {
                Some("peer has no canonical native NomadNet host announce".into())
            };

        if let Some(page) = &self.page_content {
            let path_display = self.page_path.as_deref().unwrap_or("/");
            let fields = page
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let selected = if index == self.page_field_selection { ">" } else { " " };
                    let value = self
                        .page_field_values
                        .get(&field.name)
                        .map(|values| {
                            if field.kind == styrene_ipc::types::PageFormFieldKind::Password {
                                if values.iter().any(|value| !value.is_empty()) {
                                    "********".to_string()
                                } else {
                                    String::new()
                                }
                            } else {
                                values.join(",")
                            }
                        })
                        .unwrap_or_default();
                    format!("{selected} {}={value}", field.name)
                })
                .collect::<Vec<_>>()
                .join(" | ");
            let links = page
                .link_targets
                .iter()
                .enumerate()
                .map(|(index, link)| {
                    format!(
                        "{}{}",
                        if index == self.page_link_selection { ">" } else { " " },
                        link.label.as_deref().unwrap_or(&link.target)
                    )
                })
                .collect::<Vec<_>>()
                .join(" | ");
            let stages = page
                .stages
                .iter()
                .map(|stage| {
                    format!(
                        "{:?}={:?} source={:?} evidence={:?} at={} gen={} corr={}",
                        stage.kind,
                        stage.state,
                        stage.observation.source,
                        stage.evidence_source,
                        stage
                            .observation
                            .observed_at
                            .map_or_else(|| "pending".into(), |value| value.to_string()),
                        stage
                            .observation
                            .connection_generation
                            .map_or_else(|| "unreported".into(), |value| value.to_string()),
                        stage.observation.correlation_id.as_deref().unwrap_or("unreported")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let download = self.page_download.as_ref().map_or_else(
                || "download=none".to_string(),
                |download| {
                    format!(
                        "download={:?} {:.0}% correlation={} transfer={:?} resource={} integrity={} sha256={} error={}",
                        download.state,
                        download.progress * 100.0,
                        download.correlation_id,
                        download.transfer,
                        download.resource_hash.as_deref().unwrap_or("none"),
                        download.integrity_verified,
                        download.sha256.as_deref().unwrap_or("pending"),
                        download.error.as_deref().unwrap_or("none"),
                    )
                },
            );
            let controls = unavailable.as_ref().map_or_else(
                || "keys: b/f back/forward r reload R bypass x close u/i field e edit space toggle n/p link Enter open d refresh c cancel s save".to_string(),
                |reason| format!("controls disabled: {reason}"),
            );
            let elapsed = page
                .elapsed_ms
                .map_or_else(|| "unreported".to_string(), |value| format!("{value}ms"));
            let warnings = page
                .parser_warnings
                .iter()
                .map(|warning| format!("{}: {}", warning.code, warning.message))
                .collect::<Vec<_>>()
                .join(" | ");
            let failure = page.failure.as_ref().map_or_else(
                || "none".to_string(),
                |failure| {
                    format!(
                        "{} stage={:?} retryable={}",
                        failure.code, failure.stage, failure.retryable
                    )
                },
            );
            let diagnostics = format!(
                "outcome={:?} elapsed={elapsed} failure={failure}\ncorrelation={} checksum={}\nrequest={} link={} path_hash={} rtt={:?}\ntransfer={:?} resource={} verified={} cache={:?} cache_origin={} history={}/{} back={} forward={}\nwarnings: {warnings}\nstages:\n{stages}\nfields: {fields}\nlinks: {links}\n{download}\n{controls}\n\n{}",
                page.outcome,
                page.correlation_id,
                page.source_checksum,
                page.request.request_id.as_deref().unwrap_or("unreported"),
                page.request.link_id.as_deref().unwrap_or("unreported"),
                page.request.path_hash,
                page.request.rtt_ms,
                page.transfer.kind,
                page.transfer.resource_hash.as_deref().unwrap_or("none"),
                page.transfer.verified,
                page.cache.status,
                page.cache.origin_correlation_id.as_deref().unwrap_or("none"),
                page.navigation.history_index.saturating_add(1),
                page.navigation.history_len,
                page.navigation.can_back,
                page.navigation.can_forward,
                page.rendered_text
            );
            let block = Block::default()
                .title(Span::styled(format!(" {} ", path_display), Style::default().fg(t.muted())))
                .borders(Borders::TOP)
                .border_style(Style::default().fg(t.border_dim()))
                .style(Style::default().bg(t.bg()));
            f.render_widget(
                Paragraph::new(diagnostics)
                    .block(block)
                    .wrap(ratatui::widgets::Wrap { trim: false }),
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
            for (index, path) in self.page_index.iter().enumerate() {
                let selected = index == self.page_selection;
                lines.push(Line::from(vec![
                    Span::styled(
                        if selected { "  > " } else { "    " },
                        Style::default().fg(t.accent()),
                    ),
                    Span::styled(
                        path.as_str(),
                        Style::default()
                            .fg(if selected { t.accent() } else { t.fg() })
                            .add_modifier(if selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                ]));
            }
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                unavailable.as_ref().map_or_else(
                    || "  Press Enter to browse selected page.".to_string(),
                    |reason| format!("  Browse disabled: {reason}"),
                ),
                Style::default().fg(if unavailable.is_some() { t.warning() } else { t.dim() }),
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
                    unavailable.as_ref().map_or_else(
                        || "  Press Enter to load pages from this node.".to_string(),
                        |reason| format!("  Page browsing disabled: {reason}"),
                    ),
                    Style::default().fg(if unavailable.is_some() { t.warning() } else { t.dim() }),
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
        let unavailable = self.mutation_availability("chat.send").err();

        if let Some(conv) = self.conversations.get_mut(&peer_hash) {
            let (segments, state) = conv.segments_and_state();
            f.render_stateful_widget(ConversationWidget::new(segments, t), area, state);
        } else {
            let message = unavailable.map_or_else(
                || "  No messages yet — press Enter to write the first message".to_string(),
                |reason| format!("  Chat input disabled: {reason}"),
            );
            f.render_widget(
                Paragraph::new(message).style(Style::default().fg(t.dim()).bg(t.bg())),
                area,
            );
        }
    }

    fn draw_peer_terminal(&self, f: &mut Frame, area: Rect) {
        let t = self.theme.as_ref();

        let status_line = match &self.terminal_tab.status {
            TerminalStatus::Disconnected => match self.mutation_availability("rpc.exec") {
                Ok(()) => Line::from(vec![
                    Span::styled(
                        "  Terminal session not connected. ",
                        Style::default().fg(t.dim()),
                    ),
                    Span::styled("Press Enter to open session.", Style::default().fg(t.muted())),
                ]),
                Err(reason) => Line::from(Span::styled(
                    format!("  Terminal disabled: {reason}"),
                    Style::default().fg(t.warning()),
                )),
            },
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
            let unavailable = match action {
                CommandAction::QueryStatus => self.mutation_availability("rpc.status").err(),
                CommandAction::Reboot => self.mutation_availability("rpc.reboot").err(),
                CommandAction::ConfigPush => Some("requires a profile file through the CLI".into()),
            };
            let marker = if is_selected { ">" } else { " " };
            let style = if unavailable.is_some() {
                Style::default().fg(t.dim())
            } else if is_selected {
                Style::default().fg(t.accent()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.fg())
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {marker} "), style),
                Span::styled(format!("[{}] ", action.icon()), Style::default().fg(t.muted())),
                Span::styled(action.title(), style),
                Span::styled(unavailable.unwrap_or_default(), Style::default().fg(t.warning())),
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

        let selected_unavailable = match CommandAction::ALL[self.command_tab.selected] {
            CommandAction::QueryStatus => self.mutation_availability("rpc.status").err(),
            CommandAction::Reboot => self.mutation_availability("rpc.reboot").err(),
            CommandAction::ConfigPush => Some("requires a profile file through the CLI".into()),
        };
        let result_text = if self.command_tab.is_executing {
            "  Executing...".to_string()
        } else if let Some(reason) = selected_unavailable {
            format!("  Command disabled: {reason}")
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
                    Workspace::Propagation => "j/k or PgUp/PgDn: scroll  r: refresh",
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
                let availability =
                    self.delivery_method_availability(&self.compose_delivery_method).err();
                let block = Block::default()
                    .borders(Borders::TOP)
                    .title(format!(
                        " {}  Tab: method  Ctrl-D: discard{} ",
                        self.compose_delivery_method,
                        availability
                            .as_deref()
                            .map(|reason| format!("  disabled: {reason}"))
                            .unwrap_or_default()
                    ))
                    .border_style(Style::default().fg(t.border_dim()))
                    .style(Style::default().bg(t.surface_bg()));
                let block_inner = block.inner(area);
                f.render_widget(block, area);
                f.render_widget(&self.editor.textarea, block_inner);
            }
            InputMode::PageField { name, password, buffer } => {
                let shown =
                    if *password { "*".repeat(buffer.chars().count()) } else { buffer.clone() };
                f.render_widget(
                    Paragraph::new(format!(" {name}: {shown}_"))
                        .style(Style::default().bg(t.surface_bg()).fg(t.fg())),
                    area,
                );
            }
            InputMode::SavePath { buffer, .. } => {
                f.render_widget(
                    Paragraph::new(format!(" save absolute path: {buffer}_"))
                        .style(Style::default().bg(t.surface_bg()).fg(t.fg())),
                    area,
                );
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
    pub fn send_daemon_cmd(&mut self, cmd: crate::daemon::DaemonCmd) {
        let Some(origin_generation) = self.connection_generation else {
            if matches!(&cmd, crate::daemon::DaemonCmd::RequeryStandardPropagation) {
                self.standard_propagation_error =
                    Some("capabilities unknown for this connection".into());
            }
            self.conversation
                .push_system("⬡ command disabled — capabilities unknown for this connection");
            return;
        };
        let capability = self.command_capability(&cmd);
        if let Err(reason) = self.mutation_availability(&capability) {
            if matches!(&cmd, crate::daemon::DaemonCmd::RequeryStandardPropagation) {
                self.standard_propagation_error = Some(reason.clone());
            }
            self.conversation.push_system(&format!("⬡ command disabled — {reason}"));
            return;
        }
        if let crate::daemon::DaemonCmd::LoadMessagePage { peer_hash, .. } = &cmd {
            let baseline = self
                .live_messages
                .values()
                .filter(|message| {
                    if message.is_outgoing {
                        message.destination_hash == *peer_hash
                    } else {
                        message.source_hash == *peer_hash
                    }
                })
                .map(|message| message.id.clone())
                .collect();
            self.message_page_live_baselines.insert(peer_hash.clone(), baseline);
        }
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.try_send(crate::daemon::QueuedDaemonCmd {
                command: cmd,
                origin_generation,
                capability,
            });
        }
    }

    pub fn mutation_availability(&self, capability: &str) -> Result<(), String> {
        if !self.daemon_connected {
            return Err("daemon disconnected".into());
        }
        if self.connection_generation.is_none() {
            return Err("capabilities unknown for this connection".into());
        }
        let capabilities = self.active_capabilities.as_ref().ok_or("capabilities unknown")?;
        if capabilities.version != styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION {
            return Err(format!("capabilities stale (version {})", capabilities.version));
        }
        if let Some(degraded) = capabilities.degraded.iter().find(|item| item.id == capability) {
            return Err(format!("{capability} unavailable: {}", degraded.reason));
        }
        if !capabilities.authorized_operations.iter().any(|item| item == capability) {
            return Err(format!("permission denied: {capability}"));
        }
        Ok(())
    }

    pub fn compose_peer(&self) -> Option<String> {
        self.selected_peer.clone().or_else(|| self.selected_conversation.clone())
    }

    pub fn delivery_method_availability(&self, method: &str) -> Result<(), String> {
        self.mutation_availability("chat.send")?;
        let active = self.active_capabilities.as_ref().ok_or("capabilities unknown")?;
        let runtime = |id: &str| active.runtime.iter().any(|item| item == id);
        match method {
            "direct" | "opportunistic" if runtime("runtime.lxmf.direct") => Ok(()),
            "propagated"
                if runtime("runtime.standard-lxmf.propagation-client")
                    && self.standard_propagation.as_ref().is_some_and(|snapshot| {
                        snapshot
                            .selection
                            .as_ref()
                            .and_then(|selection| selection.peer_hash.as_ref())
                            .is_some()
                    }) =>
            {
                Ok(())
            }
            "paper" if runtime("runtime.lxmf.paper-export") => Ok(()),
            "direct" | "opportunistic" => Err("runtime.lxmf.direct is not active".into()),
            "propagated" if !runtime("runtime.standard-lxmf.propagation-client") => {
                Err("standard LXMF propagation client is not active".into())
            }
            "propagated" => Err("no authoritative propagation peer is selected".into()),
            "paper" => Err("paper export is not active".into()),
            _ => Err("unknown delivery method".into()),
        }
    }

    pub fn cycle_delivery_method(&mut self) {
        self.compose_delivery_method = match self.compose_delivery_method.as_str() {
            "direct" => "opportunistic",
            "opportunistic" => "propagated",
            "propagated" => "paper",
            _ => "direct",
        }
        .into();
    }

    fn command_capability(&self, cmd: &crate::daemon::DaemonCmd) -> String {
        use crate::daemon::DaemonCmd;
        match cmd {
            DaemonCmd::SendChat { .. } => "chat.send".into(),
            DaemonCmd::MarkRead { .. }
            | DaemonCmd::SetDraft { .. }
            | DaemonCmd::LoadDraft { .. }
            | DaemonCmd::ClearDraft { .. } => "messaging.manage".into(),
            DaemonCmd::RetryMessage { .. } | DaemonCmd::CancelMessage { .. } => {
                "messaging.lifecycle".into()
            }
            DaemonCmd::LoadMessagePage { .. }
            | DaemonCmd::QueryMessage { .. }
            | DaemonCmd::LoadConversationPage { .. } => "messaging.history.read".into(),
            DaemonCmd::Announce => "network.announce".into(),
            DaemonCmd::StartNetworkOperation(request) => {
                format!("network.{}", request.kind.as_str())
            }
            DaemonCmd::CancelNetworkOperation { operation_id } => self
                .network_operations
                .iter()
                .find(|operation| operation.operation_id == *operation_id)
                .map(|operation| format!("network.{}", operation.kind.as_str()))
                .unwrap_or_else(|| "network.unknown".into()),
            DaemonCmd::StartRequest(_) => "network.request".into(),
            DaemonCmd::CancelRequest { .. } => "network.request_cancel".into(),
            DaemonCmd::CancelResource { .. } => "network.resource_cancel".into(),
            DaemonCmd::BlockPeer { .. } | DaemonCmd::UnblockPeer { .. } => "policy.update".into(),
            DaemonCmd::FleetApply { .. } => "rpc.fleet_apply".into(),
            DaemonCmd::SetIdentity { .. } | DaemonCmd::SetAutoReply { .. } => {
                "rpc.config_update".into()
            }
            DaemonCmd::Exec { .. } => "rpc.exec".into(),
            DaemonCmd::RebootDevice { .. } => "rpc.reboot".into(),
            DaemonCmd::BrowsePage { .. }
            | DaemonCmd::NavigatePage(_)
            | DaemonCmd::ClosePage { .. }
            | DaemonCmd::StartFileDownload(_)
            | DaemonCmd::QueryFileDownload { .. }
            | DaemonCmd::CancelFileDownload { .. }
            | DaemonCmd::SaveFileDownload { .. }
            | DaemonCmd::ListPages { .. } => "page.browse".into(),
            DaemonCmd::DeviceStatus { .. }
            | DaemonCmd::ReconcileNetworkObservations
            | DaemonCmd::RequeryStandardPropagation
            | DaemonCmd::InspectRoutes
            | DaemonCmd::InspectInterfaces
            | DaemonCmd::InspectLinks
            | DaemonCmd::InspectRequests
            | DaemonCmd::InspectResources => "rpc.status".into(),
        }
    }

    pub fn handle_compose_submit(&mut self, text: String) {
        if let Err(reason) = self.delivery_method_availability(&self.compose_delivery_method) {
            self.conversation.push_system(&format!("⬡ chat disabled — {reason}"));
            return;
        }
        if text.is_empty() || text.len() > styrene_ipc::types::MAX_CHAT_CONTENT_BYTES {
            self.conversation.push_system("⬡ message must be 1..=65536 UTF-8 bytes");
            return;
        }
        let dest = self
            .selected_peer
            .clone()
            .or_else(|| self.selected_conversation.clone())
            .unwrap_or_else(|| "demo".to_string());
        self.compose_pending = Some((dest.clone(), text.clone()));
        self.send_daemon_cmd(crate::daemon::DaemonCmd::SetDraft {
            peer_hash: dest.clone(),
            content: text.clone(),
        });
        self.send_daemon_cmd(crate::daemon::DaemonCmd::SendChat {
            peer_hash: dest,
            content: text,
            delivery_method: self.compose_delivery_method.clone(),
        });
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn standard_propagation_rows(
    snapshot: Option<&styrene_ipc::types::StandardPropagationSnapshot>,
    error: Option<&str>,
) -> Vec<Line<'static>> {
    let Some(snapshot) = snapshot else {
        return vec![Line::from(match error {
            Some(reason) => format!("State unavailable: {reason}"),
            None => "Waiting for an authoritative daemon snapshot.".into(),
        })];
    };
    if snapshot.observed_at.is_none() && !snapshot.registered && snapshot.policy.is_none() {
        return vec![Line::from(
            "No standard propagation host or client runtime observation is available.",
        )];
    }
    let mut rows = vec![Line::from(format!(
        "Status: {}  destination: {}  observed: {}  generation: {}",
        if snapshot.active { "active" } else { "inactive" },
        if snapshot.registered { "registered" } else { "not registered" },
        snapshot.observed_at.map_or_else(|| "not reported".into(), |value| value.to_string()),
        snapshot
            .connection_generation
            .map_or_else(|| "not reported".into(), |value| value.to_string()),
    ))];
    if let Some(reason) = error {
        rows.push(Line::from(format!("Refresh degraded: {reason}")));
    }
    if let Some(selection) = &snapshot.selection {
        rows.push(Line::from(format!(
            "Selected peer: {}  mode: {}  selected: {}",
            selection
                .peer_hash
                .as_deref()
                .map(|value| truncate_to(value, 20))
                .unwrap_or_else(|| "none".into()),
            selection.mode,
            selection.selected_at,
        )));
    } else {
        rows.push(Line::from("Selected peer: not reported"));
    }
    rows.push(Line::from(""));
    rows.push(Line::from("Capacity and policy"));
    rows.push(Line::from(format!(
        "  queue: {} items / {} bytes  acknowledged: {}  expired: {}  terminal: {}",
        snapshot.queue.queued_count,
        snapshot.queue.queued_bytes,
        snapshot.queue.acknowledged_count,
        snapshot.queue.expired_count,
        snapshot.queue.terminal_count,
    )));
    if let Some(policy) = &snapshot.policy {
        rows.extend([
            Line::from(format!(
                "  limits: {} items / {} bytes  expiry: {}s  throttle: {}s  offer links: {}",
                policy.queue_max_count,
                policy.queue_max_bytes,
                policy.expiry_secs,
                policy.throttle_secs,
                policy.max_offer_links,
            )),
            Line::from(format!(
                "  transfer: {} kB  sync: {} kB  stamp target/flexibility/peer: {}/{}/{}",
                policy.transfer_limit_kb,
                policy.sync_limit_kb,
                policy.target_cost,
                policy.flexibility,
                policy.peering_cost,
            )),
        ]);
    } else {
        rows.push(Line::from("  Policy not reported."));
    }

    rows.push(Line::from(""));
    rows.push(Line::from(format!("Peers ({})", snapshot.peers.len())));
    if snapshot.peers.is_empty() {
        rows.push(Line::from("  No peers reported."));
    }
    for peer in &snapshot.peers {
        rows.push(Line::from(format!(
            "  {}  configured={} enabled={} last={} retry={} backoff={} offered/wanted/accepted={}/{}/{} bytes={} failures={} costs={}/{}/{} limits={}/{}kB",
            truncate_to(&peer.peer_hash, 20),
            peer.configured,
            peer.enabled,
            peer.last_seen_at,
            peer.retry_at.map_or_else(|| "none".into(), |value| value.to_string()),
            peer.backoff_count,
            peer.offered_count,
            peer.wanted_count,
            peer.accepted_count,
            peer.accepted_bytes,
            peer.failure_count,
            peer.stamp_cost.map_or_else(|| "?".into(), |value| value.to_string()),
            peer.stamp_flexibility.map_or_else(|| "?".into(), |value| value.to_string()),
            peer.peering_cost.map_or_else(|| "?".into(), |value| value.to_string()),
            peer.transfer_limit_kb.map_or_else(|| "?".into(), |value| value.to_string()),
            peer.sync_limit_kb.map_or_else(|| "?".into(), |value| value.to_string()),
        )));
    }

    rows.push(Line::from(""));
    rows.push(Line::from(format!("Synchronization and transfers ({})", snapshot.attempts.len())));
    if snapshot.attempts.is_empty() {
        rows.push(Line::from("  No attempts reported."));
    }
    for attempt in &snapshot.attempts {
        rows.push(Line::from(format!(
            "  {} corr={} peer={} {:?}/{:?} {:?} outcome={:?} counts={}/{}/{} bytes={} updated={} deadline={} failure={}",
            truncate_to(&attempt.attempt_id, 18),
            truncate_to(&attempt.correlation_id, 18),
            attempt
                .peer_hash
                .as_deref()
                .map(|value| truncate_to(value, 18))
                .unwrap_or_else(|| "none".into()),
            attempt.direction,
            attempt.stage,
            attempt.state,
            attempt.outcome,
            attempt.offered_count,
            attempt.wanted_count,
            attempt.accepted_count,
            attempt.accepted_bytes,
            attempt.updated_at,
            attempt.deadline_at.map_or_else(|| "none".into(), |value| value.to_string()),
            attempt.failure_code.as_deref().unwrap_or("none"),
        )));
    }

    rows.push(Line::from(""));
    rows.push(Line::from(format!("Checkpoints ({})", snapshot.checkpoints.len())));
    if snapshot.checkpoints.is_empty() {
        rows.push(Line::from("  No checkpoints reported."));
    }
    for checkpoint in &snapshot.checkpoints {
        rows.push(Line::from(format!(
            "  peer={} {:?}/{:?} items={} bytes={} attempt={} updated={}",
            truncate_to(&checkpoint.peer_hash, 18),
            checkpoint.direction,
            checkpoint.completed_stage,
            checkpoint.item_count,
            checkpoint.byte_count,
            checkpoint
                .last_attempt_id
                .as_deref()
                .map(|value| truncate_to(value, 18))
                .unwrap_or_else(|| "none".into()),
            checkpoint.updated_at,
        )));
    }

    rows.push(Line::from(""));
    rows.push(Line::from(format!("Failures ({})", snapshot.failures.len())));
    if snapshot.failures.is_empty() {
        rows.push(Line::from("  No failures reported."));
    }
    for failure in &snapshot.failures {
        rows.push(Line::from(format!(
            "  {} at={} peer={} attempt={}",
            failure.code,
            failure.occurred_at,
            failure
                .peer_hash
                .as_deref()
                .map(|value| truncate_to(value, 18))
                .unwrap_or_else(|| "none".into()),
            failure
                .attempt_id
                .as_deref()
                .map(|value| truncate_to(value, 18))
                .unwrap_or_else(|| "none".into()),
        )));
    }
    for (truncated, label) in [
        (snapshot.peers_truncated, "peer history"),
        (snapshot.attempts_truncated, "attempt history"),
        (snapshot.checkpoints_truncated, "checkpoint history"),
        (snapshot.failures_truncated, "failure history"),
    ] {
        if truncated {
            rows.push(Line::from(format!("Notice: {label} is truncated by the daemon.")));
        }
    }
    rows
}

fn wrap_propagation_rows(rows: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let mut wrapped = Vec::new();
    for row in rows {
        let text = row.spans.into_iter().fold(String::new(), |mut text, span| {
            text.push_str(&span.content);
            text
        });
        if text.is_empty() {
            wrapped.push(Line::from(""));
            continue;
        }
        let mut line = String::new();
        let mut line_width: usize = 0;
        for character in text.chars() {
            let character_width = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
            if !line.is_empty() && line_width.saturating_add(character_width) > width {
                wrapped.push(Line::from(std::mem::take(&mut line)));
                line_width = 0;
            }
            line.push(character);
            line_width = line_width.saturating_add(character_width);
        }
        wrapped.push(Line::from(line));
    }
    wrapped
}

fn truncate_to(s: &str, max: usize) -> String {
    crate::tui::widgets::truncate_str(s, max, "…")
}

#[cfg(test)]
mod unicode_regression_tests {
    use super::*;

    #[test]
    fn truncate_to_never_slices_inside_multibyte_text() {
        let text = "Styrene 𝗲phemeral identity loaded";
        let shortened = truncate_to(text, 24);
        assert!(shortened.ends_with('…'));
        assert!(unicode_width::UnicodeWidthStr::width(shortened.as_str()) <= 24);
    }

    #[test]
    fn compose_activity_accepts_styled_unicode() {
        let mut app = App::new();
        app.daemon_connected = true;
        app.connection_generation = Some(7);
        let mut capabilities = styrene_ipc::types::ActiveCapabilitiesInfo::default();
        capabilities.version = styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION;
        capabilities.authorized_operations = vec!["chat.send".into()];
        capabilities.runtime = vec!["runtime.lxmf.direct".into()];
        app.active_capabilities = Some(capabilities);
        app.handle_compose_submit("Styrene 𝗲phemeral identity loaded across the mesh".into());
        assert!(app.compose_pending.is_some());
    }

    #[test]
    fn propagation_rows_render_authoritative_domains_and_truncation() {
        let mut snapshot = styrene_ipc::types::StandardPropagationSnapshot::default();
        snapshot.version = styrene_ipc::types::STANDARD_PROPAGATION_SNAPSHOT_VERSION;
        snapshot.registered = true;
        snapshot.active = true;
        snapshot.connection_generation = Some(7);
        snapshot.queue.queued_count = 3;
        snapshot.queue.queued_bytes = 512;
        snapshot.failures_truncated = true;
        let mut selection = styrene_ipc::types::StandardPropagationSelectionInfo::default();
        selection.peer_hash = Some("0123456789abcdef0123456789abcdef".into());
        selection.mode = "automatic".into();
        snapshot.selection = Some(selection);

        let rows = standard_propagation_rows(Some(&snapshot), None);
        let narrow_rows = wrap_propagation_rows(rows.clone(), 20);
        assert!(narrow_rows.len() > rows.len());
        assert!(narrow_rows.last().unwrap().spans[0].content.contains("daemon."));
        let rendered = rows
            .into_iter()
            .flat_map(|line| line.spans)
            .map(|span| span.content.into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Status: active"));
        assert!(rendered.contains("Selected peer: 0123456789abcdef012…"));
        assert!(rendered.contains("queue: 3 items / 512 bytes"));
        assert!(rendered.contains("failure history is truncated"));
    }
}
