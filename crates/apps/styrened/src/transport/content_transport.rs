//! MeshContentTransport — bridges `ContentTransport` trait to real mesh transport.
//!
//! Maps content distribution operations to StyreneMessage wire types
//! sent/received over the MeshTransport layer.

use std::sync::Arc;

use rns_core::hash::AddressHash;
use styrene_content::announce::ResourceAvailableAnnounce;
use styrene_content::chunk_bitset::ChunkBitset;
use styrene_content::content_id::ContentId;
use styrene_content::transport::{ContentEvent, ContentTransport};
use styrene_mesh::wire::{
    ChunkRequestPayload, ChunkResponsePayload, ResourceAvailablePayload, WireError,
};
use styrene_mesh::{StyreneMessage, StyreneMessageType};
use tokio::sync::broadcast;

use super::mesh_transport::MeshTransport;

/// ContentTransport implementation backed by real mesh wire protocol.
pub struct MeshContentTransport {
    transport: Arc<dyn MeshTransport>,
    inbound_rx: broadcast::Receiver<rns_core::transport::core_transport::ReceivedData>,
}

/// Errors from the mesh content transport.
#[derive(Debug)]
pub enum MeshContentError {
    Wire(WireError),
    Transport(String),
    Decode(String),
}

impl core::fmt::Display for MeshContentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Wire(e) => write!(f, "wire: {e}"),
            Self::Transport(e) => write!(f, "transport: {e}"),
            Self::Decode(e) => write!(f, "decode: {e}"),
        }
    }
}

impl MeshContentTransport {
    pub fn new(transport: Arc<dyn MeshTransport>) -> Self {
        let inbound_rx = transport.subscribe_inbound();
        Self { transport, inbound_rx }
    }
}

impl ContentTransport for MeshContentTransport {
    type Error = MeshContentError;

    async fn broadcast_announce(
        &mut self,
        announce: &ResourceAvailableAnnounce,
    ) -> Result<(), Self::Error> {
        let payload = ResourceAvailablePayload {
            content_id: *announce.content_id.as_bytes(),
            manifest_hash: announce.manifest_hash,
            chunks_held: announce.chunks_held.0.to_vec(),
            seeder_hash: announce.seeder_hash,
        };

        let payload_bytes = payload.encode().map_err(MeshContentError::Wire)?;
        let msg = StyreneMessage::new(StyreneMessageType::ResourceAvailable, &payload_bytes);
        let wire = msg.encode();

        // Broadcast via raw send (no link needed — announces are broadcast)
        let dest = self.transport.identity_hash(); // broadcast to all
        self.transport
            .send_raw(dest, &wire)
            .await
            .map_err(|e| MeshContentError::Transport(e.to_string()))?;

        Ok(())
    }

    async fn send_chunk_request(
        &mut self,
        seeder: &[u8; 16],
        content_id: ContentId,
        index: u32,
    ) -> Result<(), Self::Error> {
        let payload =
            ChunkRequestPayload { content_id: *content_id.as_bytes(), chunk_index: index };

        let payload_bytes = payload.encode().map_err(MeshContentError::Wire)?;
        let msg = StyreneMessage::new(StyreneMessageType::ChunkRequest, &payload_bytes);
        let wire = msg.encode();

        let dest = AddressHash::new(*seeder);
        self.transport
            .send_raw(dest, &wire)
            .await
            .map_err(|e| MeshContentError::Transport(e.to_string()))?;

        Ok(())
    }

    async fn send_chunk_response(
        &mut self,
        to: &[u8; 16],
        content_id: ContentId,
        index: u32,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        let payload = ChunkResponsePayload {
            content_id: *content_id.as_bytes(),
            chunk_index: index,
            data: data.to_vec(),
        };

        let payload_bytes = payload.encode().map_err(MeshContentError::Wire)?;
        let msg = StyreneMessage::new(StyreneMessageType::ChunkResponse, &payload_bytes);
        let wire = msg.encode();

        let dest = AddressHash::new(*to);
        self.transport
            .send_raw(dest, &wire)
            .await
            .map_err(|e| MeshContentError::Transport(e.to_string()))?;

        Ok(())
    }

    async fn recv_event(&mut self) -> Result<Option<ContentEvent>, Self::Error> {
        loop {
            match self.inbound_rx.recv().await {
                Ok(received) => {
                    let data = received.data.as_slice();

                    // Try to decode as StyreneMessage
                    let msg = match StyreneMessage::decode(data) {
                        Ok(m) => m,
                        Err(_) => continue, // Not a Styrene message — skip
                    };

                    match msg.message_type {
                        StyreneMessageType::ResourceAvailable => {
                            let p = ResourceAvailablePayload::decode(&msg.payload)
                                .map_err(MeshContentError::Wire)?;
                            let chunks_held = {
                                let mut arr = [0u8; 32];
                                let len = p.chunks_held.len().min(32);
                                arr[..len].copy_from_slice(&p.chunks_held[..len]);
                                ChunkBitset(arr)
                            };
                            return Ok(Some(ContentEvent::Announce(
                                ResourceAvailableAnnounce::new(
                                    ContentId::from_raw(p.content_id),
                                    p.manifest_hash,
                                    chunks_held,
                                    p.seeder_hash,
                                ),
                            )));
                        }
                        StyreneMessageType::ChunkRequest => {
                            let p = ChunkRequestPayload::decode(&msg.payload)
                                .map_err(MeshContentError::Wire)?;
                            // Extract sender from received data context
                            let mut from = [0u8; 16];
                            from.copy_from_slice(received.destination.as_slice());
                            return Ok(Some(ContentEvent::ChunkRequest {
                                from,
                                content_id: ContentId::from_raw(p.content_id),
                                index: p.chunk_index,
                            }));
                        }
                        StyreneMessageType::ChunkResponse => {
                            let p = ChunkResponsePayload::decode(&msg.payload)
                                .map_err(MeshContentError::Wire)?;
                            return Ok(Some(ContentEvent::ChunkResponse {
                                content_id: ContentId::from_raw(p.content_id),
                                index: p.chunk_index,
                                data: p.data,
                            }));
                        }
                        _ => continue, // Not a content message — skip
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return Ok(None),
            }
        }
    }
}
