use crate::message::WireMessage;
use crate::reticulum::Adapter;
use crate::propagation::PropagationService;
use crate::storage::PropagationStore;
use std::path::Path;

#[derive(Default)]
pub struct Router {
    outbound: Vec<WireMessage>,
    propagation_service: Option<PropagationService>,
    propagation_ingested: usize,
    last_ingest_count: usize,
}

impl Router {
    pub fn with_adapter(_adapter: Adapter) -> Self {
        Self::default()
    }

    pub fn enqueue_outbound(&mut self, msg: WireMessage) {
        self.outbound.push(msg);
    }

    pub fn outbound_len(&self) -> usize {
        self.outbound.len()
    }

    pub fn dequeue_outbound(&mut self) -> Option<WireMessage> {
        if self.outbound.is_empty() {
            None
        } else {
            Some(self.outbound.remove(0))
        }
    }

    pub fn enable_propagation(&mut self, store_root: &Path, target_cost: u32) {
        let store = PropagationStore::new(store_root);
        self.propagation_service = Some(PropagationService::new(store, target_cost));
    }

    pub fn propagation_enabled(&self) -> bool {
        self.propagation_service.is_some()
    }

    pub fn ingest_propagation(&mut self, bytes: &[u8]) -> Result<usize, crate::error::LxmfError> {
        let Some(service) = &self.propagation_service else {
            return Ok(0);
        };

        let count = service.ingest(bytes)?;
        self.propagation_ingested += count;
        self.last_ingest_count = count;
        Ok(count)
    }

    pub fn fetch_propagated(&self, transient_id: &[u8]) -> Result<Vec<u8>, crate::error::LxmfError> {
        let Some(service) = &self.propagation_service else {
            return Err(crate::error::LxmfError::Io("propagation disabled".into()));
        };

        service.fetch(transient_id)
    }

    pub fn propagation_ingested_total(&self) -> usize {
        self.propagation_ingested
    }

    pub fn last_ingest_count(&self) -> usize {
        self.last_ingest_count
    }
}
