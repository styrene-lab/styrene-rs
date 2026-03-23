//! Segment types and per-type rendering for the LXMF message view.
//!
//! Each segment renders as an independent widget. The ConvWidget
//! composes these into a scrollable view. Segment types map
//! directly to LXMF/mesh protocol events:
//!
//! - SentMessage     → outbound LXMF message (our sends)
//! - ReceivedMessage → inbound LXMF message
//! - ProtocolEvent   → link/announce/receipt/resource events
//! - SystemEvent     → daemon status, startup messages
//! - MeshEvent       → topology changes (peer found, link stale)
//! - ConvSeparator   → visual turn boundary between conversations

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, BorderType, Padding, Paragraph, Wrap};

use super::theme::Theme;
use super::widgets;

// ═══════════════════════════════════════════════════════════════════
// Segment enum
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub enum Segment {
    /// Outbound LXMF message we sent.
    SentMessage {
        dest_hash: String,
        dest_name: Option<String>,
        text: String,
        delivery_status: DeliveryStatus,
    },

    /// Inbound LXMF message received.
    ReceivedMessage {
        source_hash: String,
        source_name: Option<String>,
        title: Option<String>,
        text: String,
        timestamp: i64,
    },

    /// Protocol-layer event: link, announce, receipt, resource.
    ProtocolEvent {
        kind: ProtocolEventKind,
        peer_hash: Option<String>,
        peer_name: Option<String>,
        detail: String,
    },

    /// Daemon/system status message.
    SystemEvent { text: String },

    /// Mesh topology change (peer discovered, link stale, path found).
    MeshEvent { icon: String, text: String },

    /// Visual separator — marks conversation boundaries.
    ConvSeparator,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeliveryStatus {
    Pending,
    Sending,
    Sent,
    Delivered,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProtocolEventKind {
    Announce,
    LinkEstablished,
    LinkStale,
    LinkClosed,
    Receipt,
    ResourceStart,
    ResourceComplete,
    ResourceFailed,
    PropagationSync,
}

impl ProtocolEventKind {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Announce => "⬡",
            Self::LinkEstablished => "⟺",
            Self::LinkStale => "⟳",
            Self::LinkClosed => "✕",
            Self::Receipt => "✓",
            Self::ResourceStart => "⬇",
            Self::ResourceComplete => "✓",
            Self::ResourceFailed => "✗",
            Self::PropagationSync => "⟳",
        }
    }
}

impl DeliveryStatus {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Pending => "○",
            Self::Sending => "◎",
            Self::Sent => "◉",
            Self::Delivered => "●",
            Self::Failed(_) => "✗",
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Height calculation — needed by ConvWidget for scroll math
// ═══════════════════════════════════════════════════════════════════

impl Segment {
    /// Compute the terminal rows this segment occupies at the given width.
    pub fn height(&self, width: u16, t: &dyn Theme) -> u16 {
        let inner = width.saturating_sub(4); // borders + padding
        match self {
            Segment::SentMessage { text, .. } => {
                2 + wrapped_line_count(text, inner)
            }
            Segment::ReceivedMessage { title, text, .. } => {
                let title_lines = title.as_ref().map(|_| 1).unwrap_or(0);
                2 + title_lines + wrapped_line_count(text, inner)
            }
            Segment::ProtocolEvent { detail, .. } => {
                1 + wrapped_line_count(detail, inner).max(1)
            }
            Segment::SystemEvent { text } => {
                1 + wrapped_line_count(text, inner)
            }
            Segment::MeshEvent { text, .. } => {
                1 + wrapped_line_count(text, inner.saturating_sub(2))
            }
            Segment::ConvSeparator => 1,
        }
        .max(1)
    }

    // ─── Render dispatch ────────────────────────────────────────

