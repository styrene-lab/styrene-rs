//! ProtocolService — protocol registry and dispatch.
//!
//! Owns: 6.1 protocol registry, 6.2 StyreneProtocol, 6.3 ChatProtocol,
//! 6.4 wire models.
//! Package: G
//!
//! Wraps the async [`ProtocolRegistry`](styrene_services::protocol_registry::ProtocolRegistry)
//! from `styrene-services`, providing a `MessageRecord`-based dispatch
//! interface that the inbound worker uses.

use crate::storage::messages::MessageRecord;
use std::collections::HashMap;
use styrene_services::protocol_registry::{HandleResult, InboundMessage, ProtocolRegistry};

/// Re-export the handler trait for downstream use.
pub use styrene_services::protocol_registry::ProtocolHandler;

/// Service managing protocol handlers and inbound message routing.
///
/// Wraps `ProtocolRegistry` and converts `MessageRecord` → `InboundMessage`
/// for dispatch.
pub struct ProtocolService {
    registry: ProtocolRegistry,
}

impl ProtocolService {
    pub fn new() -> Self {
        Self { registry: ProtocolRegistry::new() }
    }

    /// Access the underlying async registry for handler registration.
    pub fn registry(&self) -> &ProtocolRegistry {
        &self.registry
    }

    /// Register a protocol handler.
    pub async fn register(&self, handler: std::sync::Arc<dyn ProtocolHandler>) {
        self.registry.register(handler).await;
    }

    /// Set the default handler for messages with no protocol field.
    pub async fn set_default(&self, handler: std::sync::Arc<dyn ProtocolHandler>) {
        self.registry.set_default(handler).await;
    }

    /// Route an inbound message to the appropriate protocol handler.
    ///
    /// Converts `MessageRecord` to `InboundMessage` and dispatches through
    /// the async registry.
    pub async fn dispatch_inbound(&self, record: &MessageRecord) -> bool {
        let inbound = record_to_inbound(record);
        match self.registry.dispatch(&inbound).await {
            HandleResult::Handled | HandleResult::Reply(_) => true,
            HandleResult::NotHandled | HandleResult::Error(_) => false,
        }
    }

    /// List registered protocol IDs.
    pub async fn registered_protocols(&self) -> Vec<String> {
        self.registry.registered_types().await
    }
}

impl Default for ProtocolService {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a `MessageRecord` to an `InboundMessage` for protocol dispatch.
fn record_to_inbound(record: &MessageRecord) -> InboundMessage {
    let protocol = record
        .fields
        .as_ref()
        .and_then(|f| f.get("protocol"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let fields: HashMap<String, serde_json::Value> = record
        .fields
        .as_ref()
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    InboundMessage {
        source_hash: record.source.clone(),
        protocol,
        content: record.content.clone(),
        fields,
        timestamp: record.timestamp,
        message_id: record.id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct TestHandler {
        id: String,
        called: Arc<AtomicBool>,
    }

    #[async_trait]
    impl ProtocolHandler for TestHandler {
        fn name(&self) -> &str {
            &self.id
        }

        fn protocol_types(&self) -> Vec<String> {
            vec![self.id.clone()]
        }

        async fn handle(&self, _msg: &InboundMessage) -> HandleResult {
            self.called.store(true, Ordering::SeqCst);
            HandleResult::Handled
        }
    }

    fn make_record_with_protocol(protocol: &str) -> MessageRecord {
        MessageRecord {
            id: "msg1".into(),
            source: "src".into(),
            destination: "dst".into(),
            title: String::new(),
            content: String::new(),
            timestamp: 1000,
            direction: "in".into(),
            fields: Some(serde_json::json!({"protocol": protocol})),
            receipt_status: None,
            read: false,
        }
    }

    #[tokio::test]
    async fn starts_with_no_protocols() {
        let svc = ProtocolService::new();
        assert!(svc.registered_protocols().await.is_empty());
    }

    #[tokio::test]
    async fn register_and_dispatch() {
        let svc = ProtocolService::new();
        let called = Arc::new(AtomicBool::new(false));
        svc.register(Arc::new(TestHandler { id: "test".into(), called: called.clone() })).await;

        let record = make_record_with_protocol("test");
        assert!(svc.dispatch_inbound(&record).await);
        assert!(called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn unknown_protocol_not_dispatched() {
        let svc = ProtocolService::new();
        let record = make_record_with_protocol("unknown");
        assert!(!svc.dispatch_inbound(&record).await);
    }

    #[tokio::test]
    async fn no_protocol_field_not_dispatched() {
        let svc = ProtocolService::new();
        let record = MessageRecord {
            id: "msg1".into(),
            source: "src".into(),
            destination: "dst".into(),
            title: String::new(),
            content: String::new(),
            timestamp: 1000,
            direction: "in".into(),
            fields: None,
            receipt_status: None,
            read: false,
        };
        assert!(!svc.dispatch_inbound(&record).await);
    }

    #[tokio::test]
    async fn registered_protocols_lists_ids() {
        let svc = ProtocolService::new();
        svc.register(Arc::new(TestHandler {
            id: "styrene".into(),
            called: Arc::new(AtomicBool::new(false)),
        }))
        .await;
        svc.register(Arc::new(TestHandler {
            id: "chat".into(),
            called: Arc::new(AtomicBool::new(false)),
        }))
        .await;

        let protos = svc.registered_protocols().await;
        assert_eq!(protos.len(), 2);
        assert!(protos.contains(&"styrene".into()));
        assert!(protos.contains(&"chat".into()));
    }
}
