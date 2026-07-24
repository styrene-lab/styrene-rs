//! LXMF conversation state
#![allow(dead_code)]
// — segment list and push/mutation methods.
//!
//! Holds the data model. Rendering is handled by `conv_widget`.

use super::conv_widget::ConvState;
use super::segments::{DeliveryStatus, ProtocolEventKind, Segment};

/// Active conversation view — segment list + scroll state.
pub struct ConversationView {
    segments: Vec<Segment>,
    pub conv_state: ConvState,
}

impl ConversationView {
    pub fn new() -> Self {
        Self { segments: Vec::new(), conv_state: ConvState::new() }
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn segments_and_state(&mut self) -> (&[Segment], &mut ConvState) {
        (&self.segments, &mut self.conv_state)
    }

    pub fn last_sent_status(&self) -> Option<&DeliveryStatus> {
        self.segments.iter().rev().find_map(|segment| match segment {
            Segment::SentMessage { delivery_status, .. } => Some(delivery_status),
            _ => None,
        })
    }

    // ─── Push methods ─────────────────────────────────────────────

    pub fn push_sent(
        &mut self,
        message_id: Option<&str>,
        dest_hash: &str,
        dest_name: Option<&str>,
        text: &str,
        status: DeliveryStatus,
    ) {
        if !self.segments.is_empty() {
            self.segments.push(Segment::ConvSeparator);
        }
        self.segments.push(Segment::SentMessage {
            message_id: message_id.map(str::to_string),
            dest_hash: dest_hash.to_string(),
            dest_name: dest_name.map(|s| s.to_string()),
            text: text.to_string(),
            delivery_status: status,
        });
        self.conv_state.invalidate();
        self.conv_state.force_scroll_to_bottom();
    }

    pub fn push_received(
        &mut self,
        source_hash: &str,
        source_name: Option<&str>,
        title: Option<&str>,
        text: &str,
        timestamp: i64,
    ) {
        self.segments.push(Segment::ReceivedMessage {
            source_hash: source_hash.to_string(),
            source_name: source_name.map(|s| s.to_string()),
            title: title.map(|s| s.to_string()),
            text: text.to_string(),
            timestamp,
        });
        self.conv_state.invalidate();
        self.conv_state.auto_scroll_to_bottom();
    }

    pub fn push_protocol_event(
        &mut self,
        kind: ProtocolEventKind,
        peer_hash: Option<&str>,
        peer_name: Option<&str>,
        detail: &str,
    ) {
        self.segments.push(Segment::ProtocolEvent {
            kind,
            peer_hash: peer_hash.map(|s| s.to_string()),
            peer_name: peer_name.map(|s| s.to_string()),
            detail: detail.to_string(),
        });
        self.conv_state.invalidate();
        self.conv_state.auto_scroll_to_bottom();
    }

    pub fn push_system(&mut self, text: &str) {
        self.segments.push(Segment::SystemEvent { text: text.to_string() });
        self.conv_state.invalidate();
        self.conv_state.force_scroll_to_bottom();
    }

    pub fn push_mesh_event(&mut self, icon: &str, text: &str) {
        self.segments.push(Segment::MeshEvent { icon: icon.to_string(), text: text.to_string() });
        self.conv_state.invalidate();
        self.conv_state.auto_scroll_to_bottom();
    }

    /// Update delivery status on the sent message with the matching daemon ID.
    pub fn update_sent_status(&mut self, message_id: &str, status: DeliveryStatus) -> bool {
        for seg in self.segments.iter_mut().rev() {
            if let Segment::SentMessage { message_id: Some(candidate), delivery_status, .. } = seg {
                if candidate == message_id {
                    *delivery_status = status;
                    self.conv_state.invalidate();
                    return true;
                }
            }
        }
        false
    }

    /// Update the most recent optimistic message before a daemon ID is known.
    pub fn update_last_sent_status(&mut self, status: DeliveryStatus) {
        for seg in self.segments.iter_mut().rev() {
            if let Segment::SentMessage { delivery_status, .. } = seg {
                *delivery_status = status;
                self.conv_state.invalidate();
                return;
            }
        }
    }

    // ─── Scroll ───────────────────────────────────────────────────

    pub fn scroll_up(&mut self, n: u16) {
        self.conv_state.scroll_up(n);
    }
    pub fn scroll_down(&mut self, n: u16) {
        self.conv_state.scroll_down(n);
    }
}
