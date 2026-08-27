//! LXMF conversation state
#![allow(dead_code)]
// — segment list and push/mutation methods.
//!
//! Holds the data model. Rendering is handled by `conv_widget`.

use super::conv_widget::ConvState;
use super::segments::{DeliveryStatus, MessageLifecycle, ProtocolEventKind, Segment};

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

    pub fn last_sent_id(&self) -> Option<&str> {
        self.segments.iter().rev().find_map(|segment| match segment {
            Segment::SentMessage { message_id: Some(id), .. } => Some(id.as_str()),
            _ => None,
        })
    }

    pub fn contains_sent(&self, message_id: &str) -> bool {
        self.segments.iter().any(
            |segment| matches!(segment, Segment::SentMessage { message_id: Some(id), .. } if id == message_id),
        )
    }

    pub fn contains_message(&self, message_id: &str) -> bool {
        self.segments.iter().any(|segment| match segment {
            Segment::SentMessage { message_id: Some(id), .. }
            | Segment::ReceivedMessage { message_id: Some(id), .. } => id == message_id,
            _ => false,
        })
    }

    pub fn prepend_history(&mut self, messages: Vec<Segment>) {
        if messages.is_empty() {
            return;
        }
        let mut segments =
            Vec::with_capacity(messages.len() * 2 + usize::from(!self.segments.is_empty()));
        for message in messages {
            if !segments.is_empty() {
                segments.push(Segment::ConvSeparator);
            }
            segments.push(message);
        }
        if !self.segments.is_empty() {
            segments.push(Segment::ConvSeparator);
        }
        segments.append(&mut self.segments);
        self.segments = segments;
        self.conv_state.invalidate();
    }

    pub fn clear(&mut self) {
        self.segments.clear();
        self.conv_state.invalidate();
    }

    pub fn remove_sent(&mut self, message_id: &str) -> bool {
        let before = self.segments.len();
        self.segments.retain(|segment| {
            !matches!(segment, Segment::SentMessage { message_id: Some(id), .. } if id == message_id)
        });
        self.segments.dedup_by(|left, right| {
            matches!(left, Segment::ConvSeparator) && matches!(right, Segment::ConvSeparator)
        });
        if self.segments.len() != before {
            self.conv_state.invalidate();
            return true;
        }
        false
    }

    pub fn remove_message(&mut self, message_id: &str) -> bool {
        let before = self.segments.len();
        self.segments.retain(|segment| match segment {
            Segment::SentMessage { message_id: Some(id), .. }
            | Segment::ReceivedMessage { message_id: Some(id), .. } => id != message_id,
            _ => true,
        });
        self.segments.dedup_by(|left, right| {
            matches!(left, Segment::ConvSeparator) && matches!(right, Segment::ConvSeparator)
        });
        let changed = self.segments.len() != before;
        if changed {
            self.conv_state.invalidate();
        }
        changed
    }

    pub fn replace_sent(
        &mut self,
        message_id: &str,
        text: &str,
        status: DeliveryStatus,
        lifecycle: MessageLifecycle,
    ) -> bool {
        for segment in self.segments.iter_mut().rev() {
            if let Segment::SentMessage {
                message_id: Some(id),
                text: current_text,
                delivery_status,
                lifecycle: current_lifecycle,
                ..
            } = segment
                && id == message_id
            {
                *current_text = text.into();
                *delivery_status = status;
                *current_lifecycle = lifecycle;
                self.conv_state.invalidate();
                return true;
            }
        }
        false
    }

    pub fn replace_received(
        &mut self,
        message_id: &str,
        title: Option<&str>,
        text: &str,
        timestamp: i64,
        lifecycle: MessageLifecycle,
    ) -> bool {
        for segment in self.segments.iter_mut().rev() {
            if let Segment::ReceivedMessage {
                message_id: Some(id),
                title: current_title,
                text: current_text,
                timestamp: current_timestamp,
                lifecycle: current_lifecycle,
                ..
            } = segment
                && id == message_id
            {
                *current_title = title.map(str::to_owned);
                *current_text = text.into();
                *current_timestamp = timestamp;
                *current_lifecycle = lifecycle;
                self.conv_state.invalidate();
                return true;
            }
        }
        false
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
        self.push_sent_with_lifecycle(
            message_id,
            dest_hash,
            dest_name,
            text,
            status,
            MessageLifecycle::default(),
        );
    }

    pub fn push_sent_with_lifecycle(
        &mut self,
        message_id: Option<&str>,
        dest_hash: &str,
        dest_name: Option<&str>,
        text: &str,
        status: DeliveryStatus,
        lifecycle: MessageLifecycle,
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
            lifecycle,
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
        self.push_received_with_lifecycle(
            None,
            source_hash,
            source_name,
            title,
            text,
            timestamp,
            MessageLifecycle::default(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_received_with_lifecycle(
        &mut self,
        message_id: Option<&str>,
        source_hash: &str,
        source_name: Option<&str>,
        title: Option<&str>,
        text: &str,
        timestamp: i64,
        lifecycle: MessageLifecycle,
    ) {
        self.segments.push(Segment::ReceivedMessage {
            message_id: message_id.map(str::to_owned),
            source_hash: source_hash.to_string(),
            source_name: source_name.map(|s| s.to_string()),
            title: title.map(|s| s.to_string()),
            text: text.to_string(),
            timestamp,
            lifecycle,
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
            if let Segment::SentMessage { message_id: Some(candidate), delivery_status, .. } = seg
                && candidate == message_id
            {
                *delivery_status = status;
                self.conv_state.invalidate();
                return true;
            }
        }
        false
    }

    /// Correlate the most recent optimistic message with its daemon ID and status.
    pub fn acknowledge_last_sent(
        &mut self,
        message_id: Option<&str>,
        status: DeliveryStatus,
    ) -> bool {
        for seg in self.segments.iter_mut().rev() {
            if let Segment::SentMessage { message_id: candidate, delivery_status, .. } = seg
                && candidate.is_none()
            {
                *candidate = message_id.map(str::to_string);
                *delivery_status = status;
                self.conv_state.invalidate();
                return true;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_message_retains_authoritative_lifecycle() {
        let lifecycle = MessageLifecycle {
            requested_method: Some("opportunistic".into()),
            actual_method: Some("direct".into()),
            fallback_reason: Some("packet limit".into()),
            correlation_id: Some("send-1".into()),
            attempts: Vec::new(),
            ..MessageLifecycle::default()
        };
        let mut conversation = ConversationView::new();
        conversation.push_sent_with_lifecycle(
            Some("message"),
            "destination",
            None,
            "content",
            DeliveryStatus::Sent,
            lifecycle.clone(),
        );

        assert!(matches!(
            &conversation.segments()[0],
            Segment::SentMessage { lifecycle: retained, .. } if retained == &lifecycle
        ));
    }

    #[test]
    fn history_prepend_preserves_order_and_scroll_position() {
        let received = |text: &str| Segment::ReceivedMessage {
            message_id: None,
            source_hash: "peer".into(),
            source_name: None,
            title: None,
            text: text.into(),
            timestamp: 1,
            lifecycle: MessageLifecycle::default(),
        };
        let mut conversation = ConversationView::new();
        conversation.push_received("peer", None, None, "new", 3);
        conversation.scroll_up(4);
        conversation.prepend_history(vec![received("oldest"), received("older")]);

        let texts: Vec<_> = conversation
            .segments()
            .iter()
            .filter_map(|segment| match segment {
                Segment::ReceivedMessage { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["oldest", "older", "new"]);
        assert_eq!(conversation.conv_state.scroll_offset, 4);
        assert!(conversation.conv_state.user_scrolled);
    }
}
