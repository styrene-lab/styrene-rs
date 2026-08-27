//! Segment types and per-type rendering
#![allow(dead_code)]
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
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph, Wrap};

use super::theme::Theme;
use super::widgets;

// ═══════════════════════════════════════════════════════════════════
// Segment enum
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub enum Segment {
    /// Outbound LXMF message we sent.
    SentMessage {
        message_id: Option<String>,
        dest_hash: String,
        dest_name: Option<String>,
        text: String,
        delivery_status: DeliveryStatus,
        lifecycle: MessageLifecycle,
    },

    /// Inbound LXMF message received.
    ReceivedMessage {
        message_id: Option<String>,
        source_hash: String,
        source_name: Option<String>,
        title: Option<String>,
        text: String,
        timestamp: i64,
        lifecycle: MessageLifecycle,
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
    Unknown,
    Pending,
    Sending,
    Sent,
    Delivered,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageLifecycle {
    pub state: styrene_ipc::types::MessageLifecycleState,
    pub terminal_detail: Option<String>,
    pub requested_method: Option<String>,
    pub actual_method: Option<String>,
    pub fallback_reason: Option<String>,
    pub correlation_id: Option<String>,
    pub attempts: Vec<styrene_ipc::types::MessageAttemptInfo>,
    pub authentication: styrene_ipc::types::MessageAuthenticationState,
    pub stamp_state: styrene_ipc::types::MessageStampState,
    pub stamp_value: Option<u32>,
    pub stamp_cost: Option<u32>,
    pub evidence: Vec<styrene_ipc::types::MessageDeliveryEvidenceInfo>,
    pub attachments: Vec<styrene_ipc::types::AttachmentInfo>,
    pub propagation: Vec<styrene_ipc::types::MessagePropagationCorrelationInfo>,
}

impl From<&styrene_ipc::types::MessageInfo> for MessageLifecycle {
    fn from(message: &styrene_ipc::types::MessageInfo) -> Self {
        Self {
            state: message.lifecycle_state,
            terminal_detail: message.terminal_detail.clone(),
            requested_method: message.requested_delivery_method.clone(),
            actual_method: message.actual_delivery_method.clone(),
            fallback_reason: message.fallback_reason.clone(),
            correlation_id: message.correlation_id.clone(),
            attempts: message.attempts.clone(),
            authentication: message.authentication_state,
            stamp_state: message.stamp_state,
            stamp_value: message.stamp_value,
            stamp_cost: message.stamp_cost,
            evidence: message.delivery_evidence.clone(),
            attachments: message.attachments.clone(),
            propagation: message.propagation_correlations.clone(),
        }
    }
}

impl MessageLifecycle {
    fn detail_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("lifecycle: {:?}", self.state),
            format!(
                "terminal detail: {}",
                self.terminal_detail.as_deref().unwrap_or("Not reported")
            ),
            format!("authenticity: {:?}", self.authentication),
            format!("stamp state: {:?}", self.stamp_state),
            format!(
                "stamp cost: {}",
                self.stamp_cost
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "Not reported".into())
            ),
            format!(
                "stamp value: {}",
                self.stamp_value
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "Not reported".into())
            ),
            format!("requested: {}", self.requested_method.as_deref().unwrap_or("Not reported")),
            format!("actual: {}", self.actual_method.as_deref().unwrap_or("Not reported")),
            format!("fallback: {}", self.fallback_reason.as_deref().unwrap_or("Not reported")),
            format!("correlation: {}", self.correlation_id.as_deref().unwrap_or("Not reported")),
        ];
        if self.attempts.is_empty() {
            lines.push("attempts: none".into());
        } else {
            lines.extend(self.attempts.iter().map(|attempt| {
                format!(
                    "attempt #{} message={} state={} started={} deadline={}",
                    attempt.number,
                    attempt.message_id,
                    attempt.state,
                    attempt.started_unix_ms,
                    attempt.deadline_unix_ms
                )
            }));
        }
        if self.evidence.is_empty() {
            lines.push("delivery evidence: Not reported".into());
        } else {
            lines.extend(self.evidence.iter().map(|item| format!(
                "evidence kind={:?} hash={} representation={} state={:?} outcome={} attempt={} correlation={} observed={} terminal={}",
                item.kind, item.hash, item.representation, item.state,
                item.outcome.as_deref().unwrap_or("Not reported"),
                item.attempt.map(|value| value.to_string()).unwrap_or_else(|| "Not reported".into()),
                item.correlation_id.as_deref().unwrap_or("Not reported"), item.observed_at,
                item.terminal_at.map(|value| value.to_string()).unwrap_or_else(|| "Not reported".into()),
            )));
        }
        if self.attachments.is_empty() {
            lines.push("attachments: Not reported".into());
        } else {
            for attachment in &self.attachments {
                lines.push(format!(
                    "attachment name={} size={} checksum={} integrity={} availability={}",
                    attachment.name,
                    attachment.size,
                    attachment.checksum,
                    attachment.integrity,
                    attachment.availability
                ));
                if let Some(transfer) = attachment.transfer.as_deref() {
                    lines.push(format!("transfer resource={} progress={}/{} checksum_verified={} cancellable={} state={} error={}", transfer.resource_hash.as_deref().unwrap_or("Not reported"), transfer.transferred, transfer.total, transfer.checksum_verified, transfer.cancellable, transfer.state, transfer.error.as_deref().unwrap_or("Not reported")));
                }
            }
        }
        if self.propagation.is_empty() {
            lines.push("propagation: Not reported".into());
        } else {
            lines.extend(self.propagation.iter().map(|item| format!("propagation relation={} transient={} attempt={} peer={} state={} created={} updated={}", item.relation, item.transient_id, item.attempt_id.as_deref().unwrap_or("Not reported"), item.peer_hash.as_deref().unwrap_or("Not reported"), item.state, item.created_at, item.updated_at)));
        }
        lines
    }
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
            Self::Unknown => "?",
            Self::Pending => "○",
            Self::Sending => "◎",
            Self::Sent => "◉",
            Self::Delivered => "●",
            Self::Cancelled => "⊘",
            Self::Failed(_) => "✗",
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Height calculation — needed by ConvWidget for scroll math
// ═══════════════════════════════════════════════════════════════════

