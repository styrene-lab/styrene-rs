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
//! through it after RBAC capability checks.

use std::sync::{Arc, Mutex};

#[cfg(feature = "i2p-proxy")]
use crate::services::I2pProxyService;
#[cfg(feature = "terminal")]
use crate::services::TerminalService;
use crate::services::{
    AutoReplyService, ConfigService, DiscoveryService, EventService, FleetService, IdentityService,
    MessagingService, PageService, PolicyService, PropagationService, ProtocolService,
    StatusService, TunnelService,
};
use crate::storage::messages::MessagesStore;
use crate::transport::mesh_transport::MeshTransport;
use rns_core::identity::PrivateIdentity;
use styrene_services::conversations::ConversationStore;
use styrene_services::node_store::NodeStore;

/// Composition root — wires all daemon services together.
///
/// Construction creates services in startup order (preserving the semantics
/// of the Python daemon's 22-step startup sequence). Later packages will
/// add constructor parameters as services gain real dependencies.
pub struct AppContext {
    transport: Arc<dyn MeshTransport>,
    store: Arc<Mutex<MessagesStore>>,
    node_store: Arc<NodeStore>,
    identity: Arc<IdentityService>,
    config: Arc<ConfigService>,
    status: Arc<StatusService>,
    fleet: Arc<FleetService>,
    policy: Arc<PolicyService>,
    auto_reply: Arc<AutoReplyService>,
    messaging: Arc<MessagingService>,
    discovery: Arc<DiscoveryService>,
    protocol: Arc<ProtocolService>,
    events: Arc<EventService>,
    tunnel: Arc<TunnelService>,
    #[cfg(feature = "i2p-proxy")]
    i2p_proxy: Arc<I2pProxyService>,
    propagation: Arc<PropagationService>,
    pages: Arc<PageService>,
    #[cfg(feature = "terminal")]
    terminal: Arc<TerminalService>,
    conversations: Arc<ConversationStore>,
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
        let node_store = Arc::new(NodeStore::in_memory().expect("in-memory node store"));
        Self::with_node_store(transport, identity_hash, store, node_store)
    }

    /// Construct with an explicit NodeStore and RBAC policy.
    pub fn with_policy(
        transport: Arc<dyn MeshTransport>,
        identity_hash: String,
        store: Arc<Mutex<MessagesStore>>,
        node_store: Arc<NodeStore>,
        policy: PolicyService,
    ) -> Self {
        let mut ctx = Self::with_node_store(transport, identity_hash, store, node_store);
        ctx.policy = Arc::new(policy);
        ctx
    }

    /// Construct with an explicit NodeStore (e.g., file-backed for production).
    pub fn with_node_store(
        transport: Arc<dyn MeshTransport>,
        identity_hash: String,
        store: Arc<Mutex<MessagesStore>>,
        node_store: Arc<NodeStore>,
    ) -> Self {
        // Phase 1: Transport + config (foundation)
        let config = Arc::new(ConfigService::new());
        let policy = Arc::new(PolicyService::default());

        // Phase 2: Identity (depends on transport)
        let identity = Arc::new(IdentityService::with_transport(identity_hash, transport.clone()));

        // Phase 3: Discovery (depends on transport, writes to NodeStore + legacy announce table)
        let discovery = Arc::new(DiscoveryService::with_stores(store.clone(), node_store.clone()));

        // Phase 4: Propagation (store-and-forward, shares MessagesStore)
        let propagation = Arc::new(PropagationService::new(store.clone()));

        // Phase 5: Messaging (depends on transport, identity, reads/writes store's message table)
        let store_ref = store.clone();
        let messaging = Arc::new(MessagingService::with_store(store));

        // Phase 6: Protocol dispatch (depends on messaging)
        let protocol = Arc::new(ProtocolService::new());

        // Phase 7: Fleet/RPC (depends on protocol, auth)
        let fleet = Arc::new(FleetService::new());

        // Phase 8: Auto-reply (depends on config, messaging)
        let auto_reply = Arc::new(AutoReplyService::new());

        // Phase 9: Status (depends on transport, config)
        let status = Arc::new(StatusService::new());

        // Phase 10: Events (standalone pub/sub)
        let events = Arc::new(EventService::new());

        // Phase 11: Tunnel (depends on transport)
        let tunnel = Arc::new(TunnelService::new());
        tunnel.set_events(events.clone());

        // Phase 12: I2P proxy (optional, feature-gated)
        // Created as placeholder — signer wired later via set_signer().
        #[cfg(feature = "i2p-proxy")]
        let i2p_proxy = Arc::new(I2pProxyService::new());

        // Phase 13: Page server (NomadNet-compatible page hosting)
        let pages = Arc::new(PageService::with_default_dir());

        // Phase 14: Terminal sessions (local shell access for operators, desktop only)
        #[cfg(feature = "terminal")]
        let terminal = Arc::new(TerminalService::new());

        // Phase 13: Conversation metadata (pin/mute)
        let conversations =
            Arc::new(ConversationStore::in_memory().expect("in-memory conversation store"));

        Self {
            transport,
            store: store_ref,
            node_store,
            identity,
            config,
            status,
            fleet,
            policy,
            auto_reply,
            messaging,
            discovery,
            protocol,
            events,
            propagation,
            tunnel,
            #[cfg(feature = "i2p-proxy")]
            i2p_proxy,
            pages,
            #[cfg(feature = "terminal")]
            terminal,
            conversations,
        }
    }

    /// Wire a signing identity into services that need outbound delivery.
    ///
    /// Call after construction when the identity is available (after transport init).
    /// Enables MessagingService.send_chat() and FleetService RPC calls.
    pub fn set_signer(&self, signer: Arc<PrivateIdentity>) {
        self.messaging.set_signer(self.transport.clone(), signer.clone());
        self.fleet.set_signer(self.transport.clone(), signer.clone());
        #[cfg(feature = "i2p-proxy")]
        self.i2p_proxy.set_signer(
            self.transport.clone(),
            signer,
            self.identity.identity_hash().to_string(),
        );
    }

    // --- Accessors ---

    /// Shared store handle for direct access (blocklist, etc.).
    pub fn store(&self) -> &Arc<Mutex<MessagesStore>> {
        &self.store
    }

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

    pub fn policy(&self) -> &PolicyService {
        &self.policy
    }

    pub fn policy_arc(&self) -> Arc<PolicyService> {
        self.policy.clone()
    }

    pub fn auto_reply(&self) -> &AutoReplyService {
        &self.auto_reply
    }

    pub fn auto_reply_arc(&self) -> Arc<AutoReplyService> {
        self.auto_reply.clone()
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

    pub fn tunnel_arc(&self) -> Arc<TunnelService> {
        self.tunnel.clone()
    }

    #[cfg(feature = "i2p-proxy")]
    pub fn i2p_proxy(&self) -> &I2pProxyService {
        &self.i2p_proxy
    }

    #[cfg(feature = "i2p-proxy")]
    pub fn i2p_proxy_arc(&self) -> Arc<I2pProxyService> {
        self.i2p_proxy.clone()
    }

    /// LXMF propagation store-and-forward service.
    pub fn propagation(&self) -> &PropagationService {
        &self.propagation
    }

    pub fn propagation_arc(&self) -> Arc<PropagationService> {
        self.propagation.clone()
    }

    /// NomadNet-compatible page server.
    pub fn pages(&self) -> &PageService {
        &self.pages
    }

    pub fn pages_arc(&self) -> Arc<PageService> {
        self.pages.clone()
    }

    #[cfg(feature = "terminal")]
    pub fn terminal(&self) -> &TerminalService {
        &self.terminal
    }

    #[cfg(feature = "terminal")]
    pub fn terminal_arc(&self) -> Arc<TerminalService> {
        self.terminal.clone()
    }

    /// The persistent node store.
    pub fn node_store(&self) -> &Arc<NodeStore> {
        &self.node_store
    }

    /// Conversation metadata store (pin/mute state).
    pub fn conversations(&self) -> &ConversationStore {
        &self.conversations
    }
}
