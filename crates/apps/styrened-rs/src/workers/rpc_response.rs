//! RPC response handler — registers as a StyreneProtocol handler
//! to correlate incoming RPC responses with FleetService pending requests.

use crate::services::FleetService;
use crate::services::protocol::ProtocolHandler;
use crate::storage::messages::MessageRecord;
use std::sync::Arc;
use styrene_mesh::StyreneMessage;

/// Protocol handler that routes incoming Styrene RPC responses to FleetService.
pub struct RpcResponseHandler {
    fleet: Arc<FleetService>,
}

impl RpcResponseHandler {
    pub fn new(fleet: Arc<FleetService>) -> Self {
        Self { fleet }
    }
}

impl ProtocolHandler for RpcResponseHandler {
    fn protocol_id(&self) -> &str {
        "styrene"
    }

    fn handle(&self, record: &MessageRecord) -> bool {
        // Extract custom_data from fields (Styrene wire payload)
        let custom_data_hex = record
            .fields
            .as_ref()
            .and_then(|f| f.get("custom_data"))
            .and_then(|v| v.as_str());

        let Some(hex_data) = custom_data_hex else {
            return false;
        };

        let Ok(wire_bytes) = hex::decode(hex_data) else {
            eprintln!("[rpc-handler] invalid hex in custom_data");
            return false;
        };

        let Ok(message) = StyreneMessage::decode(&wire_bytes) else {
            eprintln!("[rpc-handler] failed to decode StyreneMessage");
            return false;
        };

        // Check if this is a response type
        use styrene_mesh::StyreneMessageType::*;
        match message.message_type {
            StatusResponse | ExecResult | RebootResult | ConfigUpdateResult => {
                self.fleet.handle_response(message)
            }
            _ => false, // Not a response — let other handlers process
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use styrene_mesh::{StyreneMessage, StyreneMessageType};

    #[test]
    fn handler_correlates_response_with_fleet() {
        let fleet = Arc::new(FleetService::new());
        let handler = RpcResponseHandler::new(fleet.clone());

        // Register a pending request
        let request_id = [99u8; 16];
        let (tx, _rx) = tokio::sync::oneshot::channel();
        fleet.pending.lock().unwrap().insert(request_id, crate::services::fleet::PendingRequest {
            tx,
            created_at: std::time::Instant::now(),
            dest_hash: "test".into(),
        });

        // Build a response wire message
        let response = StyreneMessage::with_request_id(
            StyreneMessageType::StatusResponse,
            request_id,
            &[],
        );
        let wire_bytes = response.encode();

        // Build a MessageRecord with the wire data in fields
        let record = MessageRecord {
            id: "msg1".into(),
            source: "peer".into(),
            destination: "me".into(),
            title: String::new(),
            content: String::new(),
            timestamp: 1000,
            direction: "in".into(),
            fields: Some(serde_json::json!({
                "protocol": "styrene",
                "custom_data": hex::encode(&wire_bytes),
            })),
            receipt_status: None,
            read: false,
        };

        assert!(handler.handle(&record));
        assert_eq!(fleet.pending_count(), 0); // Request was correlated
    }
}
