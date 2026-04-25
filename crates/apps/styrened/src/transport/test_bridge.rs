use crate::storage::messages::MessageRecord;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

type InboundHandler = Arc<dyn Fn(&MessageRecord) -> bool + Send + Sync>;
type BridgeMap = HashMap<String, InboundHandler>;

fn bridge() -> &'static Mutex<BridgeMap> {
    static BRIDGE: OnceLock<Mutex<BridgeMap>> = OnceLock::new();
    BRIDGE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[allow(dead_code)]
pub fn reset() {
    bridge().lock().expect("test bridge mutex").clear();
}

#[allow(dead_code)]
pub fn register(identity: impl Into<String>, on_inbound: InboundHandler) {
    bridge().lock().expect("test bridge mutex").insert(identity.into(), on_inbound);
}

pub fn deliver_outbound(record: &MessageRecord) -> bool {
    bridge()
        .lock()
        .expect("test bridge mutex")
        .get(&record.destination)
        .is_some_and(|handler| handler(record))
}
