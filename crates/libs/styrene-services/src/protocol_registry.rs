//! Protocol registry — pluggable per-type message handler dispatch.
//!
//! Replaces hardcoded `if/else` chains in the inbound worker with a
//! trait-based registry. Each protocol handler registers for one or more
//! protocol type strings and receives messages matching those types.
//!
//! ## Protocol Discrimination
//!
//! Inbound LXMF messages carry a `protocol` field in their fields dictionary.
//! The registry routes messages by this field to the registered handler.
//! Unmatched messages fall through to a default handler (typically chat).

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// An inbound message to be routed to a protocol handler.
#[derive(Debug, Clone)]
pub struct InboundMessage {
    /// Source peer identity hash.
    pub source_hash: String,
    /// Protocol type string (from LXMF fields["protocol"]).
    pub protocol: Option<String>,
    /// Message content (plaintext body).
    pub content: String,
    /// Raw LXMF fields (protocol-specific metadata).
    pub fields: HashMap<String, serde_json::Value>,
    /// Unix timestamp.
    pub timestamp: i64,
    /// Unique message ID.
    pub message_id: String,
}

/// Result of handling a protocol message.
#[derive(Debug)]
pub enum HandleResult {
    /// Message was handled successfully.
    Handled,
    /// Message was handled and a reply should be sent.
    Reply(String),
    /// Message was not handled by this handler (pass to next).
    NotHandled,
    /// Handler encountered an error.
    Error(String),
}

/// A protocol message handler.
///
/// Implementations handle specific protocol types (e.g., "chat", "styrene",
/// "terminal", "file_transfer"). Handlers are registered with the
/// [`ProtocolRegistry`] and receive messages matching their protocol type.
#[async_trait]
pub trait ProtocolHandler: Send + Sync {
    /// Human-readable protocol name for logging.
    fn name(&self) -> &str;

    /// Protocol type strings this handler responds to.
    /// A handler can register for multiple types (e.g., ["chat", "meshchat"]).
    fn protocol_types(&self) -> Vec<String>;

    /// Handle an inbound message.
    async fn handle(&self, message: &InboundMessage) -> HandleResult;
}

/// Registry of protocol handlers.
///
/// Routes inbound messages to the appropriate handler based on the
/// `protocol` field. If no handler matches, the default handler is used.
pub struct ProtocolRegistry {
    handlers: Mutex<Vec<Arc<dyn ProtocolHandler>>>,
    type_index: Mutex<HashMap<String, usize>>,
    default_handler: Mutex<Option<Arc<dyn ProtocolHandler>>>,
}

impl ProtocolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            handlers: Mutex::new(Vec::new()),
            type_index: Mutex::new(HashMap::new()),
            default_handler: Mutex::new(None),
        }
    }

    /// Register a protocol handler. Its protocol_types() are indexed for dispatch.
    pub async fn register(&self, handler: Arc<dyn ProtocolHandler>) {
        let mut handlers = self.handlers.lock().await;
        let idx = handlers.len();
        let types = handler.protocol_types();
        handlers.push(handler);

        let mut index = self.type_index.lock().await;
        for t in types {
            index.insert(t, idx);
        }
    }

    /// Set the default handler for messages with no matching protocol type.
    pub async fn set_default(&self, handler: Arc<dyn ProtocolHandler>) {
        let mut default = self.default_handler.lock().await;
        *default = Some(handler);
    }

    /// Dispatch an inbound message to the appropriate handler.
    pub async fn dispatch(&self, message: &InboundMessage) -> HandleResult {
        let handlers = self.handlers.lock().await;
        let index = self.type_index.lock().await;

        // Try protocol-specific handler
        if let Some(protocol) = &message.protocol {
            if let Some(&idx) = index.get(protocol) {
                if let Some(handler) = handlers.get(idx) {
                    return handler.handle(message).await;
                }
            }
        }

        // Fall through to default handler
        drop(handlers);
        drop(index);

        let default = self.default_handler.lock().await;
        if let Some(handler) = default.as_ref() {
            return handler.handle(message).await;
        }

        HandleResult::NotHandled
    }

    /// List registered protocol types.
    pub async fn registered_types(&self) -> Vec<String> {
        let index = self.type_index.lock().await;
        index.keys().cloned().collect()
    }

    /// Number of registered handlers.
    pub async fn handler_count(&self) -> usize {
        self.handlers.lock().await.len()
    }
}

