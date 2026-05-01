//! MockTunnelBackend — records all tunnel operations for assertion.

use std::collections::HashMap;
use std::sync::Mutex;

use styrene_tunnel::error::TunnelError;
use styrene_tunnel::traits::{TunnelBackend, TunnelId, TunnelInfo, TunnelParams, TunnelState};

/// A mock tunnel backend that records all operations without touching
/// any system networking. Used to validate PQC session → tunnel flow.
pub struct MockTunnelBackend {
    backend_name: String,
    available: Mutex<bool>,
    tunnels: Mutex<HashMap<TunnelId, TunnelInfo>>,
    counter: Mutex<u64>,
    /// Log of establish calls for assertion.
    pub establish_log: Mutex<Vec<TunnelParams>>,
    /// Log of rekey calls: (tunnel_id, new_psk).
    pub rekey_log: Mutex<Vec<(String, [u8; 32])>>,
}

impl MockTunnelBackend {
    pub fn new(name: &str) -> Self {
        Self {
            backend_name: name.to_string(),
            available: Mutex::new(true),
            tunnels: Mutex::new(HashMap::new()),
            counter: Mutex::new(0),
            establish_log: Mutex::new(Vec::new()),
            rekey_log: Mutex::new(Vec::new()),
        }
    }

    pub fn set_available(&self, available: bool) {
        *self.available.lock().expect("lock") = available;
    }

    pub fn establish_count(&self) -> usize {
        self.establish_log.lock().expect("lock").len()
    }

    pub fn last_psk(&self) -> Option<[u8; 32]> {
        self.establish_log
            .lock()
            .expect("lock")
            .last()
            .map(|p| p.psk)
    }
}

#[async_trait::async_trait]
impl TunnelBackend for MockTunnelBackend {
    fn name(&self) -> &str {
        &self.backend_name
    }

    async fn is_available(&self) -> bool {
        *self.available.lock().expect("lock")
    }

    async fn establish(&self, params: TunnelParams) -> Result<TunnelId, TunnelError> {
        let mut counter = self.counter.lock().expect("lock");
        *counter += 1;
        let id = format!("{}-{}", self.backend_name, counter);
        let info = TunnelInfo {
            id: id.clone(),
            backend: self.backend_name.clone(),
            peer_identity: params.peer_identity.clone(),
            peer_wg_pubkey: None,
            peer_mesh_ip: params.peer_mesh_ip.clone(),
            remote_endpoint: params.remote_endpoint,
            interface_name: Some(format!("mock-{}", counter)),
            state: TunnelState::Established,
            tx_bytes: 0,
            rx_bytes: 0,
            established_at: None,
            last_rekey: None,
        };
        self.tunnels.lock().expect("lock").insert(id.clone(), info);
        self.establish_log.lock().expect("lock").push(params);
        Ok(id)
    }

    async fn teardown(&self, tunnel_id: &str) -> Result<(), TunnelError> {
        self.tunnels
            .lock()
            .expect("lock")
            .remove(tunnel_id)
            .map(|_| ())
            .ok_or_else(|| TunnelError::Crypto("tunnel not found".into()))
    }

    async fn rekey(&self, tunnel_id: &str, new_psk: &[u8; 32]) -> Result<(), TunnelError> {
        if !self.tunnels.lock().expect("lock").contains_key(tunnel_id) {
            return Err(TunnelError::Crypto("tunnel not found".into()));
        }
        self.rekey_log
            .lock()
            .expect("lock")
            .push((tunnel_id.to_string(), *new_psk));
        Ok(())
    }

    async fn status(&self, tunnel_id: &str) -> Result<TunnelInfo, TunnelError> {
        self.tunnels
            .lock()
            .expect("lock")
            .get(tunnel_id)
            .cloned()
            .ok_or_else(|| TunnelError::Crypto("tunnel not found".into()))
    }

    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, TunnelError> {
        Ok(self
            .tunnels
            .lock()
            .expect("lock")
            .values()
            .cloned()
            .collect())
    }
}
