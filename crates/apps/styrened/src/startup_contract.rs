//! Immutable diagnostics describing components initialized by a runtime.
//!
//! These values report Rust runtime composition. They do not establish
//! upstream protocol parity; parity remains governed by external gate evidence.

/// The production entrypoint that produced a startup contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeKind {
    Canonical,
    Standalone,
    Mobile,
    E2eTest,
}

/// Whether evidence from a runtime can support a production claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceScope {
    Production,
    InternalTest,
}

/// The role an initialized component has in protocol composition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentKind {
    Destination,
    Announce,
    Handler,
    Service,
    Worker,
    Scheduler,
    EventBridge,
    ReceiptBridge,
}

/// A stable diagnostic identifier for an initialized component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupComponent {
    pub kind: ComponentKind,
    pub id: &'static str,
}

impl StartupComponent {
    pub const fn new(kind: ComponentKind, id: &'static str) -> Self {
        Self { kind, id }
    }
}

/// A runtime capability and the initialized components it requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityContract {
    id: &'static str,
    required: &'static [StartupComponent],
}

/// An optional runtime capability that was requested but failed to initialize.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DegradedCapability {
    id: &'static str,
    reason: Box<str>,
}

impl DegradedCapability {
    pub fn id(&self) -> &'static str {
        self.id
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// Runtime composition and caller authorization, kept as separate dimensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveCapabilities {
    runtime: Box<[&'static str]>,
    degraded: Box<[DegradedCapability]>,
    authorized_operations: Box<[String]>,
}

impl ActiveCapabilities {
    pub fn runtime(&self) -> &[&'static str] {
        &self.runtime
    }

    pub fn degraded(&self) -> &[DegradedCapability] {
        &self.degraded
    }

    pub fn authorized_operations(&self) -> &[String] {
        &self.authorized_operations
    }
}

impl CapabilityContract {
    pub const fn id(self) -> &'static str {
        self.id
    }

    pub const fn required(self) -> &'static [StartupComponent] {
        self.required
    }
}

/// Diagnostic contract captured after a runtime completes startup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupContract {
    runtime: RuntimeKind,
    evidence_scope: EvidenceScope,
    components: Box<[StartupComponent]>,
    advertised_capabilities: Box<[CapabilityContract]>,
    degraded_capabilities: Box<[DegradedCapability]>,
}

impl StartupContract {
    fn validated(
        runtime: RuntimeKind,
        evidence_scope: EvidenceScope,
        components: impl Into<Box<[StartupComponent]>>,
        advertised_capabilities: impl Into<Box<[CapabilityContract]>>,
        degraded_capabilities: impl Into<Box<[DegradedCapability]>>,
    ) -> Result<Self, &'static str> {
        let components = components.into();
        let advertised_capabilities = advertised_capabilities.into();
        let degraded_capabilities = degraded_capabilities.into();
        if advertised_capabilities.iter().any(|capability| {
            capability.required.iter().any(|required| !components.contains(required))
        }) {
            return Err("advertised startup capability is missing a required component");
        }
        if degraded_capabilities.iter().any(|degraded| {
            degraded.reason.trim().is_empty()
                || advertised_capabilities.iter().any(|active| active.id == degraded.id)
        }) {
            return Err("degraded startup capability is active or missing a reason");
        }
        if degraded_capabilities.iter().enumerate().any(|(index, capability)| {
            degraded_capabilities[index + 1..].iter().any(|other| other.id == capability.id)
        }) {
            return Err("degraded startup capability is duplicated");
        }