impl Default for ProtocolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoHandler;

    #[async_trait]
    impl ProtocolHandler for EchoHandler {
        fn name(&self) -> &str {
            "echo"
        }
        fn protocol_types(&self) -> Vec<String> {
            vec!["echo".to_string(), "ping".to_string()]
        }
        async fn handle(&self, msg: &InboundMessage) -> HandleResult {
            HandleResult::Reply(format!("echo: {}", msg.content))
        }
    }

    struct ChatHandler;

    #[async_trait]
    impl ProtocolHandler for ChatHandler {
        fn name(&self) -> &str {
            "chat"
        }
        fn protocol_types(&self) -> Vec<String> {
            vec!["chat".to_string()]
        }
        async fn handle(&self, _msg: &InboundMessage) -> HandleResult {
            HandleResult::Handled
        }
    }

    fn test_message(protocol: Option<&str>, content: &str) -> InboundMessage {
        InboundMessage {
            source_hash: "aaa".to_string(),
            protocol: protocol.map(|s| s.to_string()),
            content: content.to_string(),
            fields: HashMap::new(),
            timestamp: 1000,
            message_id: "msg1".to_string(),
        }
    }

    #[tokio::test]
    async fn dispatch_to_registered_handler() {
        let registry = ProtocolRegistry::new();
        registry.register(Arc::new(EchoHandler)).await;

        let result = registry.dispatch(&test_message(Some("echo"), "hello")).await;
        match result {
            HandleResult::Reply(s) => assert_eq!(s, "echo: hello"),
            _ => panic!("expected Reply"),
        }
    }

    #[tokio::test]
    async fn dispatch_multi_type_handler() {
        let registry = ProtocolRegistry::new();
        registry.register(Arc::new(EchoHandler)).await;

        // "ping" also routes to EchoHandler
        let result = registry.dispatch(&test_message(Some("ping"), "test")).await;
        match result {
            HandleResult::Reply(s) => assert_eq!(s, "echo: test"),
            _ => panic!("expected Reply"),
        }
    }

    #[tokio::test]
    async fn dispatch_unknown_protocol_returns_not_handled() {
        let registry = ProtocolRegistry::new();
        registry.register(Arc::new(EchoHandler)).await;

        let result = registry.dispatch(&test_message(Some("unknown"), "test")).await;
        assert!(matches!(result, HandleResult::NotHandled));
    }

    #[tokio::test]
    async fn dispatch_no_protocol_uses_default() {
        let registry = ProtocolRegistry::new();
        registry.set_default(Arc::new(ChatHandler)).await;

        let result = registry.dispatch(&test_message(None, "hello")).await;
        assert!(matches!(result, HandleResult::Handled));
    }

    #[tokio::test]
    async fn dispatch_unknown_uses_default() {
        let registry = ProtocolRegistry::new();
        registry.register(Arc::new(EchoHandler)).await;
        registry.set_default(Arc::new(ChatHandler)).await;

        let result = registry.dispatch(&test_message(Some("unknown"), "test")).await;
        assert!(matches!(result, HandleResult::Handled));
    }

    #[tokio::test]
    async fn registered_types_lists_all() {
        let registry = ProtocolRegistry::new();
        registry.register(Arc::new(EchoHandler)).await;
        registry.register(Arc::new(ChatHandler)).await;

        let mut types = registry.registered_types().await;
        types.sort();
        assert_eq!(types, vec!["chat", "echo", "ping"]);
    }

    #[tokio::test]
    async fn handler_count() {
        let registry = ProtocolRegistry::new();
        assert_eq!(registry.handler_count().await, 0);
        registry.register(Arc::new(EchoHandler)).await;
        assert_eq!(registry.handler_count().await, 1);
    }

    #[tokio::test]
    async fn empty_registry_returns_not_handled() {
        let registry = ProtocolRegistry::new();
        let result = registry.dispatch(&test_message(Some("anything"), "test")).await;
        assert!(matches!(result, HandleResult::NotHandled));
    }
}
