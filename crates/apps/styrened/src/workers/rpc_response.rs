//! RPC response handler — registers as a StyreneProtocol handler
//! to correlate incoming RPC responses with FleetService pending requests.

use crate::services::FleetService;
use async_trait::async_trait;
use std::sync::Arc;
use styrene_mesh::StyreneMessage;
use styrene_services::protocol_registry::{HandleResult, InboundMessage, ProtocolHandler};

/// Protocol handler that routes incoming Styrene RPC responses to FleetService.
pub struct RpcResponseHandler {
    fleet: Arc<FleetService>,
}

impl RpcResponseHandler {
    pub fn new(fleet: Arc<FleetService>) -> Self {
        Self { fleet }
    }
}

#[async_trait]
impl ProtocolHandler for RpcResponseHandler {
    fn name(&self) -> &str {
        "styrene-rpc-response"
    }

    fn protocol_types(&self) -> Vec<String> {
        vec!["styrene".to_string()]
    }

    async fn handle(&self, msg: &InboundMessage) -> HandleResult {
        // Extract custom_data from fields (Styrene wire payload)
        let custom_data_hex = msg.fields.get("custom_data").and_then(|v| v.as_str());

        let Some(hex_data) = custom_data_hex else {
            return HandleResult::NotHandled;
        };

        let Ok(wire_bytes) = hex::decode(hex_data) else {
            eprintln!("[rpc-handler] invalid hex in custom_data");
            return HandleResult::Error("invalid hex in custom_data".into());
        };

        let Ok(message) = StyreneMessage::decode(&wire_bytes) else {
            eprintln!("[rpc-handler] failed to decode StyreneMessage");
            return HandleResult::Error("failed to decode StyreneMessage".into());
        };

        // Check if this is a response type
        use styrene_mesh::StyreneMessageType::*;
        match message.message_type {
            StatusResponse
            | ExecResult
            | RebootResult
            | ConfigUpdateResult
            | SelfUpdateResult
            | InboxResponse
            | MessagesResponse
            | PropagationIngestResult
            | PropagationFetchResult
            | PropagationDeleteResult
            | PageResponse => {
                if self.fleet.handle_response(message, &msg.source_hash) {
                    HandleResult::Handled
                } else {
                    HandleResult::NotHandled
                }
            }
            _ => HandleResult::NotHandled, // Not a response — let other handlers process
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use styrene_mesh::{StyreneMessage, StyreneMessageType};

    #[tokio::test]
    async fn handler_correlates_response_with_fleet() {
        let fleet = Arc::new(FleetService::new());
        let handler = RpcResponseHandler::new(fleet.clone());

        // Register a pending request
        let request_id = [99u8; 16];
        let (tx, _rx) = tokio::sync::oneshot::channel();
        fleet.pending.lock().unwrap().insert(
            request_id,
            crate::services::fleet::PendingRequest {
                tx,
                created_at: std::time::Instant::now(),
                dest_hash: "peer".into(),
            },
        );

        // Build a response wire message
        let response =
            StyreneMessage::with_request_id(StyreneMessageType::StatusResponse, request_id, &[]);
        let wire_bytes = response.encode();

        // Build an InboundMessage with the wire data in fields
        let mut fields = std::collections::HashMap::new();
        fields.insert("protocol".to_string(), serde_json::json!("styrene"));
        fields.insert("custom_data".to_string(), serde_json::json!(hex::encode(&wire_bytes)));

        let msg = InboundMessage {
            source_hash: "peer".into(),
            protocol: Some("styrene".into()),
            content: String::new(),
            fields,
            timestamp: 1000,
            message_id: "msg1".into(),
        };

        assert!(matches!(handler.handle(&msg).await, HandleResult::Handled));
        assert_eq!(fleet.pending_count(), 0); // Request was correlated
    }
}
