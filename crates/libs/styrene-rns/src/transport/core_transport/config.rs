use super::*;

impl TransportConfig {
    pub fn new<T: Into<String>>(name: T, identity: &PrivateIdentity, broadcast: bool) -> Self {
        Self {
            name: name.into(),
            identity: identity.clone(),
            broadcast,
            retransmit: false,
            announce_cache_capacity: 100_000,
            announce_retry_limit: 5,
            announce_queue_len: 64,
            announce_cap: 128,
            path_request_timeout_secs: 30,
            link_proof_timeout_secs: 600,
            link_idle_timeout_secs: 900,
            resource_retry_interval_secs: 2,
            resource_retry_limit: 5,
            ratchet_store_path: None,
            blackholed_identities: HashSet::new(),
            ingress_queue_capacities: IngressQueueCapacities::default(),
        }
    }

    pub fn set_retransmit(&mut self, retransmit: bool) {
        self.retransmit = retransmit;
    }
    pub fn set_broadcast(&mut self, broadcast: bool) {
        self.broadcast = broadcast;
    }

    pub fn set_announce_cache_capacity(&mut self, capacity: usize) {
        self.announce_cache_capacity = capacity;
    }

    pub fn set_announce_retry_limit(&mut self, limit: u8) {
        self.announce_retry_limit = limit;
    }

    pub fn set_announce_queue_len(&mut self, len: usize) {
        self.announce_queue_len = len;
    }

    pub fn set_announce_cap(&mut self, cap: usize) {
        self.announce_cap = cap;
    }

    pub fn set_path_request_timeout_secs(&mut self, secs: u64) {
        self.path_request_timeout_secs = secs;
    }

    pub fn set_link_proof_timeout_secs(&mut self, secs: u64) {
        self.link_proof_timeout_secs = secs;
    }

    pub fn set_link_idle_timeout_secs(&mut self, secs: u64) {
        self.link_idle_timeout_secs = secs;
    }

    pub fn set_resource_retry_interval_secs(&mut self, secs: u64) {
        self.resource_retry_interval_secs = secs;
    }

    pub fn set_resource_retry_limit(&mut self, limit: u8) {
        self.resource_retry_limit = limit;
    }

    pub fn set_ratchet_store_path(&mut self, path: PathBuf) {
        self.ratchet_store_path = Some(path);
    }

    pub fn set_blackholed_identities(&mut self, identities: impl IntoIterator<Item = AddressHash>) {
        self.blackholed_identities = identities.into_iter().collect();
    }

    pub fn set_ingress_queue_capacities(&mut self, capacities: IngressQueueCapacities) {
        self.ingress_queue_capacities = capacities;
    }
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            name: "tp".into(),
            identity: PrivateIdentity::new_from_rand(OsRng),
            broadcast: false,
            retransmit: false,
            announce_cache_capacity: 100_000,
            announce_retry_limit: 5,
            announce_queue_len: 64,
            announce_cap: 128,
            path_request_timeout_secs: 30,
            link_proof_timeout_secs: 600,
            link_idle_timeout_secs: 900,
            resource_retry_interval_secs: 2,
            resource_retry_limit: 5,
            ratchet_store_path: None,
            blackholed_identities: HashSet::new(),
            ingress_queue_capacities: IngressQueueCapacities::default(),
        }
    }
}