impl Segment {
    /// Compute the terminal rows this segment occupies at the given width.
    pub fn height(&self, width: u16, _t: &dyn Theme) -> u16 {
        let inner = width.saturating_sub(4); // borders + padding
        match self {
            Segment::SentMessage { text, lifecycle, .. } => {
                2 + wrapped_line_count(text, inner)
                    + lifecycle
                        .detail_lines()
                        .iter()
                        .map(|line| wrapped_line_count(line, inner))
                        .sum::<u16>()
            }
            Segment::ReceivedMessage { title, text, lifecycle, .. } => {
                let title_rows = u16::from(title.is_some());
                2 + title_rows
                    + wrapped_line_count(text, inner)
                    + lifecycle
                        .detail_lines()
                        .iter()
                        .map(|line| wrapped_line_count(line, inner))
                        .sum::<u16>()
            }
            Segment::ProtocolEvent { detail, .. } => 1 + wrapped_line_count(detail, inner).max(1),
            Segment::SystemEvent { text } => 1 + wrapped_line_count(text, inner),
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
            Segment::SentMessage {
                dest_hash, dest_name, text, delivery_status, lifecycle, ..
            } => render_sent(
                area,
                buf,
                t,
                dest_hash,
                dest_name.as_deref(),
                text,
                delivery_status,
                lifecycle,
            ),
            Segment::ReceivedMessage {
                message_id: _,
                source_hash,
                source_name,
                title,
                text,
                timestamp,
                lifecycle,
            } => {
                let ts = format_timestamp(*timestamp);
                render_received(
                    area,
                    buf,
                    t,
                    source_hash,
                    source_name.as_deref(),
                    title.as_deref(),
                    text,
                    &ts,
                    lifecycle,
                )
            }
            Segment::ProtocolEvent { kind, peer_hash, peer_name, detail } => render_protocol_event(
                area,
                buf,
                t,
                kind,
                peer_hash.as_deref(),
                peer_name.as_deref(),
                detail,
            ),
            Segment::SystemEvent { text } => render_system(area, buf, t, text),
            Segment::MeshEvent { icon, text } => render_mesh_event(area, buf, t, icon, text),
            Segment::ConvSeparator => render_separator(area, buf, t),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Per-type renderers
// ═══════════════════════════════════════════════════════════════════

#[allow(clippy::too_many_arguments)]
fn render_sent(
    area: Rect,
    buf: &mut Buffer,
    t: &dyn Theme,
    dest_hash: &str,
    dest_name: Option<&str>,
    text: &str,
    status: &DeliveryStatus,
    lifecycle: &MessageLifecycle,
) {
    let label = dest_name.unwrap_or(dest_hash);
    let short = &dest_hash[..dest_hash.len().min(8)];
    let status_icon = status.icon();
    let status_color = match status {
        DeliveryStatus::Delivered => t.success(),
        DeliveryStatus::Cancelled => t.muted(),
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
    let mut body = text.to_string();
    for line in lifecycle.detail_lines() {
        body.push('\n');
        body.push_str(&line);
    }
    Paragraph::new(body)
        .style(Style::default().fg(t.fg()))
        .wrap(Wrap { trim: false })
        .render(inner, buf);
}

#[allow(clippy::too_many_arguments)]
fn render_received(
    area: Rect,
    buf: &mut Buffer,
    t: &dyn Theme,
    source_hash: &str,
    source_name: Option<&str>,
    title: Option<&str>,
    text: &str,
    ts: &str,
    lifecycle: &MessageLifecycle,
) {
    let label = source_name.unwrap_or(source_hash);
    let short = &source_hash[..source_hash.len().min(8)];
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
            ttl,
            Style::default().fg(t.accent_bright()).add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(Span::styled(text, Style::default().fg(t.fg()))));
    lines.extend(
        lifecycle
            .detail_lines()
            .into_iter()
            .map(|line| Line::from(Span::styled(line, Style::default().fg(t.dim())))),
    );
    Paragraph::new(lines).wrap(Wrap { trim: false }).render(inner, buf);
}

fn render_protocol_event(
    area: Rect,
    buf: &mut Buffer,
    t: &dyn Theme,
    kind: &ProtocolEventKind,
    peer_hash: Option<&str>,
    peer_name: Option<&str>,
    detail: &str,
) {
    let icon = kind.icon();
    let icon_color = match kind {
        ProtocolEventKind::LinkEstablished
        | ProtocolEventKind::Receipt
        | ProtocolEventKind::ResourceComplete => t.success(),
        ProtocolEventKind::LinkClosed | ProtocolEventKind::ResourceFailed => t.error(),
        ProtocolEventKind::LinkStale => t.warning(),
        _ => t.accent_muted(),
    };
    let peer_label =
        peer_name.or(peer_hash).map(|s| format!(" {}", &s[..s.len().min(12)])).unwrap_or_default();
    let line = Line::from(vec![
        Span::styled(format!(" {icon}"), Style::default().fg(icon_color)),
        Span::styled(peer_label, Style::default().fg(t.muted())),
        Span::styled(format!("  {detail}"), Style::default().fg(t.dim())),
    ]);
    Paragraph::new(line).render(area, buf);
}

fn render_system(area: Rect, buf: &mut Buffer, t: &dyn Theme, text: &str) {
    let lines: Vec<Line> = text
        .lines()
        .map(|l| Line::from(Span::styled(format!("  {l}"), Style::default().fg(t.accent_muted()))))
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
    if width == 0 {
        return text.lines().count() as u16;
    }
    let w = width as usize;
    text.lines().map(|line| (line.len().max(1)).div_ceil(w) as u16).sum::<u16>().max(1)
}

fn format_timestamp(ts: i64) -> String {
    if ts == 0 {
        return String::new();
    }
    // Extract time-of-day from UTC epoch seconds
    let day_secs = ts % 86400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    format!("{h:02}:{m:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_details_display_every_authoritative_field() {
        let mut attempt = styrene_ipc::types::MessageAttemptInfo::default();
        attempt.message_id = "message-2".into();
        attempt.number = 2;
        attempt.started_unix_ms = 100;
        attempt.deadline_unix_ms = 200;
        attempt.state = "failed".into();
        let lifecycle = MessageLifecycle {
            requested_method: Some("opportunistic".into()),
            actual_method: Some("direct".into()),
            fallback_reason: Some("packet limit".into()),
            correlation_id: Some("send-1".into()),
            attempts: vec![attempt],
            ..MessageLifecycle::default()
        };

        let details = lifecycle.detail_lines();
        assert!(details.iter().any(|line| line == "authenticity: Unknown"));
        assert!(details.iter().any(|line| line == "stamp cost: Not reported"));
        assert!(details.iter().any(|line| {
            line == "attempt #2 message=message-2 state=failed started=100 deadline=200"
        }));
        assert!(details.iter().any(|line| line == "delivery evidence: Not reported"));
    }

    #[test]
    fn lifecycle_details_display_no_attempts_without_inventing_one() {
        let lifecycle = MessageLifecycle {
            requested_method: Some("paper".into()),
            actual_method: Some("paper".into()),
            fallback_reason: None,
            correlation_id: Some("paper-1".into()),
            attempts: Vec::new(),
            ..MessageLifecycle::default()
        };

        assert!(lifecycle.detail_lines().iter().any(|line| line == "attempts: none"));
    }
}
