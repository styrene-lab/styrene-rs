use std::collections::HashSet;

use crate::error::LxmfError;
use crate::message::WireMessage;
use crate::reticulum::Adapter;

#[derive(Default)]
pub struct Router {
    outbound: Vec<WireMessage>,
    inbound: Vec<WireMessage>,
    delivered: HashSet<[u8; 32]>,
}

impl Router {
    pub fn with_adapter(_adapter: Adapter) -> Self {
        Self::default()
    }

    pub fn enqueue_outbound(&mut self, msg: WireMessage) {
        self.outbound.push(msg);
    }

    pub fn outbound_len(&self) -> usize {
        self.outbound.len()
    }

    pub fn handle_receipt_for_test(&mut self, message_id: [u8; 32]) {
        self.delivered.insert(message_id);
    }

    pub fn is_delivered(&self, message_id: &[u8; 32]) -> bool {
        self.delivered.contains(message_id)
    }

    pub fn send_for_test(&mut self, msg: WireMessage) -> Result<(), LxmfError> {
        self.outbound.push(msg.clone());
        self.inbound.push(msg);
        Ok(())
    }

    pub fn recv_for_test(&mut self) -> Option<WireMessage> {
        self.inbound.pop()
    }
}
