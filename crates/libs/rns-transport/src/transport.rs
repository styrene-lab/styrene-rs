use core::fmt;
use std::fmt::Debug;
use std::future::ready;

#[derive(Clone, Debug, Default)]
pub struct TransportConfig {
    pub name: String,
    pub enable_receipts: bool,
}

impl TransportConfig {
    pub fn new(name: impl Into<String>, _identity: &impl Debug, enable_receipts: bool) -> Self {
        Self { name: name.into(), enable_receipts }
    }
}

#[derive(Clone, Debug)]
pub struct DeliveryReceipt {
    pub packet_id: Vec<u8>,
}

impl DeliveryReceipt {
    pub fn new(packet_id: impl Into<Vec<u8>>) -> Self {
        Self { packet_id: packet_id.into() }
    }
}

pub trait ReceiptHandler: Send + Sync {
    fn on_receipt(&self, _receipt: &DeliveryReceipt);
}

#[derive(Clone, Debug, Default)]
pub struct Transport {
    config: TransportConfig,
}

impl Transport {
    pub fn new(config: TransportConfig) -> Self {
        Self { config }
    }

    pub async fn set_receipt_handler(&mut self, _handler: Box<dyn ReceiptHandler>) {
        let _ = &self.config;
        ready(()).await
    }

    pub fn iface_manager(&self) -> InterfaceManager {
        InterfaceManager::default()
    }

    pub async fn add_destination(&self, _identity: impl Debug, _name: impl Into<String>) -> DestinationHandle {
        DestinationHandle(self.config.name.clone())
    }
}

#[derive(Clone, Debug, Default)]
pub struct InterfaceManager;

impl InterfaceManager {
    pub async fn spawn<P, F>(&self, _provider: P, _spawn: F) -> String
    where
        P: Send + 'static,
        F: Send + 'static,
    {
        "localhost".to_string()
    }
}

#[derive(Clone, Debug)]
pub struct DestinationHandle(pub String);

#[derive(Clone, Debug)]
pub enum SendPacketOutcome {
    Accepted,
    Rejected,
}

#[derive(Clone, Debug)]
pub enum ReceivedPayloadMode {
    Raw,
    Parsed,
}