        Ok(Self {
            runtime,
            evidence_scope,
            components,
            advertised_capabilities,
            degraded_capabilities,
        })
    }

    pub fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    pub fn evidence_scope(&self) -> EvidenceScope {
        self.evidence_scope
    }

    pub fn components(&self) -> &[StartupComponent] {
        &self.components
    }

    pub fn advertised_capabilities(&self) -> &[CapabilityContract] {
        &self.advertised_capabilities
    }

    pub fn active_capabilities(
        &self,
        authorized_operations: impl IntoIterator<Item = impl Into<String>>,
    ) -> ActiveCapabilities {
        let runtime: Box<[_]> =
            self.advertised_capabilities.iter().map(|capability| capability.id).collect();
        let mut degraded = self.degraded_capabilities.to_vec();
        let mut active_operations = Vec::new();
        for operation in authorized_operations.into_iter().map(Into::into) {
            match operation_requirement(&operation) {
                OperationRequirement::Core => active_operations.push(operation),
                OperationRequirement::Runtime(required) if runtime.contains(&required) => {
                    active_operations.push(operation);
                }
                OperationRequirement::Runtime(required) => degraded.push(DegradedCapability {
                    id: known_operation_id(&operation),
                    reason: format!("required runtime capability {required} is not active").into(),
                }),
                OperationRequirement::Unavailable => degraded.push(DegradedCapability {
                    id: known_operation_id(&operation),
                    reason: "operation is not exposed by the active daemon composition".into(),
                }),
            }
        }
        ActiveCapabilities {
            runtime,
            degraded: degraded.into_boxed_slice(),
            authorized_operations: active_operations.into_boxed_slice(),
        }
    }

    pub fn has_component(&self, component: StartupComponent) -> bool {
        self.components.contains(&component)
    }

    pub fn advertises(&self, capability_id: &str) -> bool {
        self.advertised_capabilities.iter().any(|capability| capability.id == capability_id)
    }

    /// Whether this composition evidence may contribute to a production claim.
    /// External interoperability gates remain required for protocol parity.
    pub fn can_support_production_claim(&self, capability_id: &str) -> bool {
        self.evidence_scope == EvidenceScope::Production && self.advertises(capability_id)
    }

    pub fn missing_requirements(&self, capability: CapabilityContract) -> Vec<StartupComponent> {
        capability
            .required
            .iter()
            .copied()
            .filter(|required| !self.components.contains(required))
            .collect()
    }
}

enum OperationRequirement {
    Core,
    Runtime(&'static str),
    Unavailable,
}

fn operation_requirement(operation: &str) -> OperationRequirement {
    use styrene_rbac::Capability as C;

    match operation {
        C::RPC_PING | C::RPC_STATUS | C::MESSAGING_HISTORY_READ | C::MESSAGING_MANAGE => {
            OperationRequirement::Core
        }
        C::RPC_CONFIG_UPDATE => OperationRequirement::Runtime(capabilities::LOCAL_CONFIG.id()),
        C::POLICY_UPDATE => OperationRequirement::Runtime(capabilities::LOCAL_POLICY.id()),
        C::CHAT_SEND | C::CHAT_RECEIVE | C::PAGE_BROWSE => {
            OperationRequirement::Runtime(capabilities::LXMF_DIRECT.id())
        }
        C::MESSAGING_LIFECYCLE => OperationRequirement::Runtime(capabilities::LXMF_DIRECT.id()),
        C::NETWORK_ANNOUNCE
        | C::NETWORK_PATH_REQUEST
        | C::NETWORK_PROBE
        | C::NETWORK_LINK_OPEN
        | C::NETWORK_LINK_CLOSE => {
            OperationRequirement::Runtime(capabilities::NETWORK_OPERATIONS.id())
        }
        C::NETWORK_REQUEST => OperationRequirement::Runtime(capabilities::RNS_REQUESTS.id()),
        C::NETWORK_REQUEST_CANCEL => {
            OperationRequirement::Runtime(capabilities::RNS_REQUEST_CANCELLATION.id())
        }
        C::NETWORK_RESOURCE_CANCEL => {
            OperationRequirement::Runtime(capabilities::RNS_RESOURCE_CANCELLATION.id())
        }
        C::RPC_INBOX_READ
        | C::RPC_EXEC
        | C::RPC_REBOOT
        | C::RPC_SELF_UPDATE
        | C::RPC_FLEET_APPLY => OperationRequirement::Runtime(capabilities::STYRENE_RPC.id()),
        _ => OperationRequirement::Unavailable,
    }
}

fn known_operation_id(operation: &str) -> &'static str {
    styrene_rbac::ALL_CAPABILITIES
        .iter()
        .copied()
        .find(|known| *known == operation)
        .unwrap_or("unknown.operation")
}

/// Mutable recorder used only while a composition root is starting.
#[derive(Clone)]
pub struct StartupContractBuilder {
    runtime: RuntimeKind,
    evidence_scope: EvidenceScope,
    components: Vec<StartupComponent>,
    capabilities: Vec<CapabilityContract>,
    degraded_capabilities: Vec<DegradedCapability>,
}

impl StartupContractBuilder {
    pub fn production(runtime: RuntimeKind) -> Self {
        assert_ne!(
            runtime,
            RuntimeKind::E2eTest,
            "E2E test runtimes cannot produce production startup evidence"
        );
        Self {
            runtime,
            evidence_scope: EvidenceScope::Production,
            components: Vec::new(),
            capabilities: Vec::new(),
            degraded_capabilities: Vec::new(),
        }
    }

