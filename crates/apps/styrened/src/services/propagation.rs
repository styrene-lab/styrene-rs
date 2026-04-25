//! PropagationService — LXMF store-and-forward for offline clients.
//!
//! When the daemon operates in Hub mode with propagation enabled, inbound
//! LXMF messages for non-local destinations are stored in SQLite for later
//! retrieval by the destination client.
//!
//! This service integrates into the inbound message pipeline:
//!   inbound LXMF → is destination local? → NO → propagation_store()
//!                                        → YES → normal delivery

use crate::storage::messages::MessagesStore;
use std::sync::{Arc, Mutex};

/// Default message expiry: 7 days.
const DEFAULT_EXPIRY_SECS: u64 = 7 * 24 * 3600;

/// Interval for expiry cleanup task.
const EXPIRY_CHECK_INTERVAL_SECS: u64 = 3600; // 1 hour

pub struct PropagationService {
    store: Arc<Mutex<MessagesStore>>,
    enabled: Mutex<bool>,
    expiry_secs: Mutex<u64>,
}

impl PropagationService {
    pub fn new(store: Arc<Mutex<MessagesStore>>) -> Self {
        Self { store, enabled: Mutex::new(false), expiry_secs: Mutex::new(DEFAULT_EXPIRY_SECS) }
    }

    /// Enable or disable propagation storage.
    pub fn set_enabled(&self, enabled: bool) {
        *self.enabled.lock().unwrap() = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        *self.enabled.lock().unwrap()
    }

    /// Set the message expiry duration.
    pub fn set_expiry_secs(&self, secs: u64) {
        *self.expiry_secs.lock().unwrap() = secs;
    }

    /// Store an LXMF message for later delivery to an offline client.
    ///
    /// Returns true if the message was stored (new), false if it was a duplicate.
    pub fn store_for_propagation(
        &self,
        dest_hash: &str,
        lxmf_bytes: &[u8],
        source_hash: Option<&str>,
    ) -> Result<bool, std::io::Error> {
        if !self.is_enabled() {
            return Ok(false);
        }

        let expiry = *self.expiry_secs.lock().unwrap();
        self.store
            .lock()
            .unwrap()
            .propagation_ingest(dest_hash, lxmf_bytes, source_hash, expiry)
            .map_err(std::io::Error::other)
    }

    /// Fetch stored messages for a destination.
    ///
    /// Returns (id, lxmf_bytes) pairs. After successful delivery, the caller
    /// should call `delete_delivered` with the IDs.
    pub fn fetch_for_destination(
        &self,
        dest_hash: &str,
    ) -> Result<Vec<(String, Vec<u8>)>, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .propagation_fetch_with_ids(dest_hash)
            .map_err(std::io::Error::other)
    }

    /// Delete messages that have been successfully delivered.
    pub fn delete_delivered(&self, ids: &[String]) -> Result<(), std::io::Error> {
        self.store.lock().unwrap().propagation_delete(ids).map_err(std::io::Error::other)
    }

    /// Remove expired messages from the store.
    pub fn expire_old(&self) -> Result<usize, std::io::Error> {
        self.store.lock().unwrap().propagation_expire().map_err(std::io::Error::other)
    }

    /// Get propagation statistics (count, total_size_bytes).
    pub fn stats(&self) -> Result<(usize, u64), std::io::Error> {
        self.store.lock().unwrap().propagation_stats().map_err(std::io::Error::other)
    }
}

/// Spawn a background task that periodically cleans up expired propagation messages.
pub fn spawn_expiry_task(service: Arc<PropagationService>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let interval = tokio::time::Duration::from_secs(EXPIRY_CHECK_INTERVAL_SECS);
        loop {
            tokio::time::sleep(interval).await;
            if service.is_enabled() {
                match service.expire_old() {
                    Ok(0) => {}
                    Ok(n) => eprintln!("[propagation] expired {n} stale messages"),
                    Err(e) => eprintln!("[propagation] expiry error: {e}"),
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_service() -> PropagationService {
        let store = MessagesStore::in_memory().unwrap();
        PropagationService::new(Arc::new(Mutex::new(store)))
    }

    #[test]
    fn disabled_by_default() {
        let svc = test_service();
        assert!(!svc.is_enabled());
    }

    #[test]
    fn store_when_enabled() {
        let svc = test_service();
        svc.set_enabled(true);

        let stored =
            svc.store_for_propagation("abc123", b"lxmf-payload-data", Some("source456")).unwrap();
        assert!(stored);

        let (count, _size) = svc.stats().unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn skip_when_disabled() {
        let svc = test_service();
        // Not enabled — should not store
        let stored = svc.store_for_propagation("abc123", b"payload", None).unwrap();
        assert!(!stored);

        let (count, _) = svc.stats().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn fetch_and_delete() {
        let svc = test_service();
        svc.set_enabled(true);

        svc.store_for_propagation("dest1", b"msg-a", None).unwrap();
        svc.store_for_propagation("dest1", b"msg-b", None).unwrap();
        svc.store_for_propagation("dest2", b"msg-c", None).unwrap();

        let msgs = svc.fetch_for_destination("dest1").unwrap();
        assert_eq!(msgs.len(), 2);

        let ids: Vec<String> = msgs.iter().map(|(id, _)| id.clone()).collect();
        svc.delete_delivered(&ids).unwrap();

        let remaining = svc.fetch_for_destination("dest1").unwrap();
        assert_eq!(remaining.len(), 0);

        // dest2 still has its message
        let dest2_msgs = svc.fetch_for_destination("dest2").unwrap();
        assert_eq!(dest2_msgs.len(), 1);
    }

    #[test]
    fn deduplication() {
        let svc = test_service();
        svc.set_enabled(true);

        svc.store_for_propagation("dest1", b"same-payload", None).unwrap();
        svc.store_for_propagation("dest1", b"same-payload", None).unwrap();

        let (count, _) = svc.stats().unwrap();
        assert_eq!(count, 1); // Deduplicated by content hash
    }
}
