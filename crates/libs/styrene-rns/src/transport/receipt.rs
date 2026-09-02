use crate::transport::core_transport::DeliveryReceipt;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The receipt correlation map: packet hash to message id.
pub type ReceiptMap = Arc<Mutex<HashMap<String, String>>>;

static POISON_REPORTED: AtomicBool = AtomicBool::new(false);

/// Lock the correlation map, recovering it after a panic elsewhere. Entries
/// carry no cross-entry invariant, so the map a poisoned guard left behind
/// is complete and safe to keep. The poison flag is cleared so later direct
/// locks succeed; the recovery is reported once, without packet or message
/// identifiers.
pub fn lock_receipt_map(map: &ReceiptMap) -> MutexGuard<'_, HashMap<String, String>> {
    match map.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            map.clear_poison();
            if !POISON_REPORTED.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "receipt correlation mutex was poisoned by a panic; recovered {} entries",
                    poisoned.get_ref().len()
                );
            }
            PoisonError::into_inner(poisoned)
        }
    }
}

pub fn resolve_receipt_message_id(map: &ReceiptMap, receipt: &DeliveryReceipt) -> Option<String> {
    let key = hex::encode(receipt.message_id);
    lock_receipt_map(map).remove(&key)
}

pub fn lookup_receipt_message_id(map: &ReceiptMap, packet_hash: &str) -> Option<String> {
    lock_receipt_map(map).get(packet_hash).cloned()
}

pub fn track_receipt_mapping(map: &ReceiptMap, packet_hash: &str, message_id: &str) {
    lock_receipt_map(map).insert(packet_hash.to_string(), message_id.to_string());
}

pub fn prune_receipt_mappings_for_message(map: &ReceiptMap, message_id: &str) {
    lock_receipt_map(map).retain(|_, mapped_message_id| mapped_message_id != message_id);
}

pub trait ReceiptRecordSink {
    fn record_receipt_status(&self, message_id: &str, status: &str) -> std::io::Result<()>;
}

impl<F> ReceiptRecordSink for F
where
    F: Fn(&str, &str) -> std::io::Result<()>,
{
    fn record_receipt_status(&self, message_id: &str, status: &str) -> std::io::Result<()> {
        self(message_id, status)
    }
}

pub fn record_receipt_status(
    sink: &impl ReceiptRecordSink,
    message_id: &str,
    status: &str,
) -> Result<(), std::io::Error> {
    sink.record_receipt_status(message_id, status)
}

#[cfg(test)]
mod poison_tests {
    use super::*;

    fn poison(map: &ReceiptMap) {
        let shared = map.clone();
        let result = std::thread::spawn(move || {
            let _guard = shared.lock().expect("lock before poisoning");
            panic!("poison the receipt map");
        })
        .join();
        assert!(result.is_err());
        assert!(map.is_poisoned(), "the map must be poisoned before recovery");
    }

    #[test]
    fn receipt_operations_recover_after_a_panic_and_clear_poison() {
        let map: ReceiptMap = Arc::new(Mutex::new(HashMap::new()));
        track_receipt_mapping(&map, "aa", "message-a");
        track_receipt_mapping(&map, "bb", "message-b");
        poison(&map);

        track_receipt_mapping(&map, "cc", "message-c");
        assert_eq!(lookup_receipt_message_id(&map, "aa").as_deref(), Some("message-a"));
        let id = [0xbb; 32];
        track_receipt_mapping(&map, &hex::encode(id), "message-hex");
        let receipt = DeliveryReceipt::new(id);
        assert_eq!(resolve_receipt_message_id(&map, &receipt).as_deref(), Some("message-hex"));
        prune_receipt_mappings_for_message(&map, "message-b");
        assert_eq!(lookup_receipt_message_id(&map, "bb"), None);
        assert_eq!(lookup_receipt_message_id(&map, "cc").as_deref(), Some("message-c"));
        assert!(!map.is_poisoned());
        assert!(map.lock().is_ok());
    }
}