    pub fn internal_test(runtime: RuntimeKind) -> Self {
        Self {
            runtime,
            evidence_scope: EvidenceScope::InternalTest,
            components: Vec::new(),
            capabilities: Vec::new(),
            degraded_capabilities: Vec::new(),
        }
    }

    pub fn record(&mut self, component: StartupComponent) {
        if !self.components.contains(&component) {
            self.components.push(component);
        }
    }

    pub fn record_local_execution_services(&mut self) {
        for component in [
            components::CONFIG_SERVICE,
            components::IDENTITY_SERVICE,
            components::AUTO_REPLY_SERVICE,
            components::POLICY_SERVICE,
            components::FLEET_SERVICE,
        ] {
            self.record(component);
        }
    }

    pub fn record_transport_state_services(&mut self) {
        for component in [
            components::REQUEST_STATE_SERVICE,
            components::REQUEST_CANCELLATION_SERVICE,
            components::RESOURCE_STATE_SERVICE,
            components::RESOURCE_CANCELLATION_SERVICE,
            components::TRANSPORT_REQUEST_OBSERVATION_BRIDGE,
            components::TRANSPORT_RESOURCE_OBSERVATION_BRIDGE,
        ] {
            self.record(component);
        }
    }

    pub fn advertise(&mut self, capability: CapabilityContract) -> Result<(), &'static str> {
        if capability.required.iter().any(|required| !self.components.contains(required)) {
            return Err("advertised startup capability is missing a required component");
        }
        if self.degraded_capabilities.iter().any(|degraded| degraded.id == capability.id) {
            return Err("startup capability is already degraded");
        }
        if !self.capabilities.contains(&capability) {
            self.capabilities.push(capability);
        }
        Ok(())
    }

    pub fn degrade(
        &mut self,
        capability: CapabilityContract,
        reason: impl Into<Box<str>>,
    ) -> Result<(), &'static str> {
        let reason = reason.into();
        if reason.trim().is_empty()
            || self.capabilities.iter().any(|active| active.id == capability.id)
        {
            return Err("degraded startup capability is active or missing a reason");
        }
        if self.degraded_capabilities.iter().any(|degraded| degraded.id == capability.id) {
            return Err("degraded startup capability is duplicated");
        }
        self.degraded_capabilities.push(DegradedCapability { id: capability.id, reason });
        Ok(())
    }

    pub fn finish(self) -> StartupContract {
        match StartupContract::validated(
            self.runtime,
            self.evidence_scope,
            self.components,
            self.capabilities,
            self.degraded_capabilities,
        ) {
            Ok(contract) => contract,
            Err(error) => panic!("invalid recorded startup contract: {error}"),
        }
    }
}

pub mod components {
    use super::{ComponentKind, StartupComponent};

