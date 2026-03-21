//! AppContext — composition root for the daemon service graph.
//!
//! Owns: service construction, dependency wiring, lifecycle orchestration.
//! Does NOT own: any business logic, any IPC dispatch.
//!
//! Pattern: `Arc<AppContext>` held by services for cross-service access.
//! Services accessed via accessor methods.
//!
//! AppContext does NOT implement the `Daemon` trait — that's `DaemonFacade`
//! (Package I), which holds `Arc<AppContext>` and dispatches IPC calls
//! through it after auth checks.

use std::sync::{Arc, Mutex};

use crate::services::{
    AuthService, AutoReplyService, ConfigService, DiscoveryService, EventService, FleetService,
    IdentityService, MessagingService, ProtocolService, StatusService, TunnelService,
};
use crate::storage::messages::MessagesStore;
use crate::transport::mesh_transport::MeshTransport;

/// Composition root — wires all daemon services together.
///
/// Construction creates services in startup order (preserving the semantics
/// of the Python daemon's 22-step startup sequence). Later packages will
/// add constructor parameters as services gain real dependencies.
pub struct AppContext {
    transport: Arc<dyn MeshTransport>,
    identity: Arc<IdentityService>,
    config: Arc<ConfigService>,
    status: Arc<StatusService>,
    fleet: Arc<FleetService>,
    auth: Arc<AuthService>,
    auto_reply: Arc<AutoReplyService>,
    messaging: Arc<MessagingService>,
    discovery: Arc<DiscoveryService>,
    protocol: Arc<ProtocolService>,
    events: Arc<EventService>,
    tunnel: Arc<TunnelService>,
}

impl AppContext {
    /// Construct all services with the given transport, identity hash, and store.
    ///
    /// Services are created in startup order. Messaging and Discovery share
    /// the same MessagesStore (single SQLite connection for both messages
    /// and announces).
    pub fn new(
        transport: Arc<dyn MeshTransport>,
        identity_hash: String,
        store: Arc<Mutex<MessagesStore>>,
    ) -> Self {
        // Phase 1: Transport + config (foundation)
        let config = Arc::new(ConfigService::new());
        let auth = Arc::new(AuthService::new());

        // Phase 2: Identity (depends on transport)
        let identity = Arc::new(IdentityService::with_transport(
            identity_hash,
            transport.clone(),
        ));

        // Phase 3: Discovery (depends on transport, writes to store's announce table)
        let discovery = Arc::new(DiscoveryService::with_store(store.clone()));

        // Phase 4: Messaging (depends on transport, identity, reads/writes store's message table)
        let messaging = Arc::new(MessagingService::with_store(store));

        // Phase 5: Protocol dispatch (depends on messaging)
        let protocol = Arc::new(ProtocolService::new());

        // Phase 6: Fleet/RPC (depends on protocol, auth)
        let fleet = Arc::new(FleetService::new());

        // Phase 7: Auto-reply (depends on config, messaging)
        let auto_reply = Arc::new(AutoReplyService::new());

        // Phase 8: Status (depends on transport, config)
        let status = Arc::new(StatusService::new());

        // Phase 9: Events (standalone pub/sub)
        let events = Arc::new(EventService::new());

        // Phase 10: Tunnel (depends on transport)
        let tunnel = Arc::new(TunnelService::new());

        Self {
            transport,
            identity,
            config,
            status,
            fleet,
            auth,
            auto_reply,
            messaging,
            discovery,
            protocol,
            events,
            tunnel,
        }
    }

    // --- Accessors ---

    /// The transport abstraction (real or null).
    pub fn transport(&self) -> &dyn MeshTransport {
        self.transport.as_ref()
    }

    /// Shared transport handle for services that need `Arc<dyn MeshTransport>`.
    pub fn transport_arc(&self) -> Arc<dyn MeshTransport> {
        self.transport.clone()
    }

    pub fn identity(&self) -> &IdentityService {
        &self.identity
    }

    pub fn config(&self) -> &ConfigService {
        &self.config
    }

    pub fn status(&self) -> &StatusService {
        &self.status
    }

    pub fn auth(&self) -> &AuthService {
        &self.auth
    }

    pub fn auto_reply(&self) -> &AutoReplyService {
        &self.auto_reply
    }

    pub fn messaging(&self) -> &MessagingService {
        &self.messaging
    }

    pub fn messaging_arc(&self) -> Arc<MessagingService> {
        self.messaging.clone()
    }

    pub fn discovery(&self) -> &DiscoveryService {
        &self.discovery
    }

    pub fn discovery_arc(&self) -> Arc<DiscoveryService> {
        self.discovery.clone()
    }

    pub fn protocol(&self) -> &ProtocolService {
        &self.protocol
    }

    pub fn protocol_arc(&self) -> Arc<ProtocolService> {
        self.protocol.clone()
    }

    pub fn fleet(&self) -> &FleetService {
        &self.fleet
    }

    pub fn fleet_arc(&self) -> Arc<FleetService> {
        self.fleet.clone()
    }

    pub fn events(&self) -> &EventService {
        &self.events
    }

    pub fn events_arc(&self) -> Arc<EventService> {
        self.events.clone()
    }

    pub fn tunnel(&self) -> &TunnelService {
        &self.tunnel
    }
}
