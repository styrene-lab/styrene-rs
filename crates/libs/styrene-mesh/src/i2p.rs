//! I2P proxy payloads — CBOR-serializable structures for
//! I2P HTTP proxy messages (0x84-0x88).
//!
//! These payloads are used over both RNS Channels (fast path) and
//! LXMF store-and-forward (degraded path) for proxying HTTP requests
//! to `.i2p` eepsites through the hub's i2pd router.

use serde::{Deserialize, Serialize};

/// I2P_PROXY_REQUEST (0x84) — client requests an HTTP fetch through the hub's i2pd.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I2pProxyRequest {
    /// HTTP method (GET, POST, HEAD, etc.).
    pub method: String,
    /// Full URL including `.i2p` host (e.g., `http://xyz.b32.i2p/path`).
    pub url: String,
    /// HTTP request headers.
    pub headers: Vec<(String, String)>,
    /// Optional request body (POST/PUT). Max 64KB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Vec<u8>>,
    /// Sequence number for multiplexing concurrent requests over one Channel.
    pub seq: u32,
}

/// I2P_PROXY_RESPONSE (0x85) — hub sends HTTP response headers back to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I2pProxyResponse {
    /// HTTP status code.
    pub status: u16,
    /// HTTP response headers.
    pub headers: Vec<(String, String)>,
    /// Correlating sequence number from the request.
    pub seq: u32,
    /// Total response body size in bytes (if known from Content-Length).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_size: Option<u64>,
    /// Total number of data chunks that will follow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_chunks: Option<u32>,
}

/// I2P_PROXY_DATA (0x86) — hub sends a chunk of the response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I2pProxyData {
    /// Correlating sequence number from the request.
    pub seq: u32,
    /// Zero-based chunk index for ordered reassembly.
    pub chunk_index: u32,
    /// Raw chunk data.
    pub data: Vec<u8>,
    /// True if this is the final chunk.
    pub final_chunk: bool,
}

/// I2P_PROXY_ERROR (0x87) — hub reports an error for a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I2pProxyError {
    /// Correlating sequence number from the request.
    pub seq: u32,
    /// HTTP-style error code (502 = i2pd unreachable, 504 = timeout, etc.).
    pub code: u16,
    /// Human-readable error description.
    pub message: String,
}

/// I2P_PROXY_CLOSE (0x88) — either side aborts an in-flight request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I2pProxyClose {
    /// Sequence number of the request to abort.
    pub seq: u32,
}

/// Maximum request body size (64KB).
pub const MAX_REQUEST_BODY: usize = 64 * 1024;

/// Maximum concurrent in-flight requests per session.
pub const MAX_CONCURRENT_REQUESTS: usize = 8;

/// Default idle timeout for proxy sessions (seconds).
pub const SESSION_IDLE_TIMEOUT_SECS: u64 = 300;

/// Default rate limit (requests per minute per identity).
pub const RATE_LIMIT_PER_MINUTE: u32 = 60;

/// Default hub-side timeout waiting for i2pd response (seconds).
pub const I2PD_RESPONSE_TIMEOUT_SECS: u64 = 120;

/// Default client-side local proxy bind address.
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:4480";

#[cfg(test)]
mod tests {
    use super::*;

    fn cbor_roundtrip<T: Serialize + for<'de> Deserialize<'de>>(val: &T) -> T {
        let mut buf = Vec::new();
        ciborium::into_writer(val, &mut buf).unwrap();
        ciborium::from_reader(&buf[..]).unwrap()
    }

    #[test]
    fn request_roundtrip() {
        let req = I2pProxyRequest {
            method: "GET".into(),
            url: "http://styrene.b32.i2p/".into(),
            headers: vec![("Accept".into(), "text/html".into())],
            body: None,
            seq: 0,
        };
        let decoded: I2pProxyRequest = cbor_roundtrip(&req);
        assert_eq!(decoded.method, "GET");
        assert_eq!(decoded.url, "http://styrene.b32.i2p/");
        assert_eq!(decoded.seq, 0);
        assert!(decoded.body.is_none());
    }

    #[test]
    fn response_roundtrip() {
        let resp = I2pProxyResponse {
            status: 200,
            headers: vec![("Content-Type".into(), "text/html".into())],
            seq: 0,
            total_size: Some(15234),
            total_chunks: Some(43),
        };
        let decoded: I2pProxyResponse = cbor_roundtrip(&resp);
        assert_eq!(decoded.status, 200);
        assert_eq!(decoded.total_size, Some(15234));
    }

    #[test]
    fn data_chunk_roundtrip() {
        let chunk = I2pProxyData {
            seq: 1,
            chunk_index: 5,
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
            final_chunk: true,
        };
        let decoded: I2pProxyData = cbor_roundtrip(&chunk);
        assert_eq!(decoded.seq, 1);
        assert_eq!(decoded.chunk_index, 5);
        assert!(decoded.final_chunk);
        assert_eq!(decoded.data, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn error_roundtrip() {
        let err = I2pProxyError { seq: 0, code: 502, message: "i2pd proxy unreachable".into() };
        let decoded: I2pProxyError = cbor_roundtrip(&err);
        assert_eq!(decoded.code, 502);
    }

    #[test]
    fn close_roundtrip() {
        let close = I2pProxyClose { seq: 3 };
        let decoded: I2pProxyClose = cbor_roundtrip(&close);
        assert_eq!(decoded.seq, 3);
    }
}
