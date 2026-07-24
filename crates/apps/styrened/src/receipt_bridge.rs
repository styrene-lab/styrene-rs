use crate::rpc::RpcDaemon;
use rns_core::transport::core_transport::{DeliveryReceipt, ReceiptHandler};
use rns_core::transport::receipt::{
    record_receipt_status, resolve_receipt_message_id,
    track_receipt_mapping as shared_track_receipt_mapping,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

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