    pub const LXMF_DELIVERY: StartupComponent =
        StartupComponent::new(ComponentKind::Destination, "lxmf.delivery");
    pub const NOMADNET_NODE_DESTINATION: StartupComponent =
        StartupComponent::new(ComponentKind::Destination, "nomadnetwork.node");
    pub const NOMADNET_NODE_ANNOUNCE: StartupComponent =
        StartupComponent::new(ComponentKind::Announce, "nomadnetwork.node");
    pub const PARTIAL_PROPAGATION_STATS_DESTINATION: StartupComponent =
        StartupComponent::new(ComponentKind::Destination, "lxmf.propagation.control.partial-stats");
    pub const STANDARD_LXMF_PROPAGATION_DESTINATION: StartupComponent =
        StartupComponent::new(ComponentKind::Destination, "lxmf.propagation");
    pub const STANDARD_LXMF_PROPAGATION_ANNOUNCE: StartupComponent =
        StartupComponent::new(ComponentKind::Announce, "lxmf.propagation.active");
    pub const RPC_RESPONSE_HANDLER: StartupComponent =
        StartupComponent::new(ComponentKind::Handler, "styrene-rpc-response");
    pub const RPC_REQUEST_HANDLER: StartupComponent =
        StartupComponent::new(ComponentKind::Handler, "styrene-rpc-request");
    pub const NATIVE_NOMADNET_REQUEST_HANDLER: StartupComponent =
        StartupComponent::new(ComponentKind::Handler, "native-nomadnet-request");
    pub const STANDARD_LXMF_PROPAGATION_OFFER_HANDLER: StartupComponent =
        StartupComponent::new(ComponentKind::Handler, "standard-lxmf-propagation-offer");
    pub const STANDARD_LXMF_PROPAGATION_GET_HANDLER: StartupComponent =
        StartupComponent::new(ComponentKind::Handler, "standard-lxmf-propagation-get");
    pub const STANDARD_LXMF_PROPAGATION_INGRESS_WORKER: StartupComponent =
        StartupComponent::new(ComponentKind::Worker, "standard-lxmf-propagation-ingress");
    pub const STANDARD_LXMF_PROPAGATION_CLIENT_COORDINATOR: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "standard-lxmf-propagation-client");
    pub const STANDARD_LXMF_PROPAGATION_SYNC_SCHEDULER: StartupComponent =
        StartupComponent::new(ComponentKind::Scheduler, "standard-lxmf-propagation-sync");
    pub const STYRENE_PAGE_REQUEST_HANDLER: StartupComponent =
        StartupComponent::new(ComponentKind::Handler, "styrene-page-request");
    pub const STYRENE_PROPAGATION_REQUEST_HANDLER: StartupComponent =
        StartupComponent::new(ComponentKind::Handler, "styrene-propagation-request");
    pub const STYRENE_PROPAGATION_SERVICE: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "styrene-propagation-enabled");
    pub const TUNNEL_HANDLER: StartupComponent =
        StartupComponent::new(ComponentKind::Handler, "tunnel-handler");
    pub const I2P_PROXY_HANDLER: StartupComponent =
        StartupComponent::new(ComponentKind::Handler, "i2p-proxy-handler");
    pub const INBOUND_PACKET_WORKER: StartupComponent =
        StartupComponent::new(ComponentKind::Worker, "inbound-packet");
    pub const INBOUND_RESOURCE_WORKER: StartupComponent =
        StartupComponent::new(ComponentKind::Worker, "inbound-resource");
    pub const ANNOUNCE_WORKER: StartupComponent =
        StartupComponent::new(ComponentKind::Worker, "announce");
    pub const LINK_WORKER: StartupComponent = StartupComponent::new(ComponentKind::Worker, "link");
    pub const ROUTE_WORKER: StartupComponent =
        StartupComponent::new(ComponentKind::Worker, "route");
    pub const NETWORK_OPERATION_COORDINATOR: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "network-operation-coordinator");
    pub const CONFIG_SERVICE: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "local-config");
    pub const IDENTITY_SERVICE: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "local-identity");
    pub const AUTO_REPLY_SERVICE: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "local-auto-reply");
    pub const POLICY_SERVICE: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "local-policy");
    pub const FLEET_SERVICE: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "styrene-fleet");
    pub const REQUEST_STATE_SERVICE: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "rns-request-state");
    pub const REQUEST_CANCELLATION_SERVICE: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "rns-request-cancellation");
    pub const RESOURCE_STATE_SERVICE: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "rns-resource-state");
    pub const RESOURCE_CANCELLATION_SERVICE: StartupComponent =
        StartupComponent::new(ComponentKind::Service, "rns-resource-cancellation");
    pub const TRANSPORT_REQUEST_OBSERVATION_BRIDGE: StartupComponent =
        StartupComponent::new(ComponentKind::EventBridge, "transport-requests");
    pub const TRANSPORT_RESOURCE_OBSERVATION_BRIDGE: StartupComponent =
        StartupComponent::new(ComponentKind::EventBridge, "transport-resources");
    pub const LEGACY_INBOUND_WORKER: StartupComponent =
        StartupComponent::new(ComponentKind::Worker, "legacy-inbound");
    pub const LEGACY_MESSAGE_EVENT_ADAPTER: StartupComponent =
        StartupComponent::new(ComponentKind::EventBridge, "legacy-message-observations");
    pub const LEGACY_ANNOUNCE_WORKER: StartupComponent =
        StartupComponent::new(ComponentKind::Worker, "legacy-announce");
    pub const LEGACY_RECEIPT_WORKER: StartupComponent =
        StartupComponent::new(ComponentKind::Worker, "legacy-receipt");
    pub const PARTIAL_PROPAGATION_STATS_WORKER: StartupComponent =
        StartupComponent::new(ComponentKind::Worker, "partial-propagation-stats");
    pub const PROPAGATION_EXPIRY_SCHEDULER: StartupComponent =
        StartupComponent::new(ComponentKind::Scheduler, "styrene-propagation-expiry");
    pub const NATIVE_RESOURCE_RETRY_SCHEDULER: StartupComponent =
        StartupComponent::new(ComponentKind::Scheduler, "rns-resource-retry");
    pub const LXMF_ROUTER_DEADLINE_SCHEDULER: StartupComponent =
        StartupComponent::new(ComponentKind::Scheduler, "lxmf-router-deadline");
    pub const ANNOUNCE_SCHEDULER: StartupComponent =
        StartupComponent::new(ComponentKind::Scheduler, "legacy-announce");
    pub const TRANSPORT_ANNOUNCE_BRIDGE: StartupComponent =
        StartupComponent::new(ComponentKind::EventBridge, "transport-announces");
    pub const TRANSPORT_LINK_BRIDGE: StartupComponent =
        StartupComponent::new(ComponentKind::EventBridge, "transport-links");
    pub const IPC_EVENT_BRIDGE: StartupComponent =
        StartupComponent::new(ComponentKind::EventBridge, "daemon-events-to-ipc");
    pub const LEGACY_RECEIPT_BRIDGE: StartupComponent =
        StartupComponent::new(ComponentKind::ReceiptBridge, "legacy-rpc-receipts");
    /// Authenticated RNS delivery receipts correlated to exact LXMF packets.
    pub const SERVICE_RECEIPT_BRIDGE: StartupComponent =
        StartupComponent::new(ComponentKind::ReceiptBridge, "service-rns-delivery-receipts");
    pub const OUTBOUND_RESOURCE_COMPLETION_WORKER: StartupComponent =
        StartupComponent::new(ComponentKind::Worker, "outbound-resource-completion");
}

