//! Propagation request handler — processes incoming propagation protocol
//! messages on hub nodes (ingest, fetch, delete).
//!
//! Registered as a ProtocolHandler for the "styrene" protocol type.
//! Only active on nodes with propagation enabled (Hub role).

use crate::services::{MessagingService, PropagationService};
use crate::transport::mesh_transport::MeshTransport;
use async_trait::async_trait;
use lxmf::inbound_decode::InboundPayloadMode;
use rns_core::destination::DestinationName;
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use std::sync::Arc;
use styrene_mesh::wire::{
    PropagationDeletePayload, PropagationFetchPayload, PropagationFetchResultPayload,
    PropagationIngestPayload, PropagationMessageEntry, PropagationStatusPayload,
};
use styrene_mesh::{StyreneMessage, StyreneMessageType};
use styrene_services::protocol_registry::{HandleResult, InboundMessage, ProtocolHandler};

/// Hub-side handler for propagation protocol messages.
pub struct PropagationRequestHandler {
    transport: Arc<dyn MeshTransport>,
    signer: Arc<PrivateIdentity>,
    propagation: Arc<PropagationService>,
    messaging: Arc<MessagingService>,
    local_delivery_hash: Option<String>,
}

impl PropagationRequestHandler {
    pub fn new(
        transport: Arc<dyn MeshTransport>,
        signer: Arc<PrivateIdentity>,
        propagation: Arc<PropagationService>,
        messaging: Arc<MessagingService>,
        local_delivery_hash: Option<String>,
    ) -> Self {
        Self { transport, signer, propagation, messaging, local_delivery_hash }
    }

    /// Build and send a response back to the source peer.
    async fn send_response(
        &self,
        source_hash: &str,
        request_id: [u8; 16],
        response_type: StyreneMessageType,
        payload: &[u8],
    ) -> Result<(), String> {
        let identity_bytes: [u8; 16] = hex::decode(source_hash)
            .map_err(|e| format!("invalid source hash: {e}"))?
            .try_into()
            .map_err(|_| "source hash must be 16 bytes".to_string())?;

        let delivery_addr = {
            let name = DestinationName::new("lxmf", "delivery");
            let mut combined = Vec::with_capacity(48);
            combined.extend_from_slice(name.as_name_hash_slice());
            combined.extend_from_slice(&identity_bytes);
            let truncated = rns_core::hash::address_hash(&combined);
            AddressHash::new(truncated)
        };
        let mut dest_bytes = [0u8; 16];
        dest_bytes.copy_from_slice(delivery_addr.as_slice());

        let wire_msg = StyreneMessage::with_request_id(response_type, request_id, payload);
        let wire_bytes = wire_msg.encode();

        let source_hash_addr = self.transport.identity_hash();
        let mut source_bytes = [0u8; 16];
        source_bytes.copy_from_slice(source_hash_addr.as_slice());

        let fields = serde_json::json!({
            "protocol": "styrene",
            "custom_type": "styrene.io",
            "custom_data": hex::encode(&wire_bytes),
        });

        let lxmf_payload = crate::lxmf_bridge::build_wire_message(
            source_bytes,
            dest_bytes,
            "",
            "",
            Some(fields),
            &self.signer,
        )
        .map_err(|e| format!("wire encode: {e}"))?;

        MessagingService::deliver(self.transport.as_ref(), delivery_addr, &lxmf_payload)
            .await
            .map_err(|e| format!("delivery failed: {e}"))?;

        Ok(())
    }

