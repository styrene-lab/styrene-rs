use crate::error::LxmfError;
use crate::message::WireMessage;
use crate::storage::Store;

pub struct PropagationNode {
    store: Box<dyn Store + Send + Sync>,
}

impl PropagationNode {
    pub fn new(store: Box<dyn Store + Send + Sync>) -> Self {
        Self { store }
    }

    pub fn store(&mut self, msg: WireMessage) -> Result<(), LxmfError> {
        self.store.save(&msg)
    }

    pub fn fetch(&self, id: &[u8; 32]) -> Result<WireMessage, LxmfError> {
        self.store.get(id)
    }
}