pub mod capabilities {
    use super::{CapabilityContract, StartupComponent, components as c};

    const DIRECT_COMPONENTS: &[StartupComponent] = &[
        c::LXMF_DELIVERY,
        c::INBOUND_PACKET_WORKER,
        c::INBOUND_RESOURCE_WORKER,
        c::ANNOUNCE_WORKER,
        c::LINK_WORKER,
        c::TRANSPORT_ANNOUNCE_BRIDGE,
        c::TRANSPORT_LINK_BRIDGE,
        c::SERVICE_RECEIPT_BRIDGE,
        c::OUTBOUND_RESOURCE_COMPLETION_WORKER,
        c::NATIVE_RESOURCE_RETRY_SCHEDULER,
        c::LXMF_ROUTER_DEADLINE_SCHEDULER,
    ];
    const STYRENE_RPC_COMPONENTS: &[StartupComponent] = &[
        c::LXMF_DELIVERY,
        c::RPC_RESPONSE_HANDLER,
        c::RPC_REQUEST_HANDLER,
        c::INBOUND_PACKET_WORKER,
        c::INBOUND_RESOURCE_WORKER,
        c::FLEET_SERVICE,
    ];
    const PAPER_EXPORT_COMPONENTS: &[StartupComponent] = &[c::LXMF_DELIVERY];
    const NETWORK_OPERATION_COMPONENTS: &[StartupComponent] = &[
        c::NETWORK_OPERATION_COORDINATOR,
        c::ROUTE_WORKER,
        c::LINK_WORKER,
        c::TRANSPORT_ANNOUNCE_BRIDGE,
        c::TRANSPORT_LINK_BRIDGE,
    ];
    const LOCAL_CONFIG_COMPONENTS: &[StartupComponent] =
        &[c::CONFIG_SERVICE, c::IDENTITY_SERVICE, c::AUTO_REPLY_SERVICE];
    const LOCAL_POLICY_COMPONENTS: &[StartupComponent] = &[c::POLICY_SERVICE];
    const RNS_REQUEST_COMPONENTS: &[StartupComponent] =
        &[c::REQUEST_STATE_SERVICE, c::TRANSPORT_REQUEST_OBSERVATION_BRIDGE];
    const RNS_REQUEST_CANCELLATION_COMPONENTS: &[StartupComponent] = &[
        c::REQUEST_STATE_SERVICE,
        c::REQUEST_CANCELLATION_SERVICE,
        c::TRANSPORT_REQUEST_OBSERVATION_BRIDGE,
    ];
    const RNS_RESOURCE_CANCELLATION_COMPONENTS: &[StartupComponent] = &[
        c::RESOURCE_STATE_SERVICE,
        c::RESOURCE_CANCELLATION_SERVICE,
        c::TRANSPORT_RESOURCE_OBSERVATION_BRIDGE,
        c::INBOUND_RESOURCE_WORKER,
        c::OUTBOUND_RESOURCE_COMPLETION_WORKER,
    ];
    const LEGACY_RECEIPT_COMPONENTS: &[StartupComponent] =
        &[c::LEGACY_RECEIPT_BRIDGE, c::LEGACY_RECEIPT_WORKER];
    const NATIVE_NOMADNET_COMPONENTS: &[StartupComponent] = &[
        c::NOMADNET_NODE_DESTINATION,
        c::NATIVE_NOMADNET_REQUEST_HANDLER,
        c::NOMADNET_NODE_ANNOUNCE,
    ];
    const STANDARD_PROPAGATION_COMPONENTS: &[StartupComponent] = &[
        c::STANDARD_LXMF_PROPAGATION_DESTINATION,
        c::STANDARD_LXMF_PROPAGATION_OFFER_HANDLER,
        c::STANDARD_LXMF_PROPAGATION_GET_HANDLER,
        c::STANDARD_LXMF_PROPAGATION_INGRESS_WORKER,
        c::STANDARD_LXMF_PROPAGATION_ANNOUNCE,
    ];
    const STANDARD_PROPAGATION_CLIENT_COMPONENTS: &[StartupComponent] = &[
        c::LXMF_DELIVERY,
        c::STANDARD_LXMF_PROPAGATION_CLIENT_COORDINATOR,
        c::STANDARD_LXMF_PROPAGATION_SYNC_SCHEDULER,
        c::REQUEST_STATE_SERVICE,
    ];
    const STYRENE_PAGE_COMPONENTS: &[StartupComponent] = &[
        c::LXMF_DELIVERY,
        c::INBOUND_PACKET_WORKER,
        c::INBOUND_RESOURCE_WORKER,
        c::STYRENE_PAGE_REQUEST_HANDLER,
    ];
    const STYRENE_PROPAGATION_COMPONENTS: &[StartupComponent] = &[
        c::LXMF_DELIVERY,
        c::INBOUND_PACKET_WORKER,
        c::INBOUND_RESOURCE_WORKER,
        c::STYRENE_PROPAGATION_REQUEST_HANDLER,
        c::STYRENE_PROPAGATION_SERVICE,
    ];