    fn cbor_encode<T: serde::Serialize>(value: &T) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(value, &mut buf).unwrap_or_default();
        buf
    }

    fn error_response(error: &str) -> Vec<u8> {
        Self::cbor_encode(&PropagationStatusPayload {
            success: false,
            error: Some(error.to_string()),
            count: None,
        })
    }

    fn handle_ingest(&self, payload: &[u8]) -> Vec<u8> {
        if !self.propagation.is_enabled() {
            return Self::error_response("propagation not enabled on this node");
        }

        let request: PropagationIngestPayload = match ciborium::from_reader(payload) {
            Ok(v) => v,
            Err(e) => return Self::error_response(&format!("invalid payload: {e}")),
        };

        // If dest_hash matches our own delivery hash, deliver locally instead of storing
        if let Some(ref local) = self.local_delivery_hash {
            if request.dest_hash == *local {
                let mut dest = [0u8; 16];
                if let Ok(bytes) = hex::decode(&request.dest_hash) {
                    if bytes.len() == 16 {
                        dest.copy_from_slice(&bytes);
                        if let Some(record) = self.messaging.accept_inbound(
                            dest,
                            &request.lxmf_bytes,
                            InboundPayloadMode::FullWire,
                        ) {
                            eprintln!(
                                "[propagation] ingest for local dest — delivered as id={}",
                                record.id
                            );
                            return Self::cbor_encode(&PropagationStatusPayload {
                                success: true,
                                error: None,
                                count: Some(1),
                            });
                        }
                    }
                }
                return Self::error_response("failed to deliver locally");
            }
        }

        match self.propagation.store_for_propagation(
            &request.dest_hash,
            &request.lxmf_bytes,
            request.source_hash.as_deref(),
        ) {
            Ok(stored) => {
                let count = if stored { 1 } else { 0 };
                eprintln!(
                    "[propagation] ingested for dest={} stored={}",
                    request.dest_hash, stored
                );
                Self::cbor_encode(&PropagationStatusPayload {
                    success: true,
                    error: None,
                    count: Some(count),
                })
            }
            Err(e) => Self::error_response(&format!("storage error: {e}")),
        }
    }

    fn handle_fetch(&self, payload: &[u8], caller_identity: &str) -> Vec<u8> {
        if !self.propagation.is_enabled() {
            return Self::error_response("propagation not enabled on this node");
        }

        let request: PropagationFetchPayload = match ciborium::from_reader(payload) {
            Ok(v) => v,
            Err(e) => return Self::error_response(&format!("invalid payload: {e}")),
        };

        // RBAC: caller can only fetch messages for their own delivery hash.
        let expected_delivery = {
            let id_bytes: [u8; 16] = match hex::decode(caller_identity)
                .and_then(|b| b.try_into().map_err(|_| hex::FromHexError::InvalidStringLength))
            {
                Ok(b) => b,
                Err(_) => return Self::error_response("invalid caller identity hash"),
            };
            let name = DestinationName::new("lxmf", "delivery");
            let mut combined = Vec::with_capacity(48);
            combined.extend_from_slice(name.as_name_hash_slice());
            combined.extend_from_slice(&id_bytes);
            hex::encode(rns_core::hash::address_hash(&combined))
        };

        if request.dest_hash != expected_delivery {
            eprintln!(
                "[propagation] DENIED fetch: caller={} requested={} expected={}",
                caller_identity, request.dest_hash, expected_delivery
            );
            return Self::error_response("permission denied: can only fetch your own messages");
        }

        match self.propagation.fetch_for_destination(&request.dest_hash) {
            Ok(messages) => {
                let entries: Vec<PropagationMessageEntry> = messages
                    .into_iter()
                    .map(|(id, lxmf_bytes)| PropagationMessageEntry { id, lxmf_bytes })
                    .collect();
                eprintln!(
                    "[propagation] fetch for dest={}: {} messages",
                    request.dest_hash,
                    entries.len()
                );
                Self::cbor_encode(&PropagationFetchResultPayload { messages: entries })
            }
            Err(e) => Self::error_response(&format!("fetch error: {e}")),
        }
    }

    fn handle_delete(&self, payload: &[u8]) -> Vec<u8> {
        if !self.propagation.is_enabled() {
            return Self::error_response("propagation not enabled on this node");
        }

        let request: PropagationDeletePayload = match ciborium::from_reader(payload) {
            Ok(v) => v,
            Err(e) => return Self::error_response(&format!("invalid payload: {e}")),
        };

        let count = request.ids.len();
        match self.propagation.delete_delivered(&request.ids) {
            Ok(()) => {
                eprintln!("[propagation] deleted {} messages", count);
                Self::cbor_encode(&PropagationStatusPayload {
                    success: true,
                    error: None,
                    count: Some(count),
                })
            }
            Err(e) => Self::error_response(&format!("delete error: {e}")),
        }
    }
}

#[async_trait]
impl ProtocolHandler for PropagationRequestHandler {
    fn name(&self) -> &str {
        "styrene-propagation-request"
    }

    fn protocol_types(&self) -> Vec<String> {
        vec!["styrene".to_string()]
    }

    async fn handle(&self, msg: &InboundMessage) -> HandleResult {
        let custom_data_hex = msg.fields.get("custom_data").and_then(|v| v.as_str());

        let Some(hex_data) = custom_data_hex else {
            return HandleResult::NotHandled;
        };

        let Ok(wire_bytes) = hex::decode(hex_data) else {
            return HandleResult::NotHandled;
        };

        let Ok(message) = StyreneMessage::decode(&wire_bytes) else {
            return HandleResult::NotHandled;
        };

        let source = &msg.source_hash;

        let (response_type, response_payload) = match message.message_type {
            StyreneMessageType::PropagationIngest => {
                (StyreneMessageType::PropagationIngestResult, self.handle_ingest(&message.payload))
            }
            StyreneMessageType::PropagationFetch => (
                StyreneMessageType::PropagationFetchResult,
                self.handle_fetch(&message.payload, source),
            ),
            StyreneMessageType::PropagationDelete => {
                (StyreneMessageType::PropagationDeleteResult, self.handle_delete(&message.payload))
            }
            _ => return HandleResult::NotHandled,
        };

        let request_id = message.request_id;
        let source = msg.source_hash.clone();

        match self.send_response(&source, request_id, response_type, &response_payload).await {
            Ok(()) => {
                eprintln!(
                    "[propagation] handled {:?} from {}, sent {:?}",
                    message.message_type, source, response_type
                );
                HandleResult::Handled
            }
            Err(e) => {
                eprintln!(
                    "[propagation] failed to send response for {:?} from {}: {}",
                    message.message_type, source, e
                );
                HandleResult::Error(e)
            }
        }
    }
}