    /// Render this segment into the given area.
    pub fn render(&self, area: Rect, buf: &mut Buffer, t: &dyn Theme) {
        match self {
            Segment::SentMessage { dest_hash, dest_name, text, delivery_status } => {
                render_sent(area, buf, t, dest_hash, dest_name.as_deref(), text, delivery_status)
            }
            Segment::ReceivedMessage { source_hash, source_name, title, text, timestamp } => {
                render_received(area, buf, t, source_hash, source_name.as_deref(),
                    title.as_deref(), text, *timestamp)
            }
            Segment::ProtocolEvent { kind, peer_hash, peer_name, detail } => {
                render_protocol_event(area, buf, t, kind, peer_hash.as_deref(),
                    peer_name.as_deref(), detail)
            }
            Segment::SystemEvent { text } => render_system(area, buf, t, text),
            Segment::MeshEvent { icon, text } => render_mesh_event(area, buf, t, icon, text),
            Segment::ConvSeparator => render_separator(area, buf, t),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Per-type renderers
// ═══════════════════════════════════════════════════════════════════

fn render_sent(
    area: Rect, buf: &mut Buffer, t: &dyn Theme,
    dest_hash: &str, dest_name: Option<&str>,
    text: &str, status: &DeliveryStatus,
) {
    let label = dest_name.unwrap_or(dest_hash);
    let short = &dest_hash[..dest_hash.len().min(8)];
    let status_icon = status.icon();
    let status_color = match status {
        DeliveryStatus::Delivered => t.success(),
        DeliveryStatus::Failed(_) => t.error(),
        DeliveryStatus::Sending => t.accent(),
        _ => t.muted(),
    };
    let title_line = Line::from(vec![
        Span::styled(" → ", Style::default().fg(t.accent())),
        Span::styled(label, Style::default().fg(t.fg()).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {short}…"), Style::default().fg(t.dim())),
        Span::styled(format!("  {status_icon}"), Style::default().fg(status_color)),
    ]);
    let block = Block::default()
        .title(title_line)
        .borders(Borders::LEFT)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(t.accent_muted()))
        .style(Style::default().bg(t.sent_msg_bg()))
        .padding(Padding::new(1, 1, 0, 0));
    let inner = block.inner(area);
    block.render(area, buf);
    Paragraph::new(text)
        .style(Style::default().fg(t.fg()))
        .wrap(Wrap { trim: false })
        .render(inner, buf);
}

fn render_received(
    area: Rect, buf: &mut Buffer, t: &dyn Theme,
    source_hash: &str, source_name: Option<&str>,
    title: Option<&str>, text: &str, timestamp: i64,
) {
    let label = source_name.unwrap_or(source_hash);
    let short = &source_hash[..source_hash.len().min(8)];
    let ts = format_timestamp(timestamp);
    let title_line = Line::from(vec![
        Span::styled(" ← ", Style::default().fg(t.success())),
        Span::styled(label, Style::default().fg(t.fg()).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {short}…"), Style::default().fg(t.dim())),
        Span::styled(format!("  {ts}"), Style::default().fg(t.dim())),
    ]);
    let block = Block::default()
        .title(title_line)
        .borders(Borders::LEFT)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(t.success()))
        .style(Style::default().bg(t.received_msg_bg()))
        .padding(Padding::new(1, 1, 0, 0));
    let inner = block.inner(area);
    block.render(area, buf);

    let mut lines = vec![];
    if let Some(ttl) = title {
        lines.push(Line::from(Span::styled(
            ttl, Style::default().fg(t.accent_bright()).add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(Span::styled(text, Style::default().fg(t.fg()))));
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .render(inner, buf);
}

fn render_protocol_event(
    area: Rect, buf: &mut Buffer, t: &dyn Theme,
    kind: &ProtocolEventKind, peer_hash: Option<&str>,
    peer_name: Option<&str>, detail: &str,
) {
    let icon = kind.icon();
    let icon_color = match kind {
        ProtocolEventKind::LinkEstablished | ProtocolEventKind::Receipt
            | ProtocolEventKind::ResourceComplete => t.success(),
        ProtocolEventKind::LinkClosed | ProtocolEventKind::ResourceFailed => t.error(),
        ProtocolEventKind::LinkStale => t.warning(),
        _ => t.accent_muted(),
    };
    let peer_label = peer_name
        .or(peer_hash)
        .map(|s| format!(" {}", &s[..s.len().min(12)]))
        .unwrap_or_default();
    let line = Line::from(vec![
        Span::styled(format!(" {icon}"), Style::default().fg(icon_color)),
        Span::styled(peer_label, Style::default().fg(t.muted())),
        Span::styled(format!("  {detail}"), Style::default().fg(t.dim())),
    ]);
    Paragraph::new(line).render(area, buf);
}

fn render_system(area: Rect, buf: &mut Buffer, t: &dyn Theme, text: &str) {
    let lines: Vec<Line> = text.lines()
        .map(|l| Line::from(Span::styled(
            format!("  {l}"),
            Style::default().fg(t.accent_muted()),
        )))
        .collect();
    Paragraph::new(lines).render(area, buf);
}

fn render_mesh_event(area: Rect, buf: &mut Buffer, t: &dyn Theme, icon: &str, text: &str) {
    let line = Line::from(vec![
        Span::styled(format!(" {icon} "), Style::default().fg(t.accent())),
        Span::styled(text, Style::default().fg(t.muted())),
    ]);
    Paragraph::new(line).render(area, buf);
}

fn render_separator(area: Rect, buf: &mut Buffer, t: &dyn Theme) {
    let line = widgets::section_divider("", area.width as usize, t);
    Paragraph::new(line).render(area, buf);
}

// ═══════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════

fn wrapped_line_count(text: &str, width: u16) -> u16 {
    if width == 0 { return text.lines().count() as u16; }
    text.lines()
        .map(|line| {
            let w = line.len().max(1);
            ((w + width as usize - 1) / width as usize).max(1) as u16
        })
        .sum::<u16>()
        .max(1)
}

fn format_timestamp(ts: i64) -> String {
    // Simple formatting: show time if today, else date
    if ts == 0 { return String::new(); }
    let secs = ts;
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    format!("{h:02}:{m:02}")
}