    /// Initialized direct-LXMF runtime path; not a verified interoperability claim.
    pub const LXMF_DIRECT: CapabilityContract =
        CapabilityContract { id: "runtime.lxmf.direct", required: DIRECT_COMPONENTS };
    /// Local paper URI export path; this is runtime availability, not parity evidence.
    pub const LXMF_PAPER_EXPORT: CapabilityContract =
        CapabilityContract { id: "runtime.lxmf.paper-export", required: PAPER_EXPORT_COMPONENTS };
    /// Styrene CBOR RPC over LXMF; not a native RNS request capability.
    pub const STYRENE_RPC: CapabilityContract =
        CapabilityContract { id: "runtime.styrene.rpc", required: STYRENE_RPC_COMPONENTS };
    pub const NETWORK_OPERATIONS: CapabilityContract = CapabilityContract {
        id: "runtime.network.operations",
        required: NETWORK_OPERATION_COMPONENTS,
    };
    pub const LOCAL_CONFIG: CapabilityContract =
        CapabilityContract { id: "runtime.local.config", required: LOCAL_CONFIG_COMPONENTS };
    pub const LOCAL_POLICY: CapabilityContract =
        CapabilityContract { id: "runtime.local.policy", required: LOCAL_POLICY_COMPONENTS };
    pub const RNS_REQUESTS: CapabilityContract =
        CapabilityContract { id: "runtime.rns.requests", required: RNS_REQUEST_COMPONENTS };
    pub const RNS_REQUEST_CANCELLATION: CapabilityContract = CapabilityContract {
        id: "runtime.rns.request-cancellation",
        required: RNS_REQUEST_CANCELLATION_COMPONENTS,
    };
    pub const RNS_RESOURCE_CANCELLATION: CapabilityContract = CapabilityContract {
        id: "runtime.rns.resource-cancellation",
        required: RNS_RESOURCE_CANCELLATION_COMPONENTS,
    };
    /// Receipt correlation owned by the frozen legacy RPC daemon only.
    pub const LEGACY_RPC_RECEIPTS: CapabilityContract = CapabilityContract {
        id: "runtime.legacy-rpc.receipts",
        required: LEGACY_RECEIPT_COMPONENTS,
    };
    pub const NATIVE_NOMADNET_HOST: CapabilityContract = CapabilityContract {
        id: "runtime.native-nomadnet.host",
        required: NATIVE_NOMADNET_COMPONENTS,
    };
    pub const STANDARD_LXMF_PROPAGATION: CapabilityContract = CapabilityContract {
        id: "runtime.standard-lxmf.propagation",
        required: STANDARD_PROPAGATION_COMPONENTS,
    };
    pub const STANDARD_LXMF_PROPAGATION_CLIENT: CapabilityContract = CapabilityContract {
        id: "runtime.standard-lxmf.propagation-client",
        required: STANDARD_PROPAGATION_CLIENT_COMPONENTS,
    };
    pub const STYRENE_PAGE_HOST: CapabilityContract =
        CapabilityContract { id: "runtime.styrene.pages.host", required: STYRENE_PAGE_COMPONENTS };
    pub const STYRENE_PROPAGATION_HOST: CapabilityContract = CapabilityContract {
        id: "runtime.styrene.propagation.host",
        required: STYRENE_PROPAGATION_COMPONENTS,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_advertisement_is_rejected() {
        let mut builder = StartupContractBuilder::production(RuntimeKind::Mobile);
        builder.record(components::LXMF_DELIVERY);
        let result = builder.advertise(capabilities::LXMF_DIRECT);

        assert_eq!(
            result.err(),
            Some("advertised startup capability is missing a required component")
        );
    }

    #[test]
    #[should_panic(expected = "E2E test runtimes cannot produce production startup evidence")]
    fn e2e_runtime_cannot_create_production_evidence() {
        let _ = StartupContractBuilder::production(RuntimeKind::E2eTest);
    }

    #[test]
    fn active_and_degraded_runtime_capabilities_stay_separate_from_authorization() {
        let mut builder = StartupContractBuilder::production(RuntimeKind::Mobile);
        for component in capabilities::LXMF_DIRECT.required() {
            builder.record(*component);
        }
        builder.advertise(capabilities::LXMF_DIRECT).unwrap();
        builder.degrade(capabilities::NATIVE_NOMADNET_HOST, "request handler unavailable").unwrap();
        let contract = builder.finish();

        let effective = contract.active_capabilities(["rpc.status", "chat.receive"]);

        assert_eq!(effective.runtime(), [capabilities::LXMF_DIRECT.id()]);
        assert_eq!(effective.degraded()[0].id(), capabilities::NATIVE_NOMADNET_HOST.id());
        assert_eq!(effective.degraded()[0].reason(), "request handler unavailable");
        assert_eq!(effective.authorized_operations(), ["rpc.status", "chat.receive"]);
    }

    #[test]
    fn active_and_degraded_capability_states_cannot_conflict() {
        let mut builder = StartupContractBuilder::production(RuntimeKind::Mobile);
        builder.degrade(capabilities::LXMF_DIRECT, "transport unavailable").unwrap();

        assert_eq!(
            builder.degrade(capabilities::LXMF_DIRECT, "still unavailable").unwrap_err(),
            "degraded startup capability is duplicated"
        );
        assert_eq!(
            builder.degrade(capabilities::STYRENE_RPC, "  ").unwrap_err(),
            "degraded startup capability is active or missing a reason"
        );
        for component in capabilities::LXMF_DIRECT.required() {
            builder.record(*component);
        }
        assert_eq!(
            builder.advertise(capabilities::LXMF_DIRECT).unwrap_err(),
            "startup capability is already degraded"
        );
    }

    #[test]
    fn authorized_operations_are_intersected_with_active_composition() {
        let contract = StartupContractBuilder::production(RuntimeKind::Canonical).finish();
        let effective = contract.active_capabilities([
            styrene_rbac::Capability::RPC_STATUS,
            styrene_rbac::Capability::CHAT_SEND,
            styrene_rbac::Capability::RPC_EXEC,
        ]);

        assert_eq!(effective.authorized_operations(), [styrene_rbac::Capability::RPC_STATUS]);
        assert!(effective.degraded().iter().any(|item| {
            item.id() == styrene_rbac::Capability::CHAT_SEND
                && item.reason().contains(capabilities::LXMF_DIRECT.id())
        }));
        assert!(effective.degraded().iter().any(|item| {
            item.id() == styrene_rbac::Capability::RPC_EXEC
                && item.reason().contains(capabilities::STYRENE_RPC.id())
        }));
    }

    #[test]
    fn network_operations_require_their_coordinator_and_observation_workers() {
        let mut builder = StartupContractBuilder::production(RuntimeKind::Canonical);
        for component in capabilities::LXMF_DIRECT.required() {
            builder.record(*component);
        }
        builder.advertise(capabilities::LXMF_DIRECT).unwrap();
        let direct_only = builder.clone().finish().active_capabilities([
            styrene_rbac::Capability::CHAT_SEND,
            styrene_rbac::Capability::NETWORK_PATH_REQUEST,
        ]);
        assert_eq!(direct_only.authorized_operations(), [styrene_rbac::Capability::CHAT_SEND]);
        assert!(direct_only.degraded().iter().any(|item| {
            item.id() == styrene_rbac::Capability::NETWORK_PATH_REQUEST
                && item.reason().contains(capabilities::NETWORK_OPERATIONS.id())
        }));

        for component in capabilities::NETWORK_OPERATIONS.required() {
            builder.record(*component);
        }
        builder.advertise(capabilities::NETWORK_OPERATIONS).unwrap();
        let active =
            builder.finish().active_capabilities([styrene_rbac::Capability::NETWORK_PATH_REQUEST]);
        assert_eq!(
            active.authorized_operations(),
            [styrene_rbac::Capability::NETWORK_PATH_REQUEST]
        );
    }

    #[test]
    fn local_and_remote_mutations_negotiate_against_distinct_composition() {
        let operations = [
            styrene_rbac::Capability::RPC_CONFIG_UPDATE,
            styrene_rbac::Capability::POLICY_UPDATE,
            styrene_rbac::Capability::RPC_FLEET_APPLY,
        ];
        let absent = StartupContractBuilder::production(RuntimeKind::Canonical)
            .finish()
            .active_capabilities(operations);
        assert!(absent.authorized_operations().is_empty());
        for (operation, runtime) in [
            (styrene_rbac::Capability::RPC_CONFIG_UPDATE, capabilities::LOCAL_CONFIG.id()),
            (styrene_rbac::Capability::POLICY_UPDATE, capabilities::LOCAL_POLICY.id()),
            (styrene_rbac::Capability::RPC_FLEET_APPLY, capabilities::STYRENE_RPC.id()),
        ] {
            assert!(
                absent
                    .degraded()
                    .iter()
                    .any(|item| { item.id() == operation && item.reason().contains(runtime) })
            );
        }

        let mut builder = StartupContractBuilder::production(RuntimeKind::Canonical);
        for capability in
            [capabilities::LOCAL_CONFIG, capabilities::LOCAL_POLICY, capabilities::STYRENE_RPC]
        {
            for component in capability.required() {
                builder.record(*component);
            }
            builder.advertise(capability).unwrap();
        }
        let active = builder.finish().active_capabilities(operations);
        assert_eq!(active.authorized_operations(), operations);
    }

    #[test]
    fn request_and_cancellation_operations_require_exact_transport_state_paths() {
        let operations = [
            styrene_rbac::Capability::NETWORK_REQUEST,
            styrene_rbac::Capability::NETWORK_REQUEST_CANCEL,
            styrene_rbac::Capability::NETWORK_RESOURCE_CANCEL,
        ];
        let mut route_only = StartupContractBuilder::production(RuntimeKind::Canonical);
        for component in capabilities::NETWORK_OPERATIONS.required() {
            route_only.record(*component);
        }
        route_only.advertise(capabilities::NETWORK_OPERATIONS).unwrap();
        let degraded = route_only.finish().active_capabilities(operations);
        assert!(degraded.authorized_operations().is_empty());
        for (operation, runtime) in [
            (styrene_rbac::Capability::NETWORK_REQUEST, capabilities::RNS_REQUESTS.id()),
            (
                styrene_rbac::Capability::NETWORK_REQUEST_CANCEL,
                capabilities::RNS_REQUEST_CANCELLATION.id(),
            ),
            (
                styrene_rbac::Capability::NETWORK_RESOURCE_CANCEL,
                capabilities::RNS_RESOURCE_CANCELLATION.id(),
            ),
        ] {
            assert!(
                degraded
                    .degraded()
                    .iter()
                    .any(|item| { item.id() == operation && item.reason().contains(runtime) })
            );
        }

        let mut builder = StartupContractBuilder::production(RuntimeKind::Canonical);
        for capability in [
            capabilities::RNS_REQUESTS,
            capabilities::RNS_REQUEST_CANCELLATION,
            capabilities::RNS_RESOURCE_CANCELLATION,
        ] {
            for component in capability.required() {
                builder.record(*component);
            }
            builder.advertise(capability).unwrap();
        }
        let active = builder.finish().active_capabilities(operations);
        assert_eq!(active.authorized_operations(), operations);
    }
}
