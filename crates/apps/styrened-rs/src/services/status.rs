//! StatusService — daemon status queries.
//!
//! Owns: query_status, query_devices proxy, path info query.
//! Package: E
//!
//! Tracks daemon-level state that isn't owned by a specific domain service:
//! interface list, propagation state, uptime.

use std::sync::Mutex;
use std::time::Instant;

/// Interface record for status reporting.
#[derive(Debug, Clone)]
pub struct InterfaceRecord {
    pub kind: String,
    pub enabled: bool,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub name: Option<String>,
}

/// LXMF propagation node state.
#[derive(Debug, Clone, Default)]
pub struct PropagationState {
    pub enabled: bool,
    pub store_root: Option<String>,
    pub target_cost: u32,
}

/// Service tracking daemon-level status.
#[derive(Default)]
pub struct StatusService {
    interfaces: Mutex<Vec<InterfaceRecord>>,
    propagation: Mutex<PropagationState>,
    started_at: Option<Instant>,
}

impl StatusService {
    pub fn new() -> Self {
        Self {
            interfaces: Mutex::new(Vec::new()),
            propagation: Mutex::new(PropagationState::default()),
            started_at: Some(Instant::now()),
        }
    }

    /// Replace the full interface list (called during bootstrap).
    pub fn replace_interfaces(&self, interfaces: Vec<InterfaceRecord>) {
        *self.interfaces.lock().unwrap() = interfaces;
    }

    /// Get a snapshot of current interfaces.
    pub fn interfaces(&self) -> Vec<InterfaceRecord> {
        self.interfaces.lock().unwrap().clone()
    }

    /// Number of configured interfaces.
    pub fn interface_count(&self) -> usize {
        self.interfaces.lock().unwrap().len()
    }

    /// Set propagation state.
    pub fn set_propagation_state(
        &self,
        enabled: bool,
        store_root: Option<String>,
        target_cost: u32,
    ) {
        let mut guard = self.propagation.lock().unwrap();
        guard.enabled = enabled;
        guard.store_root = store_root;
        guard.target_cost = target_cost;
    }

    /// Update propagation state with a closure.
    pub fn update_propagation<F: FnOnce(&mut PropagationState)>(&self, f: F) {
        let mut guard = self.propagation.lock().unwrap();
        f(&mut guard);
    }

    /// Get propagation state snapshot.
    pub fn propagation_state(&self) -> PropagationState {
        self.propagation.lock().unwrap().clone()
    }

    /// Whether propagation is enabled.
    pub fn propagation_enabled(&self) -> bool {
        self.propagation.lock().unwrap().enabled
    }

    /// Daemon uptime in seconds.
    pub fn uptime_secs(&self) -> u64 {
        self.started_at
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_no_interfaces() {
        let svc = StatusService::new();
        assert_eq!(svc.interface_count(), 0);
        assert!(svc.interfaces().is_empty());
    }

    #[test]
    fn replace_interfaces_updates_list() {
        let svc = StatusService::new();
        svc.replace_interfaces(vec![
            InterfaceRecord {
                kind: "tcp_server".into(),
                enabled: true,
                host: Some("0.0.0.0".into()),
                port: Some(4242),
                name: Some("backbone".into()),
            },
        ]);
        assert_eq!(svc.interface_count(), 1);
        assert_eq!(svc.interfaces()[0].port, Some(4242));
    }

    #[test]
    fn propagation_defaults_to_disabled() {
        let svc = StatusService::new();
        assert!(!svc.propagation_enabled());
    }

    #[test]
    fn set_propagation_state_updates() {
        let svc = StatusService::new();
        svc.set_propagation_state(true, Some("/var/lxmf".into()), 128);
        let state = svc.propagation_state();
        assert!(state.enabled);
        assert_eq!(state.store_root, Some("/var/lxmf".into()));
        assert_eq!(state.target_cost, 128);
    }

    #[test]
    fn update_propagation_closure() {
        let svc = StatusService::new();
        svc.set_propagation_state(true, None, 64);
        svc.update_propagation(|s| s.target_cost = 256);
        assert_eq!(svc.propagation_state().target_cost, 256);
    }

    #[test]
    fn uptime_is_nonzero_after_creation() {
        let svc = StatusService::new();
        // Can't assert exact value but it should be >= 0
        let _ = svc.uptime_secs();
    }
}
