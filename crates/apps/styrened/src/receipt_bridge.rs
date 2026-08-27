use crate::rpc::RpcDaemon;
use crate::services::MessagingService;
use rns_core::transport::core_transport::{DeliveryReceipt, ReceiptHandler};
use rns_core::transport::receipt::{
    record_receipt_status, resolve_receipt_message_id,
    track_receipt_mapping as shared_track_receipt_mapping,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc::UnboundedSender, oneshot};

#[derive(Debug, Clone)]
pub struct ReceiptEvent {
    pub message_id: String,
    pub status: String,
}

pub type ReceiptWaiters = Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>;

pub fn register_receipt_waiter(
    waiters: &ReceiptWaiters,
    message_id: &str,
) -> oneshot::Receiver<String> {
    let (tx, rx) = oneshot::channel();
    waiters.lock().expect("receipt waiters").insert(message_id.to_string(), tx);
    rx
}

#[derive(Clone)]
pub struct ReceiptBridge {
    map: Arc<Mutex<HashMap<String, String>>>,
    waiters: ReceiptWaiters,
    tx: UnboundedSender<ReceiptEvent>,
}

impl ReceiptBridge {
    pub fn new(
        map: Arc<Mutex<HashMap<String, String>>>,
        waiters: ReceiptWaiters,
        tx: UnboundedSender<ReceiptEvent>,
    ) -> Self {
        Self { map, waiters, tx }
    }
}

impl ReceiptHandler for ReceiptBridge {
    fn on_receipt(&self, receipt: &DeliveryReceipt) {
        let message_id = resolve_receipt_message_id(&self.map, receipt);
        if let Some(message_id) = message_id {
            if let Some(waiter) =
                self.waiters.lock().ok().and_then(|mut guard| guard.remove(&message_id))
            {
                let _ = waiter.send("delivered".to_string());
            }
            let _ = self.tx.send(ReceiptEvent { message_id, status: "delivered".into() });
        }
    }
}

pub struct ServiceReceiptBridge {
    target: Arc<std::sync::OnceLock<std::sync::Weak<MessagingService>>>,
}

#[derive(Clone)]
pub struct PacketReceiptBridge {
    tx: broadcast::Sender<[u8; 32]>,
}

impl PacketReceiptBridge {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self { tx }
    }

    pub fn sender(&self) -> broadcast::Sender<[u8; 32]> {
        self.tx.clone()
    }
}

impl Default for PacketReceiptBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl ReceiptHandler for PacketReceiptBridge {
    fn on_receipt(&self, receipt: &DeliveryReceipt) {
        let _ = self.tx.send(receipt.message_id);
    }
}

impl ServiceReceiptBridge {
    pub fn new(target: Arc<std::sync::OnceLock<std::sync::Weak<MessagingService>>>) -> Self {
        Self { target }
    }
}

impl ReceiptHandler for ServiceReceiptBridge {
    fn on_receipt(&self, receipt: &DeliveryReceipt) {
        // Transport invokes this handler only after decoding and authenticating
        // the canonical Reticulum proof against its destination or link. For
        // the exact tracked LXMF packet, this is application delivery.
        let Some(messaging) = self.target.get().and_then(std::sync::Weak::upgrade) else {
            crate::daemon_diagnostic!("[receipt-bridge] service target is not initialized");
            return;
        };
        if let Err(error) =
            messaging.handle_packet_delivery_receipt(&hex::encode(receipt.message_id))
        {
            crate::daemon_diagnostic!("[receipt-bridge] service receipt error: {error}");
        }
    }
}

pub struct CompositeReceiptHandler {
    handlers: Vec<Box<dyn ReceiptHandler>>,
}

impl CompositeReceiptHandler {
    pub fn new(handlers: Vec<Box<dyn ReceiptHandler>>) -> Self {
        Self { handlers }
    }
}

impl ReceiptHandler for CompositeReceiptHandler {
    fn on_receipt(&self, receipt: &DeliveryReceipt) {
        for handler in &self.handlers {
            handler.on_receipt(receipt);
        }
    }
}

pub fn handle_receipt_event(daemon: &RpcDaemon, event: ReceiptEvent) -> Result<(), std::io::Error> {
    record_receipt_status(
        &|message_id: &str, status: &str| {
            let _ = daemon.handle_rpc(crate::rpc::RpcRequest {
                id: 0,
                method: "record_receipt".into(),
                params: Some(serde_json::json!({
                    "message_id": message_id,
                    "status": status,
                })),
            })?;
            Ok(())
        },
        &event.message_id,
        &event.status,
    )
}

pub fn track_receipt_mapping(
    map: &Arc<Mutex<HashMap<String, String>>>,
    packet_hash: &str,
    message_id: &str,
) {
    shared_track_receipt_mapping(map, packet_hash, message_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_receipt_bridge_fans_out_exact_authenticated_hash() {
        let bridge = PacketReceiptBridge::new();
        let mut receipts = bridge.sender().subscribe();
        bridge.on_receipt(&DeliveryReceipt::new([0x5a; 32]));
        assert_eq!(receipts.try_recv().unwrap(), [0x5a; 32]);
    }
}
