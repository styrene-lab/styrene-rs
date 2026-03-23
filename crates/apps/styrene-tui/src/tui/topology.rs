#![allow(dead_code)]
//! Topology sidebar — peer tree with link children.
//!
//! Rendered as a right-side panel when terminal width >= 120 columns.
//! Uses `tui-tree-widget` for interactive expand/collapse navigation.
//!
//! Tree structure:
//!   ◉ node-1  a3b7f2d1…   (peer — online)
//!   ├── ⟺ link a3b7f2d1  12.4ms  (link — active)
//!   ◎ node-2  ff00cafe…   (peer — stale)
//!   ○ node-3  deadbeef…   (peer — offline)

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use super::theme::Theme;
use crate::mesh_state::{LinkRecord, LinkStatus, PeerRecord, PeerStatus};

/// Sidebar state — tree selection + expand state.
pub struct TopologyState {
    pub tree_state: TreeState<String>,
    pub sidebar_active: bool,
}

impl TopologyState {
    pub fn new() -> Self {
        Self {
            tree_state: TreeState::default(),
            sidebar_active: false,
        }
    }

    pub fn toggle_active(&mut self) {
        self.sidebar_active = !self.sidebar_active;
        if self.sidebar_active && self.tree_state.selected().is_empty() {
            self.tree_state.select_first();
        }
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Up => { self.tree_state.key_up(); true }
            KeyCode::Down => { self.tree_state.key_down(); true }
            KeyCode::Left => { self.tree_state.key_left(); true }
            KeyCode::Right => { self.tree_state.key_right(); true }
            KeyCode::Enter => { self.tree_state.toggle_selected(); true }
            _ => false,
        }
    }

    /// Returns the selected peer hash, if any.
    pub fn selected_peer_hash<'a>(&self, peers: &'a [PeerRecord]) -> Option<&'a str> {
        let selected = self.tree_state.selected();
        let id = selected.first()?;
        // Peer IDs are their hash; link IDs are "link::<id>"
        if id.starts_with("link::") { return None; }
        peers.iter().find(|p| &p.hash == id).map(|p| p.hash.as_str())
    }
}

impl Default for TopologyState {
    fn default() -> Self { Self::new() }
}

// ─── Build tree items ─────────────────────────────────────────────────────────

fn peer_item<'a>(peer: &PeerRecord, links: &[LinkRecord], t: &dyn Theme) -> TreeItem<'a, String> {
    let status_color = match peer.status {
        PeerStatus::Online => t.peer_online(),
        PeerStatus::Stale => t.link_stale(),
        PeerStatus::Offline => t.peer_offline(),
    };
    let hop_str = if peer.hop_count > 1 {
        format!(" {}↗", peer.hop_count)
    } else {
        String::new()
    };

    let label_text = Text::from(Line::from(vec![
        Span::styled(
            format!("{} ", peer.status.icon()),
            Style::default().fg(status_color),
        ),
        Span::styled(
            peer.label().to_string(),
            Style::default().fg(t.fg()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}…{}", peer.short_hash(), hop_str),
            Style::default().fg(t.dim()),
        ),
    ]));

    // Link children
    let peer_links: Vec<&LinkRecord> = links.iter()
        .filter(|l| l.peer_hash == peer.hash)
        .collect();

    let children: Vec<TreeItem<String>> = peer_links.iter().map(|link| {
        let link_color = match link.status {
            LinkStatus::Active => t.link_active(),
            LinkStatus::Stale => t.link_stale(),
            _ => t.link_closed(),
        };
        let rtt_str = if link.rtt_ms > 0.0 {
            format!("  {:.0}ms", link.rtt_ms)
        } else {
            String::new()
        };
        let link_text = Text::from(Line::from(vec![
            Span::styled(
                format!("  {} ", link.status.icon()),
                Style::default().fg(link_color),
            ),
            Span::styled(link.short_id().to_string(), Style::default().fg(t.muted())),
            Span::styled(rtt_str, Style::default().fg(t.dim())),
        ]));
        TreeItem::new_leaf(format!("link::{}", link.id), link_text)
    }).collect();

    if children.is_empty() {
        TreeItem::new_leaf(peer.hash.clone(), label_text)
    } else {
        TreeItem::new(peer.hash.clone(), label_text, children)
            .expect("unique peer hash")
    }
}

// ─── Render ──────────────────────────────────────────────────────────────────

