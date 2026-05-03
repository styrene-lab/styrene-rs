//! Page request handler — serves Micron pages to remote peers.
//!
//! Registered as a ProtocolHandler for the "styrene" protocol type.
//! When a remote node sends a PageRequest, this handler reads from
//! the local PageService and sends back a PageResponse.

use crate::services::{MessagingService, PageService};
use crate::transport::mesh_transport::MeshTransport;
use async_trait::async_trait;
use rns_core::destination::DestinationName;
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use std::sync::Arc;
use styrene_mesh::wire::{PageRequestPayload, PageResponsePayload};
use styrene_mesh::{StyreneMessage, StyreneMessageType};
use styrene_services::protocol_registry::{HandleResult, InboundMessage, ProtocolHandler};

/// Handler that serves Micron pages to remote peers via mesh RPC.
pub struct PageRequestHandler {
    transport: Arc<dyn MeshTransport>,
    signer: Arc<PrivateIdentity>,
    pages: Arc<PageService>,
}

impl PageRequestHandler {
    pub fn new(
        transport: Arc<dyn MeshTransport>,
        signer: Arc<PrivateIdentity>,
        pages: Arc<PageService>,
    ) -> Self {
        Self { transport, signer, pages }
    }

    /// Build and send a response back to the source peer.
    async fn send_response(
        &self,
        source_hash: &str,
        request_id: [u8; 16],
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

        let wire_msg =
            StyreneMessage::with_request_id(StyreneMessageType::PageResponse, request_id, payload);
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
}

#[async_trait]
impl ProtocolHandler for PageRequestHandler {
    fn name(&self) -> &str {
        "styrene-page-request"
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

        if message.message_type != StyreneMessageType::PageRequest {
            return HandleResult::NotHandled;
        }

        let request: PageRequestPayload = match ciborium::from_reader(&message.payload[..]) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[pages] invalid PageRequest payload: {e}");
                return HandleResult::NotHandled;
            }
        };

        // Serve the page from local PageService
        let content = self.pages.handle_request(&request.path);
        let source = String::from_utf8_lossy(&content).to_string();

        let response = if source.is_empty() {
            PageResponsePayload {
                success: false,
                source: String::new(),
                error: Some(format!("page not found: {}", request.path)),
            }
        } else {
            PageResponsePayload { success: true, source, error: None }
        };

        let response_payload = Self::cbor_encode(&response);
        let request_id = message.request_id;
        let source_hash = msg.source_hash.clone();

        match self.send_response(&source_hash, request_id, &response_payload).await {
            Ok(()) => {
                eprintln!(
                    "[pages] served {} to {} ({}b)",
                    request.path,
                    &source_hash[..8.min(source_hash.len())],
                    response.source.len()
                );
                HandleResult::Handled
            }
            Err(e) => {
                eprintln!("[pages] failed to send response: {e}");
                HandleResult::Error(e)
            }
        }
    }
}
