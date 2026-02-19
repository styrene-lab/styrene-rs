//! RPC boundary crate for protocol and daemon contracts.

pub mod rpc {
    use serde_json::{self, Value};

    #[derive(Clone, Debug, PartialEq)]
    pub struct RpcRequest {
        pub id: i64,
        pub method: String,
        pub params: Value,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct RpcResponse {
        pub id: i64,
        pub result: Option<Value>,
        pub error: Option<String>,
    }

    #[derive(Clone, Debug, Default)]
    pub struct InterfaceRecord {
        pub kind: String,
        pub enabled: bool,
        pub host: Option<String>,
        pub port: Option<u16>,
        pub name: Option<String>,
    }

    #[derive(Clone, Debug, Default)]
    pub struct OutboundDeliveryOptions {
        pub include_receipt: bool,
    }

    pub trait OutboundBridge: Send + Sync {
        fn send_payload(&self, _request: RpcRequest, _options: &OutboundDeliveryOptions) {}
    }

    pub trait AnnounceBridge: Send + Sync {
        fn announce_now(&self) {}
    }

    #[derive(Clone, Debug, Default)]
    pub struct RpcEvent {
        pub event: String,
        pub payload: Value,
    }

    #[derive(Clone, Debug, Default)]
    pub struct RpcDaemon {
        pub identity_hash: String,
    }

    impl RpcDaemon {
        pub fn with_store(store: crate::storage::messages::MessagesStore, identity_hash: String) -> Self {
            let _ = store;
            Self { identity_hash }
        }

        pub fn with_store_and_bridge(
            store: crate::storage::messages::MessagesStore,
            identity_hash: String,
            _bridge: Option<std::sync::Arc<dyn OutboundBridge>>,
        ) -> Self {
            Self::with_store(store, identity_hash)
        }

        pub fn with_store_and_bridges(
            store: crate::storage::messages::MessagesStore,
            identity_hash: String,
            _outbound_bridge: Option<std::sync::Arc<dyn OutboundBridge>>,
            _announce_bridge: Option<std::sync::Arc<dyn AnnounceBridge>>,
        ) -> Self {
            Self::with_store(store, identity_hash)
        }

        pub fn test_instance() -> Self {
            Self::with_store(crate::storage::messages::MessagesStore::in_memory(), "test-identity".into())
        }

        pub fn test_instance_with_identity(identity: &str) -> Self {
            Self::test_instance().with_identity(identity)
        }

        pub fn with_identity(mut self, identity: &str) -> Self {
            self.identity_hash = identity.to_string();
            self
        }

        pub fn set_delivery_destination_hash(&self, _hash: Option<String>) {}
        pub fn replace_interfaces(&self, _interfaces: Vec<InterfaceRecord>) {}
        pub fn set_propagation_state(&self, _enabled: bool, _peer_count: Option<u64>, _node_count: u32) {}
        pub fn start_announce_scheduler(&self, _seconds: u64) -> Option<u64> {
            Some(1)
        }
        pub fn announce_destination_hash(&self) -> Option<String> {
            None
        }
    }

    pub mod codec {
        use super::RpcRequest;

        pub fn encode_frame(request: &RpcRequest) -> Result<Vec<u8>, std::io::Error> {
            Ok(request.method.clone().into_bytes())
        }

        pub fn decode_frame(bytes: &[u8]) -> Result<RpcRequest, std::io::Error> {
            Ok(RpcRequest { id: 0, method: String::from_utf8_lossy(bytes).to_string(), params: serde_json::json!(null) })
        }
    }

    pub mod http {
        pub fn find_header_end(header: &[u8]) -> Option<usize> {
            if header.windows(4).any(|window| window == b"\r\n\r\n") {
                Some(0)
            } else {
                None
            }
        }

        pub fn parse_content_length(_header: &[u8]) -> Option<usize> {
            None
        }

        pub fn handle_http_request(_daemon: &super::RpcDaemon, _body: &[u8]) -> Result<Vec<u8>, std::io::Error> {
            Ok(Vec::new())
        }

        pub fn build_error_response(_msg: &str) -> Vec<u8> {
            b"HTTP/1.1 500\r\nContent-Length: 2\r\n\r\nok".to_vec()
        }
    }

    pub use http::{build_error_response, find_header_end, parse_content_length};
    pub use codec::{decode_frame, encode_frame};

    pub fn handle_framed_request(daemon: &RpcDaemon, bytes: &[u8]) -> Result<Vec<u8>, std::io::Error> {
        let request = codec::decode_frame(bytes)?;
        let _ = daemon.identity_hash.as_str();
        codec::encode_frame(&request)
    }

    pub fn handle_http_request(daemon: &RpcDaemon, bytes: &[u8]) -> Result<Vec<u8>, std::io::Error> {
        http::handle_http_request(daemon, bytes)
    }
}

pub mod storage {
    pub mod messages {
        use serde_json::Value;

        #[derive(Clone, Debug, Default)]
        pub struct MessageRecord {
            pub id: String,
            pub destination_hash: String,
            pub payload: Vec<u8>,
            pub metadata: Option<Value>,
            pub status: Option<String>,
        }

        #[derive(Clone, Debug, Default)]
        pub struct AnnounceRecord {
            pub id: String,
            pub record: MessageRecord,
        }

        #[derive(Clone, Debug, Default)]
        pub struct MessagesStore;

        impl MessagesStore {
            pub fn in_memory() -> Self {
                Self
            }

            pub fn open(_path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
                Ok(Self)
            }
        }
    }
}

pub use rpc::*;
pub use storage::messages;