pub fn render(
    area: Rect,
    frame: &mut Frame,
    state: &mut TopologyState,
    peers: &[PeerRecord],
    links: &[LinkRecord],
    t: &dyn Theme,
) {
    // Clear to prevent conversation bleed-through on resize
    frame.render_widget(ratatui::widgets::Clear, area);

    let border_color = if state.sidebar_active { t.accent() } else { t.border_dim() };
    let block = Block::default()
        .title(Span::styled(" Topology ", Style::default().fg(t.muted())))
        .borders(Borders::LEFT | Borders::TOP)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.bg()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 4 || inner.height < 4 {
        return;
    }

    if peers.is_empty() {
        let hint = Paragraph::new(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("⬡", Style::default().fg(t.dim())),
            Span::styled(" no peers", Style::default().fg(t.dim())),
        ]));
        frame.render_widget(hint, inner);
        render_hint(inner, frame, t);
        return;
    }

    // Build tree items — sorted online first, then stale, then offline
    let mut sorted_peers: Vec<&PeerRecord> = peers.iter().collect();
    sorted_peers.sort_by_key(|p| match p.status {
        PeerStatus::Online => 0,
        PeerStatus::Stale => 1,
        PeerStatus::Offline => 2,
    });

    let items: Vec<TreeItem<String>> = sorted_peers.iter()
        .map(|p| peer_item(p, links, t))
        .collect();

    // Auto-open all peers on first render
    if state.tree_state.opened().is_empty() && !items.is_empty() {
        for item in &items {
            state.tree_state.open(vec![item.identifier().clone()]);
        }
    }

    let tree_style = Style::default()
        .fg(t.fg())
        .bg(t.bg());
    let highlight_style = Style::default()
        .fg(t.accent_bright())
        .bg(t.surface_bg())
        .add_modifier(Modifier::BOLD);

    let Ok(tree) = Tree::new(&items) else {
        // Duplicate IDs — shouldn't happen; render fallback
        let p = Paragraph::new("  [tree error]").style(Style::default().fg(t.error()));
        frame.render_widget(p, inner);
        return;
    };

    let tree = tree
        .highlight_style(highlight_style)
        .style(tree_style);

    frame.render_stateful_widget(tree, inner, &mut state.tree_state);

    // Scrollbar when content overflows
    let total_items = peers.len() + links.len();
    if total_items as u16 > inner.height {
        let mut scroll_state = ScrollbarState::new(total_items);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .style(Style::default().fg(t.border_dim())),
            inner,
            &mut scroll_state,
        );
    }
}

fn render_hint(area: Rect, frame: &mut Frame, t: &dyn Theme) {
    if area.height < 3 { return; }
    let hint_y = area.y + area.height - 1;
    let hint = Paragraph::new(Line::from(vec![
        Span::styled("  r ", Style::default().fg(t.accent())),
        Span::styled("announce", Style::default().fg(t.dim())),
    ]));
    frame.render_widget(hint, Rect { y: hint_y, height: 1, ..area });
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::StyreneTheme;

    #[test]
    fn empty_peers_does_not_panic() {
        let mut state = TopologyState::new();
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        let mut frame_buf = ratatui::buffer::Buffer::empty(area);
        // Just verify the state functions don't panic
        let _ = state.selected_peer_hash(&[]);
        state.toggle_active();
        assert!(state.sidebar_active);
        state.toggle_active();
        assert!(!state.sidebar_active);
    }

    #[test]
    fn peer_item_renders_leaf_without_links() {
        let peer = PeerRecord::new("aabbccdd11223344".into(), Some("test-node".into()), 1000);
        let links: Vec<LinkRecord> = vec![];
        let item = peer_item(&peer, &links, &StyreneTheme);
        assert_eq!(item.identifier(), "aabbccdd11223344");
        assert!(item.children().is_empty());
    }

    #[test]
    fn peer_item_renders_with_link_children() {
        let peer = PeerRecord::new("aabbccdd11223344".into(), Some("test-node".into()), 1000);
        let link = LinkRecord::new(
            "linkid0011223344".into(),
            "aabbccdd11223344".into(),
            Some("test-node".into()),
            1000,
        );
        let item = peer_item(&peer, &[link], &StyreneTheme);
        assert_eq!(item.children().len(), 1);
    }

    #[test]
    fn topology_state_key_handling() {
        let mut state = TopologyState::new();
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        // Key handling should not panic with empty tree
        let _ = state.handle_key(up);
    }
}
