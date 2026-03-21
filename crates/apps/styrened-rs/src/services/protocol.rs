//! ProtocolService — protocol registry and dispatch.
//!
//! Owns: 6.1 protocol registry, 6.2 StyreneProtocol, 6.3 ChatProtocol,
//! 6.4 wire models.
//! Package: G
//!
//! Manages protocol handlers that process inbound LXMF messages based on
//! the `fields["protocol"]` discriminator. When a message arrives, the
//! ProtocolService routes it to the appropriate handler.
//!
//! Currently a registration skeleton — concrete protocol handlers
//! (StyreneProtocol, ChatProtocol) will be added as the inbound pipeline
//! is wired through MessagingService → ProtocolService.

use crate::storage::messages::MessageRecord;
use std::collections::HashMap;
use std::sync::Mutex;

/// A protocol handler processes inbound messages of a specific protocol type.
pub trait ProtocolHandler: Send + Sync {
    /// Protocol identifier (e.g., "styrene", "chat").
    fn protocol_id(&self) -> &str;

    /// Handle an inbound message. Returns true if the message was consumed.
    fn handle(&self, record: &MessageRecord) -> bool;
}

/// Service managing protocol handlers and inbound message routing.
pub struct ProtocolService {
    handlers: Mutex<HashMap<String, Box<dyn ProtocolHandler>>>,
}

impl ProtocolService {
    pub fn new() -> Self {
        Self {
            handlers: Mutex::new(HashMap::new()),
        }
    }

    /// Register a protocol handler.
    pub fn register(&self, handler: Box<dyn ProtocolHandler>) {
        let id = handler.protocol_id().to_string();
        self.handlers.lock().unwrap().insert(id, handler);
    }

    /// Route an inbound message to the appropriate protocol handler.
    ///
    /// Extracts the protocol field from the message's JSON fields,
    /// looks up the handler, and dispatches. Returns true if handled.
    pub fn dispatch(&self, record: &MessageRecord) -> bool {
        let protocol_id = record
            .fields
            .as_ref()
            .and_then(|f| f.get("protocol"))
            .and_then(|v| v.as_str());

        let Some(id) = protocol_id else {
            return false; // no protocol field — not a protocol message
        };

        let handlers = self.handlers.lock().unwrap();
        if let Some(handler) = handlers.get(id) {
            handler.handle(record)
        } else {
            false // no handler registered for this protocol
        }
    }

    /// Process an inbound message through protocol dispatch.
    /// Alias for `dispatch()` used by the inbound worker.
    pub fn dispatch_inbound(&self, record: &MessageRecord) -> bool {
        self.dispatch(record)
    }

    /// List registered protocol IDs.
    pub fn registered_protocols(&self) -> Vec<String> {
        self.handlers.lock().unwrap().keys().cloned().collect()
    }
}

impl Default for ProtocolService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct TestHandler {
        id: String,
        called: Arc<AtomicBool>,
    }

    impl ProtocolHandler for TestHandler {
        fn protocol_id(&self) -> &str {
            &self.id
        }

        fn handle(&self, _record: &MessageRecord) -> bool {
            self.called.store(true, Ordering::SeqCst);
            true
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

    #[test]
    fn starts_with_no_protocols() {
        let svc = ProtocolService::new();
        assert!(svc.registered_protocols().is_empty());
    }

    #[test]
    fn register_and_dispatch() {
        let svc = ProtocolService::new();
        let called = Arc::new(AtomicBool::new(false));
        svc.register(Box::new(TestHandler {
            id: "test".into(),
            called: called.clone(),
        }));

        let record = make_record_with_protocol("test");
        assert!(svc.dispatch(&record));
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn unknown_protocol_not_dispatched() {
        let svc = ProtocolService::new();
        let record = make_record_with_protocol("unknown");
        assert!(!svc.dispatch(&record));
    }

    #[test]
    fn no_protocol_field_not_dispatched() {
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
        assert!(!svc.dispatch(&record));
    }

    #[test]
    fn registered_protocols_lists_ids() {
        let svc = ProtocolService::new();
        svc.register(Box::new(TestHandler {
            id: "styrene".into(),
            called: Arc::new(AtomicBool::new(false)),
        }));
        svc.register(Box::new(TestHandler {
            id: "chat".into(),
            called: Arc::new(AtomicBool::new(false)),
        }));

        let protos = svc.registered_protocols();
        assert_eq!(protos.len(), 2);
        assert!(protos.contains(&"styrene".into()));
        assert!(protos.contains(&"chat".into()));
    }
}
