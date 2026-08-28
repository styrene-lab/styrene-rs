use crate::backend::{BackendCapabilities, ConnectionGeneration};
use crate::daemon_bridge::{DaemonEvent, InterfaceStats, PathTableEntry};
use crate::scenario::ScenarioRun;
use crate::state;
use std::collections::{HashMap, HashSet, VecDeque};

use serde::Serialize;

const TIMELINE_CAPACITY: usize = 200;
const MESSAGE_TRANSIENT_CAPACITY: usize = 1_024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DataState {
    #[default]
    Loading,
    Empty,
    Ready,
    Degraded {
        reason: String,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeStore {
    pub generation: ConnectionGeneration,
    pub server_generation: Option<u64>,
    pub event_server_generation: Option<u64>,
    pub capabilities: Option<styrene_ipc::types::ActiveCapabilitiesInfo>,
    pub profile: String,
    pub connected: bool,
    pub connection_mode: String,
    pub state: DataState,
}

#[derive(Clone, Debug, Default)]
pub struct IdentityStore {
    pub current: Option<styrene_ipc::types::IdentityInfo>,
    pub state: DataState,
}

#[derive(Clone, Debug, Default)]
pub struct NetworkStore {
    pub status: state::MeshStatusInfo,
    pub peers: Vec<state::PeerEntry>,
    pub paths: Vec<state::PathEntry>,
    pub links: Vec<state::LinkInfo>,
    pub interfaces: Vec<state::InterfaceInfo>,
    pub announces: Vec<state::AnnounceEvent>,
    pub operations: Vec<styrene_ipc::types::NetworkOperationInfo>,
    pub requests: Vec<styrene_ipc::types::RequestObservationInfo>,
    pub resources: Vec<styrene_ipc::types::ResourceTransferInfo>,
    pub state: DataState,
}

#[derive(Clone, Debug, Default)]
pub struct MessageStore {
    pub conversations: Vec<state::ConversationEntry>,
    pub messages: Vec<state::ChatMessage>,
    pub state: DataState,
    pub conversation_cursor: Option<String>,
    pub message_cursors: HashMap<String, String>,
    pub drafts: HashMap<String, styrene_ipc::types::ConversationDraft>,
    pub accepted_compose: Option<(String, String)>,
    pub paper_export: Option<PaperExportState>,
    live_message_ids: HashSet<String>,
    live_message_order: VecDeque<String>,
    loaded_peer_hashes: HashSet<String>,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct PaperExportState {
    pub message_id: String,
    pub uri: String,
}

impl std::fmt::Debug for PaperExportState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PaperExportState")
            .field("message_id", &self.message_id)
            .field("uri", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Default)]
pub struct FleetStore {
    pub managed_peers: Vec<String>,
    pub statuses: Vec<styrene_ipc::types::RemoteStatusInfo>,
    pub jobs: Vec<FleetJob>,
    pub state: DataState,
}

#[derive(Clone, Debug, Default)]
pub struct PropagationStore {
    pub enabled: bool,
    pub snapshot: Option<styrene_ipc::types::PropagationSnapshot>,
    pub state: DataState,
    pub standard_snapshot: Option<styrene_ipc::types::StandardPropagationSnapshot>,
    pub standard_state: DataState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FleetOperation {
    Status,
    Execute { command: String, args: Vec<String> },
    Reboot { delay_secs: Option<u64> },
    Block,
    ApplyProfile,
}

impl FleetOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Execute { .. } => "Execute",
            Self::Reboot { .. } => "Reboot",
            Self::Block => "Block",
            Self::ApplyProfile => "Apply profile",
        }
    }

    pub fn required_capability(&self) -> &'static str {
        match self {
            Self::Status => "rpc.status",
            Self::Execute { .. } => "rpc.exec",
            Self::Reboot { .. } => "rpc.reboot",
            Self::Block => "policy.update",
            Self::ApplyProfile => "rpc.fleet_apply",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FleetJobState {
    Running,
    Succeeded,
    Denied,
    Unsupported,
    TimedOut,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FleetJob {
    pub id: String,
    pub target: String,
    pub operation: FleetOperation,
    pub state: FleetJobState,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ContentStore {
    pub page: Option<state::PageView>,
    pub download: Option<styrene_ipc::types::FileDownloadInfo>,
    pub local_inventory: Vec<styrene_ipc::types::PageInfo>,
    pub state: DataState,
}

#[derive(Clone, Debug, Default)]
pub struct ScenarioStore {
    pub run: Option<ScenarioRun>,
    pub state: DataState,
}

#[derive(Clone, Debug, Default)]
pub struct ActivityStore {
    pub entries: Vec<state::ActivityEntry>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandSummary {
    pub transport_active: bool,
    pub interface_count: u32,
    pub observed_peers: usize,
    pub route_count: usize,
    pub active_links: u32,
    pub link_records: usize,
    pub propagation_enabled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DomainStores {
    pub runtime: RuntimeStore,
    pub identity: IdentityStore,
    pub network: NetworkStore,
    pub messages: MessageStore,
    pub fleet: FleetStore,
    pub propagation: PropagationStore,
    pub content: ContentStore,
    pub scenario: ScenarioStore,
    pub activity: ActivityStore,
}

impl DomainStores {
    pub fn begin_session(&mut self, profile: &str, generation: ConnectionGeneration) {
        *self = Self::default();
        self.runtime.generation = generation;
        self.runtime.profile = profile.into();
        self.runtime.connection_mode = "connecting...".into();
    }

    pub fn fail_session(&mut self, generation: ConnectionGeneration, message: impl Into<String>) {
        if !self.accepts(generation) {
            return;
        }
        let message = message.into();
        tracing::warn!(target: "dx::session", error_bytes = message.len(), "backend session failed");
        let message = "backend session unavailable".to_string();
        self.runtime.connected = false;
        self.runtime.connection_mode = format!("failed: {message}");
        let state = DataState::Error { message };
        self.runtime.state = state.clone();
        self.identity.state = state.clone();
        self.network.state = state.clone();
        self.messages.state = state.clone();
        self.fleet.state = state.clone();
        self.propagation.state = state.clone();
        self.propagation.standard_state = state.clone();
        self.content.state = state.clone();
        self.scenario.state = state;
    }

    pub fn resolve_message(
        &mut self,
        generation: ConnectionGeneration,
        message_id: &str,
        message: Option<styrene_ipc::types::MessageInfo>,
    ) -> bool {
        if !self.accepts(generation) {
            return false;
        }
        if let Some(mut message) = message {
            message.projection_complete = true;
            return self
                .apply_daemon_event(generation, DaemonEvent::MessageReceived(Box::new(message)));
        }

        self.messages.messages.retain(|message| message.id != message_id);
        self.messages.live_message_ids.remove(message_id);
        self.messages.live_message_order.retain(|id| id != message_id);
        self.messages.state = DataState::Ready;
        true
    }

    pub fn apply_daemon_event(
        &mut self,
        generation: ConnectionGeneration,
        event: DaemonEvent,
    ) -> bool {
        if !self.accepts(generation) {
            return false;
        }
        let event = redact_daemon_event(event);
        let observation = match &event {
            DaemonEvent::NetworkOperation(value) => Some(&value.observation),
            DaemonEvent::Request(value) => Some(&value.observation),
            DaemonEvent::Resource(value) => Some(&value.observation),
            DaemonEvent::LinkObservation(value) => Some(&value.observation),
            DaemonEvent::RouteLifecycle(value) => Some(&value.observation),
            _ => None,
        };
        if observation.is_some_and(|value| !self.observation_generation_valid(value)) {
            return false;
        }
        let unchanged = match &event {
            DaemonEvent::NetworkOperation(operation) => self
                .network
                .operations
                .iter()
                .any(|item| item.operation_id == operation.operation_id && item == operation),
            DaemonEvent::Request(request) => self
                .network
                .requests
                .iter()
                .any(|item| item.request_id == request.request_id && item == request),
            DaemonEvent::Resource(resource) => self
                .network
                .resources
                .iter()
                .any(|item| item.resource_hash == resource.resource_hash && item == resource),
            DaemonEvent::LinkObservation(link) => self
                .network
                .links
                .iter()
                .any(|item| item.link_id == link.link_id && item == &link_info(link.clone())),
            _ => false,
        };
        if unchanged {
            return true;
        }
        let activity = activity_entry(&event);
        match event {
            DaemonEvent::Connected => {
                self.runtime.connected = true;
                self.runtime.connection_mode = self.runtime.profile.clone();
                self.runtime.state = DataState::Ready;
                self.content.state = DataState::Empty;
                self.scenario.state = DataState::Ready;
                self.fleet.state = DataState::Empty;
                self.propagation.state = DataState::Empty;
                self.propagation.standard_state = DataState::Empty;
                self.refresh_network_state();
            }
            DaemonEvent::EventGeneration(event_generation) => {
                if event_generation == 0 {
                    return false;
                }
                if self.runtime.event_server_generation.is_some()
                    && self.runtime.event_server_generation != Some(event_generation)
                {
                    self.runtime.event_server_generation = None;
                    return false;
                }
                self.runtime.event_server_generation = Some(event_generation);
            }
            DaemonEvent::Disconnected(reason) => {
                tracing::warn!(target: "dx::session", reason_bytes = reason.len(), "daemon disconnected");
                self.runtime.connected = false;
                self.runtime.connection_mode = "disconnected".into();
                self.runtime.state = DataState::Degraded { reason: "daemon disconnected".into() };
                self.runtime.capabilities = None;
                self.runtime.server_generation = None;
                self.runtime.event_server_generation = None;
                self.propagation.standard_snapshot = None;
                self.propagation.standard_state =
                    DataState::Degraded { reason: "daemon disconnected".into() };
            }
            DaemonEvent::Identity(info) => {
                self.identity.current = Some(info);
                self.identity.state = DataState::Ready;
            }
            DaemonEvent::Status(status) => {
                if self.runtime.server_generation.is_some()
                    && self.runtime.server_generation != status.connection_generation
                {
                    self.network.operations.clear();
                    self.network.requests.clear();
                    self.network.paths.clear();
                    self.network.links.clear();
                    self.network.resources.clear();
                    self.runtime.capabilities = None;
                    self.runtime.server_generation = None;
                    self.propagation.standard_snapshot = None;
                    self.propagation.standard_state = DataState::Empty;
                    return false;
                }
                self.runtime.server_generation =
                    status.connection_generation.filter(|value| *value != 0);
                self.runtime.capabilities =
                    self.runtime.server_generation.and(status.active_capabilities.clone());
                self.network.status = state::MeshStatusInfo {
                    transport_active: status.transport_enabled,
                    peer_count: status.device_count,
                    link_count: status.active_links,
                    interface_count: status.interface_count,
                    propagation_enabled: status.propagation_enabled,
                    uptime: status.uptime,
                    version: status.daemon_version,
                };
                self.propagation.enabled = status.propagation_enabled;
                self.propagation.state =
                    if status.propagation_enabled { DataState::Ready } else { DataState::Empty };
                self.refresh_network_state();
            }
            DaemonEvent::PeerDiscovered(device) => self.reduce_peer(device),
            DaemonEvent::LocalPageInventory(pages) => {
                self.content.local_inventory = pages;
                self.content.state = DataState::Ready;
            }
            DaemonEvent::MessageReceived(message) => {
                let complete = message.projection_complete;
                if !complete {
                    self.messages.state = DataState::Degraded {
                        reason: "sparse message event requires message requery".into(),
                    };
                    return false;
                }
                let incoming = state::ChatMessage::from(*message);
                let incoming_id = incoming.id.clone();
                if complete {
                    self.mark_message_live(incoming_id.clone());
                }
                if let Some(existing) =
                    self.messages.messages.iter_mut().find(|item| item.id == incoming.id)
                {
                    *existing = incoming;
                } else {
                    self.messages.messages.push(incoming);
                }
                self.sort_messages();
                self.messages.state = DataState::Ready;
            }
            DaemonEvent::MessagingOperation(_) => {
                self.clear_message_pagination();
                self.messages.live_message_ids.clear();
                self.messages.live_message_order.clear();
            }
            DaemonEvent::PathTable(paths) => self.set_paths(generation, paths),
            DaemonEvent::RouteLifecycle(_) => self.refresh_network_state(),
            DaemonEvent::NetworkOperation(operation) => self.upsert_operation(operation),
            DaemonEvent::Request(request) => self.upsert_request(request),
            DaemonEvent::Resource(resource) => self.upsert_resource(resource),
            DaemonEvent::ReconcileRequests { connection_generation, .. } => {
                if self.runtime.event_server_generation != Some(connection_generation) {
                    return false;
                }
            }
            DaemonEvent::ReconcileRequired { connection_generation, .. } => {
                if self.runtime.event_server_generation != Some(connection_generation) {
                    return false;
                }
                self.clear_message_pagination();
                self.messages.live_message_ids.clear();
                self.messages.live_message_order.clear();
            }
            DaemonEvent::StandardPropagationChanged { connection_generation } => {
                if self.runtime.event_server_generation != Some(connection_generation) {
                    return false;
                }
                return true;
            }
            DaemonEvent::LinkObservation(event) => {
                if let Some(link) =
                    self.network.links.iter_mut().find(|item| item.link_id == event.link_id)
                {
                    *link = link_info(event);
                } else {
                    self.network.links.push(link_info(event));
                }
                self.refresh_network_state();
            }
        }
        self.push_activity(activity);
        true
    }

    pub fn set_paths(&mut self, generation: ConnectionGeneration, paths: Vec<PathTableEntry>) {
        if !self.accepts(generation) {
            return;
        }
        let Some(expected) = self.runtime.server_generation else {
            return;
        };
        if paths.iter().any(|path| path.observation.connection_generation != Some(expected)) {
            return;
        }
        self.network.paths = paths
            .into_iter()
            .map(|entry| state::PathEntry {
                destination_hash: entry.destination_hash,
                hops: entry.hops,
                next_hop: entry.next_hop,
                interface: entry.interface,
                expires: entry.expires,
                observation: entry.observation,
            })
            .collect();
        self.refresh_network_state();
    }

    pub fn set_interfaces(
        &mut self,
        generation: ConnectionGeneration,
        mut interfaces: Vec<InterfaceStats>,
    ) {
        if !self.accepts(generation) {
            return;
        }
        let Some(expected) = self.runtime.server_generation else {
            return;
        };
        if interfaces
            .iter()
            .any(|interface| interface.observation.connection_generation != Some(expected))
        {
            return;
        }
        for interface in &mut interfaces {
            interface.local_endpoint = interface.local_endpoint.take().map(redact_endpoint);
            interface.remote_endpoint = interface.remote_endpoint.take().map(redact_endpoint);
            interface.host = interface.host.take().map(redact_endpoint);
        }
        self.network.interfaces = interfaces;
        self.refresh_network_state();
    }

    pub fn set_operations(
        &mut self,
        generation: ConnectionGeneration,
        mut operations: Vec<styrene_ipc::types::NetworkOperationInfo>,
    ) {
        if self.accepts(generation)
            && self.runtime.server_generation.is_some()
            && operations.iter().all(|value| {
                value.observation.connection_generation == self.runtime.server_generation
            })
        {
            for operation in &mut operations {
                if operation.detail.is_some() {
                    operation.detail = Some("daemon operation detail redacted".into());
                }
            }
            self.network.operations = operations;
        }
    }

    pub fn set_links(
        &mut self,
        generation: ConnectionGeneration,
        snapshot: styrene_ipc::types::LinkSnapshot,
    ) {
        if !self.accepts(generation) {
            return;
        }
        if self.runtime.server_generation.is_none() {
            return;
        }
        if snapshot
            .active
            .iter()
            .chain(&snapshot.history)
            .any(|value| value.observation.connection_generation != self.runtime.server_generation)
        {
            return;
        }
        self.network.links =
            snapshot.active.into_iter().chain(snapshot.history).map(link_info).collect();
        self.refresh_network_state();
    }

    pub fn set_requests(
        &mut self,
        generation: ConnectionGeneration,
        mut requests: Vec<styrene_ipc::types::RequestObservationInfo>,
    ) {
        if self.accepts(generation)
            && self.runtime.server_generation.is_some()
            && requests.iter().all(|value| {
                value.observation.connection_generation == self.runtime.server_generation
            })
        {
            for request in &mut requests {
                request.response = None;
            }
            self.network.requests = requests;
        }
    }

    pub fn set_resources(
        &mut self,
        generation: ConnectionGeneration,
        resources: Vec<styrene_ipc::types::ResourceTransferInfo>,
    ) {
        if self.accepts(generation)
            && self.runtime.server_generation.is_some()
            && resources.iter().all(|value| {
                value.observation.connection_generation == self.runtime.server_generation
            })
        {
            self.network.resources = resources;
        }
    }

    pub fn mutation_availability(&self, capability: &str) -> Result<(), String> {
        if !self.runtime.connected {
            return Err("daemon disconnected".into());
        }
        let Some(server_generation) = self.runtime.server_generation else {
            return Err("capabilities unknown: daemon generation was not negotiated".into());
        };
        let Some(capabilities) = &self.runtime.capabilities else {
            return Err("capabilities unknown".into());
        };
        if capabilities.version != styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION {
            return Err(format!(
                "capabilities stale: unsupported version {}",
                capabilities.version
            ));
        }
        if server_generation == 0 {
            return Err("capabilities stale: invalid daemon generation".into());
        }
        if let Some(degraded) = capabilities.degraded.iter().find(|item| item.id == capability) {
            return Err(format!("{capability} unavailable: {}", degraded.reason));
        }
        if !capabilities.authorized_operations.iter().any(|item| item == capability) {
            return Err(format!("permission denied: {capability}"));
        }
        Ok(())
    }

    pub fn mutation_availability_at(
        &self,
        generation: ConnectionGeneration,
        capability: &str,
    ) -> Result<(), String> {
        if !self.accepts(generation) {
            return Err("stale frontend connection generation".into());
        }
        self.mutation_availability(capability)
    }

    pub fn delivery_method_availability(&self, method: &str) -> Result<(), String> {
        self.mutation_availability("chat.send")?;
        let capabilities = self.runtime.capabilities.as_ref().ok_or("capabilities unknown")?;
        let runtime = |id: &str| capabilities.runtime.iter().any(|item| item == id);
        match method {
            "direct" | "opportunistic" if runtime("runtime.lxmf.direct") => Ok(()),
            "propagated"
                if runtime("runtime.standard-lxmf.propagation-client")
                    && self.propagation.standard_snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot
                            .selection
                            .as_ref()
                            .and_then(|selection| selection.peer_hash.as_ref())
                            .is_some()
                    }) =>
            {
                Ok(())
            }
            "paper" if runtime("runtime.lxmf.paper-export") => Ok(()),
            "direct" | "opportunistic" => Err("runtime.lxmf.direct is not active".into()),
            "propagated" if !runtime("runtime.standard-lxmf.propagation-client") => {
                Err("standard LXMF propagation client is not active".into())
            }
            "propagated" => Err("no authoritative propagation peer is selected".into()),
            "paper" => Err("paper export is not active".into()),
            _ => Err("unknown delivery method".into()),
        }
    }

    pub fn set_draft(
        &mut self,
        generation: ConnectionGeneration,
        peer_hash: &str,
        draft: Option<styrene_ipc::types::ConversationDraft>,
    ) {
        if !self.accepts(generation) {
            return;
        }
        match draft {
            Some(draft) => {
                self.messages.drafts.insert(peer_hash.into(), draft);
            }
            None => {
                self.messages.drafts.remove(peer_hash);
            }
        }
    }

    pub fn merge_conversation_page(
        &mut self,
        generation: ConnectionGeneration,
        conversations: Vec<state::ConversationEntry>,
        next_cursor: Option<String>,
        reset: bool,
    ) {
        if !self.accepts(generation) {
            return;
        }
        if reset {
            self.messages.conversations.clear();
        }
        for conversation in conversations {
            if let Some(existing) = self
                .messages
                .conversations
                .iter_mut()
                .find(|item| item.peer_hash == conversation.peer_hash)
            {
                *existing = conversation;
            } else {
                self.messages.conversations.push(conversation);
            }
        }
        self.messages.conversations.sort_by(|left, right| {
            right.pinned.cmp(&left.pinned).then_with(|| {
                right
                    .last_timestamp
                    .cmp(&left.last_timestamp)
                    .then_with(|| left.peer_hash.cmp(&right.peer_hash))
            })
        });
        self.messages.conversation_cursor = next_cursor;
        self.messages.state = if self.messages.conversations.is_empty() {
            DataState::Empty
        } else {
            DataState::Ready
        };
    }

    pub fn merge_peer_message_page(
        &mut self,
        generation: ConnectionGeneration,
        peer_hash: &str,
        messages: Vec<state::ChatMessage>,
        next_cursor: Option<String>,
    ) {
        if !self.accepts(generation) {
            return;
        }
        self.messages.loaded_peer_hashes.insert(peer_hash.to_string());
        for incoming in messages {
            let incoming_id = incoming.id.clone();
            if let Some(existing) =
                self.messages.messages.iter_mut().find(|item| item.id == incoming.id)
            {
                if !self.messages.live_message_ids.contains(&incoming.id) {
                    *existing = incoming;
                }
            } else {
                self.messages.messages.push(incoming);
            }
            self.messages.live_message_ids.remove(&incoming_id);
            self.messages.live_message_order.retain(|id| id != &incoming_id);
        }
        match next_cursor {
            Some(cursor) => {
                self.messages.message_cursors.insert(peer_hash.to_string(), cursor);
            }
            None => {
                self.messages.message_cursors.remove(peer_hash);
            }
        }
        self.sort_messages();
        self.messages.state =
            if self.messages.messages.is_empty() { DataState::Empty } else { DataState::Ready };
    }

    pub fn reset_peer_message_snapshot(
        &mut self,
        generation: ConnectionGeneration,
        peer_hash: &str,
    ) {
        if !self.accepts(generation) {
            return;
        }
        self.messages.messages.retain(|message| {
            let belongs = message.source == peer_hash || message.destination == peer_hash;
            !belongs || self.messages.live_message_ids.contains(&message.id)
        });
        self.messages.message_cursors.remove(peer_hash);
    }

    pub fn push_outgoing(&mut self, generation: ConnectionGeneration, message: state::ChatMessage) {
        if self.accepts(generation) {
            self.mark_message_live(message.id.clone());
            if let Some(existing) =
                self.messages.messages.iter_mut().find(|item| item.id == message.id)
            {
                *existing = message;
            } else {
                self.messages.messages.push(message);
            }
            self.sort_messages();
            self.messages.state = DataState::Ready;
        }
    }

    pub fn apply_send_outcome(
        &mut self,
        generation: ConnectionGeneration,
        peer_hash: String,
        submitted_content: String,
        outcome: styrene_ipc::types::SendChatOutcome,
    ) -> bool {
        if !self.accepts(generation) {
            return false;
        }
        self.push_outgoing(generation, state::ChatMessage::from(outcome.message));
        let accepted = match outcome.disposition {
            styrene_ipc::types::SendChatDisposition::Accepted => true,
            styrene_ipc::types::SendChatDisposition::PaperExported => {
                let Some(uri) = outcome.paper_uri else {
                    return false;
                };
                self.messages.paper_export =
                    Some(PaperExportState { message_id: outcome.message_id, uri });
                true
            }
            _ => false,
        };
        if accepted {
            self.messages.accepted_compose = Some((peer_hash, submitted_content));
        }
        accepted
    }

    pub fn apply_lifecycle_outcome(
        &mut self,
        generation: ConnectionGeneration,
        outcome: styrene_ipc::types::MessagingOperationOutcome,
    ) -> Option<String> {
        if !self.accepts(generation) {
            return None;
        }
        let had_message = outcome.message.is_some();
        if let Some(message) = outcome.message {
            self.push_outgoing(generation, state::ChatMessage::from(message));
        }
        if !had_message
            && matches!(
                outcome.disposition,
                styrene_ipc::types::MessagingDisposition::TerminalConflict
                    | styrene_ipc::types::MessagingDisposition::AlreadyCancelled
            )
        {
            self.messages.state = DataState::Degraded {
                reason: "terminal mutation requires authoritative message requery".into(),
            };
        }
        if outcome.disposition != styrene_ipc::types::MessagingDisposition::NotFound {
            return None;
        }
        let peer =
            self.messages.messages.iter().find(|message| message.id == outcome.target_id).map(
                |message| {
                    if message.is_outgoing {
                        message.destination.clone()
                    } else {
                        message.source.clone()
                    }
                },
            );
        self.messages.messages.retain(|message| message.id != outcome.target_id);
        self.messages.live_message_ids.remove(&outcome.target_id);
        self.messages.live_message_order.retain(|id| id != &outcome.target_id);
        if let Some(peer) = &peer {
            self.messages.message_cursors.remove(peer);
        }
        peer
    }

    pub fn clear_message_pagination(&mut self) {
        self.messages.conversation_cursor = None;
        self.messages.message_cursors.clear();
    }

    pub fn loaded_message_peers(&self) -> Vec<String> {
        let mut peers = self.messages.loaded_peer_hashes.iter().cloned().collect::<Vec<_>>();
        peers.sort();
        peers
    }

    pub fn mark_message_peer_loaded(&mut self, generation: ConnectionGeneration, peer_hash: &str) {
        if self.accepts(generation) {
            self.messages.loaded_peer_hashes.insert(peer_hash.to_string());
        }
    }

    fn mark_message_live(&mut self, id: String) {
        self.messages.live_message_order.retain(|live| live != &id);
        self.messages.live_message_order.push_back(id.clone());
        self.messages.live_message_ids.insert(id);
        let retained_limit =
            self.messages.messages.len().saturating_add(1).min(MESSAGE_TRANSIENT_CAPACITY);
        while self.messages.live_message_order.len() > retained_limit {
            if let Some(oldest) = self.messages.live_message_order.pop_front() {
                self.messages.live_message_ids.remove(&oldest);
            }
        }
    }

    fn sort_messages(&mut self) {
        let mut seen = HashSet::new();
        self.messages.messages.retain(|message| seen.insert(message.id.clone()));
        self.messages
            .messages
            .sort_by(|left, right| (left.timestamp, &left.id).cmp(&(right.timestamp, &right.id)));
    }

    pub fn set_page(&mut self, generation: ConnectionGeneration, mut page: state::PageView) {
        if !self.accepts(generation) {
            return;
        }
        if page.error.is_some() {
            page.error = Some("page request failed".into());
        }
        self.content.state = if page.loading {
            DataState::Loading
        } else if let Some(error) = &page.error {
            DataState::Error { message: error.clone() }
        } else {
            DataState::Ready
        };
        self.content.page = Some(page);
    }

    pub fn clear_page(&mut self, generation: ConnectionGeneration) {
        if self.accepts(generation) {
            self.content.page = None;
            self.content.download = None;
            self.content.state = DataState::Empty;
        }
    }

    pub fn set_download(
        &mut self,
        generation: ConnectionGeneration,
        mut download: styrene_ipc::types::FileDownloadInfo,
    ) {
        if self.accepts(generation) {
            if download.error.is_some() {
                download.error = Some("file download failed".into());
            }
            self.content.download = Some(download);
            self.content.state = DataState::Ready;
        }
    }

    pub fn set_scenario_run(&mut self, run: ScenarioRun) {
        self.scenario.run = Some(run);
        self.scenario.state = DataState::Ready;
    }

    pub fn update_scenario_run(&mut self, run: ScenarioRun) {
        if self.scenario.run.as_ref().is_some_and(|current| current.run_id == run.run_id) {
            self.scenario.run = Some(run);
            self.scenario.state = DataState::Ready;
        }
    }

    pub fn set_propagation_snapshot(
        &mut self,
        generation: ConnectionGeneration,
        mut snapshot: styrene_ipc::types::PropagationSnapshot,
        append: bool,
    ) {
        if !self.accepts(generation) {
            return;
        }
        if append && let Some(current) = &self.propagation.snapshot {
            let mut queue = current.queue.clone();
            queue.extend(snapshot.queue);
            queue.sort_by(|left, right| left.id.cmp(&right.id));
            queue.dedup_by(|left, right| left.id == right.id);
            snapshot.queue = queue;
        }
        self.propagation.enabled = snapshot.enabled;
        self.propagation.state = if snapshot.enabled { DataState::Ready } else { DataState::Empty };
        self.propagation.snapshot = Some(snapshot);
    }

    pub fn fail_propagation(
        &mut self,
        generation: ConnectionGeneration,
        reason: impl Into<String>,
    ) {
        if self.accepts(generation) {
            let reason = reason.into();
            tracing::warn!(target: "dx::propagation", reason_bytes = reason.len(), "propagation refresh failed");
            self.propagation.state =
                DataState::Degraded { reason: "propagation refresh failed".into() };
        }
    }

    pub fn set_standard_propagation_snapshot(
        &mut self,
        generation: ConnectionGeneration,
        snapshot: styrene_ipc::types::StandardPropagationSnapshot,
    ) {
        if !self.accepts(generation)
            || snapshot.connection_generation != self.runtime.server_generation
        {
            return;
        }
        if snapshot.version != styrene_ipc::types::STANDARD_PROPAGATION_SNAPSHOT_VERSION {
            self.propagation.standard_state = DataState::Degraded {
                reason: format!("unsupported standard propagation snapshot v{}", snapshot.version),
            };
            return;
        }
        self.propagation.standard_state =
            if snapshot.active { DataState::Ready } else { DataState::Empty };
        self.propagation.standard_snapshot = Some(snapshot);
    }

    pub fn fail_standard_propagation(
        &mut self,
        generation: ConnectionGeneration,
        reason: impl Into<String>,
    ) {
        if self.accepts(generation) {
            let reason = reason.into();
            tracing::warn!(target: "dx::propagation", reason_bytes = reason.len(), "standard propagation refresh failed");
            self.propagation.standard_state =
                DataState::Degraded { reason: "standard propagation refresh failed".into() };
        }
    }

    pub fn begin_fleet_job(
        &mut self,
        generation: ConnectionGeneration,
        target: String,
        operation: FleetOperation,
    ) -> Option<String> {
        if !self.accepts(generation) {
            return None;
        }
        let operation = redact_fleet_operation(operation);
        let id = format!("fleet-{}-{}", generation.0, self.fleet.jobs.len() + 1);
        self.fleet.jobs.push(FleetJob {
            id: id.clone(),
            target,
            operation,
            state: FleetJobState::Running,
            started_at: now(),
            finished_at: None,
            result: None,
            error: None,
        });
        if self.fleet.jobs.len() > TIMELINE_CAPACITY {
            let excess = self.fleet.jobs.len() - TIMELINE_CAPACITY;
            self.fleet.jobs.drain(..excess);
        }
        self.fleet.state = DataState::Ready;
        Some(id)
    }

    pub fn finish_fleet_job(
        &mut self,
        generation: ConnectionGeneration,
        id: &str,
        outcome: Result<String, String>,
    ) {
        if !self.accepts(generation) {
            return;
        }
        let Some(job) = self.fleet.jobs.iter_mut().find(|job| job.id == id) else {
            return;
        };
        job.finished_at = Some(now());
        match outcome {
            Ok(result) => {
                tracing::debug!(target: "dx::fleet", result_bytes = result.len(), "Fleet operation completed");
                job.state = FleetJobState::Succeeded;
                job.result = Some(format!("{} completed", job.operation.label()));
            }
            Err(error) => {
                job.state = classify_fleet_error(&error);
                job.error = Some(format!("{:?}", job.state));
            }
        }
    }

    pub fn set_fleet_status(
        &mut self,
        generation: ConnectionGeneration,
        status: styrene_ipc::types::RemoteStatusInfo,
    ) {
        if !self.accepts(generation) {
            return;
        }
        if let Some(existing) = self
            .fleet
            .statuses
            .iter_mut()
            .find(|existing| existing.destination_hash == status.destination_hash)
        {
            *existing = status;
        } else {
            self.fleet.statuses.push(status);
        }
    }

    pub fn route_state(
        &self,
        route: &state::AppRoute,
        capabilities: BackendCapabilities,
    ) -> DataState {
        match route {
            state::AppRoute::Command | state::AppRoute::System => self.runtime.state.clone(),
            state::AppRoute::Network => self.network.state.clone(),
            state::AppRoute::Messages if !capabilities.messaging => {
                DataState::Degraded { reason: "messaging is unsupported by this backend".into() }
            }
            state::AppRoute::Messages => self.messages.state.clone(),
            state::AppRoute::Fleet if !capabilities.fleet => DataState::Degraded {
                reason: "fleet operations are unsupported by this backend".into(),
            },
            state::AppRoute::Fleet => self.fleet.state.clone(),
            state::AppRoute::Propagation if !capabilities.propagation => DataState::Degraded {
                reason: "propagation inspection is unsupported by this backend".into(),
            },
            state::AppRoute::Propagation => self.propagation.state.clone(),
            state::AppRoute::Content if !capabilities.content => DataState::Degraded {
                reason: "content browsing is unsupported by this backend".into(),
            },
            state::AppRoute::Content => self.content.state.clone(),
            state::AppRoute::Lab if !capabilities.scenarios => DataState::Degraded {
                reason: "scenario execution is unsupported by this backend".into(),
            },
            state::AppRoute::Lab => self.scenario.state.clone(),
        }
    }

    pub fn command_summary(&self) -> CommandSummary {
        CommandSummary {
            transport_active: self.network.status.transport_active,
            interface_count: self.network.status.interface_count,
            observed_peers: self.network.peers.len(),
            route_count: self.network.paths.len(),
            active_links: self.network.status.link_count,
            link_records: self.network.links.len(),
            propagation_enabled: self.propagation.enabled,
        }
    }

    pub fn export_activity(
        &self,
        diagnostics: crate::daemon_bridge::BrokerDiagnostics,
    ) -> Result<String, String> {
        #[derive(Serialize)]
        struct DiagnosticsExport<'a> {
            schema_version: u8,
            profile: &'a str,
            client_generation: u64,
            server_generation: Option<u64>,
            diagnostics: DiagnosticsSummary,
            activity: &'a [state::ActivityEntry],
        }

        #[derive(Serialize)]
        struct DiagnosticsSummary {
            queue_depth: usize,
            in_flight: usize,
            completed: u64,
            timed_out: u64,
            cancelled: u64,
            overloaded: u64,
            disconnected: u64,
            reconnects: u64,
            stale_responses: u64,
            dropped_responses: u64,
            dropped_updates: u64,
            last_latency_ms: u64,
        }

        let export = DiagnosticsExport {
            schema_version: 1,
            profile: &self.runtime.profile,
            client_generation: self.runtime.generation.0,
            server_generation: self.runtime.server_generation,
            diagnostics: DiagnosticsSummary {
                queue_depth: diagnostics.queue_depth,
                in_flight: diagnostics.in_flight,
                completed: diagnostics.completed,
                timed_out: diagnostics.timed_out,
                cancelled: diagnostics.cancelled,
                overloaded: diagnostics.overloaded,
                disconnected: diagnostics.disconnected,
                reconnects: diagnostics.reconnects,
                stale_responses: diagnostics.stale_responses,
                dropped_responses: diagnostics.dropped_responses,
                dropped_updates: diagnostics.dropped_updates,
                last_latency_ms: diagnostics.last_latency_ms,
            },
            activity: &self.activity.entries,
        };
        serde_json::to_string_pretty(&export).map_err(|error| error.to_string())
    }

    fn accepts(&self, generation: ConnectionGeneration) -> bool {
        self.runtime.generation == generation
    }

    fn reduce_peer(&mut self, device: styrene_ipc::types::DeviceInfo) {
        let parsed = state::parse_announce_name(&device.name);
        let native_page_host = device
            .discovered_capabilities
            .contains(&styrene_ipc::types::DiscoveredCapability::NativeNomadNetHost);
        let role = if native_page_host {
            state::PeerRole::PageHost
        } else if parsed.is_styrene {
            if parsed.role == state::PeerRole::PageHost {
                state::PeerRole::Styrene
            } else {
                parsed.role
            }
        } else if device.is_styrene_node {
            state::PeerRole::Styrene
        } else {
            state::PeerRole::Rns
        };
        let entry = state::PeerEntry {
            hash: device.destination_hash.clone(),
            identity_hash: (!device.identity_hash.is_empty()).then_some(device.identity_hash),
            name: (!parsed.display_name.is_empty()).then_some(parsed.display_name),
            status: device.status,
            node_role: role.clone(),
            capabilities: parsed.capabilities,
            version: parsed.version,
            last_announce: device.last_announce,
            announce_count: device.announce_count,
        };
        if let Some(existing) =
            self.network.peers.iter_mut().find(|item| item.hash == device.destination_hash)
        {
            if entry.last_announce >= existing.last_announce {
                *existing = entry.clone();
            }
        } else {
            self.network.peers.push(entry.clone());
        }
        if entry.node_role != state::PeerRole::Rns
            && !self.fleet.managed_peers.contains(&entry.hash)
        {
            self.fleet.managed_peers.push(entry.hash.clone());
            self.fleet.state = DataState::Ready;
        }
        if !self.network.announces.iter().any(|announce| {
            announce.peer_hash == entry.hash
                && announce.timestamp == entry.last_announce.unwrap_or(0)
        }) {
            self.network.announces.push(state::AnnounceEvent {
                peer_hash: entry.hash,
                peer_name: entry.name,
                timestamp: entry.last_announce.unwrap_or_else(now),
                node_role: role,
            });
            bound(&mut self.network.announces);
        }
        self.refresh_network_state();
    }

    fn upsert_operation(&mut self, operation: styrene_ipc::types::NetworkOperationInfo) {
        if let Some(current) = self
            .network
            .operations
            .iter_mut()
            .find(|current| current.operation_id == operation.operation_id)
        {
            *current = operation;
        } else {
            self.network.operations.push(operation);
        }
    }

    fn upsert_request(&mut self, request: styrene_ipc::types::RequestObservationInfo) {
        if let Some(current) = self
            .network
            .requests
            .iter_mut()
            .find(|current| current.request_id == request.request_id)
        {
            *current = request;
        } else {
            self.network.requests.push(request);
        }
    }

    fn upsert_resource(&mut self, resource: styrene_ipc::types::ResourceTransferInfo) {
        if let Some(current) = self
            .network
            .resources
            .iter_mut()
            .find(|current| current.resource_hash == resource.resource_hash)
        {
            *current = resource;
        } else {
            self.network.resources.push(resource);
        }
    }

    fn observation_generation_valid(
        &self,
        observation: &styrene_ipc::types::ObservationMetadata,
    ) -> bool {
        observation.connection_generation.is_some_and(|actual| {
            self.runtime.server_generation == Some(actual)
                || self.runtime.event_server_generation == Some(actual)
        })
    }

    fn refresh_network_state(&mut self) {
        self.network.state = if !self.runtime.connected {
            DataState::Degraded { reason: "backend disconnected".into() }
        } else if self.network.peers.is_empty()
            && self.network.paths.is_empty()
            && self.network.interfaces.is_empty()
        {
            DataState::Empty
        } else {
            DataState::Ready
        };
    }

    fn push_activity(&mut self, entry: state::ActivityEntry) {
        self.activity.entries.push(entry);
        bound(&mut self.activity.entries);
    }
}

fn redact_fleet_operation(operation: FleetOperation) -> FleetOperation {
    match operation {
        FleetOperation::Execute { args, .. } => FleetOperation::Execute {
            command: "[REDACTED]".into(),
            args: vec![format!("{} arguments redacted", args.len())],
        },
        other => other,
    }
}

fn bound<T>(values: &mut Vec<T>) {
    if values.len() > TIMELINE_CAPACITY {
        values.drain(..values.len() - TIMELINE_CAPACITY);
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn classify_fleet_error(error: &str) -> FleetJobState {
    let error = error.to_ascii_lowercase();
    if error.contains("permission denied") || error.contains("unauthorized") {
        FleetJobState::Denied
    } else if error.contains("not implemented") || error.contains("unsupported") {
        FleetJobState::Unsupported
    } else if error.contains("timed out") || error.contains("timeout") {
        FleetJobState::TimedOut
    } else {
        FleetJobState::Failed
    }
}

fn activity_entry(event: &DaemonEvent) -> state::ActivityEntry {
    let (severity, kind, summary, entity) = match event {
        DaemonEvent::Connected => {
            (state::ActivitySeverity::Info, "session", "Backend session connected".into(), None)
        }
        DaemonEvent::EventGeneration(generation) => (
            state::ActivitySeverity::Info,
            "session",
            format!("Event connection generation {generation} negotiated"),
            None,
        ),
        DaemonEvent::Disconnected(_) => {
            (state::ActivitySeverity::Error, "session", "Backend session disconnected".into(), None)
        }
        DaemonEvent::Identity(info) => (
            state::ActivitySeverity::Info,
            "identity",
            "Local identity loaded".into(),
            Some(info.destination_hash.clone()),
        ),
        DaemonEvent::Status(_) => {
            (state::ActivitySeverity::Info, "status", "Runtime status refreshed".into(), None)
        }
        DaemonEvent::PeerDiscovered(peer) => (
            state::ActivitySeverity::Info,
            "discovery",
            "Peer observation accepted".into(),
            Some(peer.destination_hash.clone()),
        ),
        DaemonEvent::LocalPageInventory(pages) => (
            state::ActivitySeverity::Info,
            "content",
            format!("Local native content inventory contains {} entries", pages.len()),
            None,
        ),
        DaemonEvent::MessageReceived(message) => (
            state::ActivitySeverity::Info,
            "message",
            "Message received".into(),
            Some(message.id.clone()),
        ),
        DaemonEvent::MessagingOperation(outcome) => (
            state::ActivitySeverity::Info,
            "message",
            format!("Messaging operation: {:?}", outcome.disposition).to_ascii_lowercase(),
            Some(outcome.target_id.clone()),
        ),
        DaemonEvent::LinkObservation(event) => (
            state::ActivitySeverity::Info,
            "link",
            format!("Link status changed to {}", event.status),
            Some(event.link_id.clone()),
        ),
        DaemonEvent::PathTable(paths) => (
            state::ActivitySeverity::Info,
            "routes",
            format!("Route snapshot contains {} entries", paths.len()),
            None,
        ),
        DaemonEvent::RouteLifecycle(event) => (
            if event.kind == styrene_ipc::types::RouteEventKind::Lost {
                state::ActivitySeverity::Warning
            } else {
                state::ActivitySeverity::Info
            },
            "route",
            format!("Route {:?}", event.kind).to_ascii_lowercase(),
            Some(event.route.destination_hash.clone()),
        ),
        DaemonEvent::NetworkOperation(operation) => (
            if operation.outcome.is_some() {
                state::ActivitySeverity::Info
            } else {
                state::ActivitySeverity::Warning
            },
            "network_operation",
            format!(
                "{}: {}",
                operation.kind.as_str(),
                operation
                    .outcome
                    .map(|value| value.as_str())
                    .unwrap_or(operation.progress.as_str())
            ),
            Some(operation.operation_id.clone()),
        ),
        DaemonEvent::Request(request) => (
            state::ActivitySeverity::Info,
            "request",
            format!("request: {:?}", request.state).to_ascii_lowercase(),
            Some(request.request_id.clone()),
        ),
        DaemonEvent::Resource(resource) => (
            state::ActivitySeverity::Info,
            "resource",
            format!("resource transfer: {:?}", resource.state).to_ascii_lowercase(),
            Some(resource.resource_hash.clone()),
        ),
        DaemonEvent::ReconcileRequests { dropped, .. } => (
            state::ActivitySeverity::Warning,
            "request",
            format!("request event gap ({dropped} dropped); reconciling snapshot"),
            None,
        ),
        DaemonEvent::ReconcileRequired { dropped, .. } => (
            state::ActivitySeverity::Warning,
            "session",
            format!("event gap ({dropped} dropped); reconciling all network snapshots"),
            None,
        ),
        DaemonEvent::StandardPropagationChanged { .. } => (
            state::ActivitySeverity::Info,
            "propagation",
            "standard propagation state changed; refreshing snapshot".into(),
            None,
        ),
    };
    let correlation_id = match event {
        DaemonEvent::RouteLifecycle(event) => event.observation.correlation_id.clone(),
        DaemonEvent::NetworkOperation(operation) => operation.observation.correlation_id.clone(),
        DaemonEvent::Request(request) => request.observation.correlation_id.clone(),
        DaemonEvent::Resource(resource) => resource.observation.correlation_id.clone(),
        _ => None,
    };
    let provenance = match event {
        DaemonEvent::RouteLifecycle(event) => event.observation.source.as_str(),
        DaemonEvent::NetworkOperation(operation) => operation.observation.source.as_str(),
        DaemonEvent::Request(request) => request.observation.source.as_str(),
        DaemonEvent::Resource(resource) => resource.observation.source.as_str(),
        DaemonEvent::LinkObservation(link) => link.observation.source.as_str(),
        DaemonEvent::PeerDiscovered(_) if kind == "discovery" => "daemon-discovery",
        _ => "daemon-session",
    }
    .to_string();
    state::ActivityEntry {
        timestamp: now(),
        severity,
        kind,
        summary,
        entity: entity.map(|value| redact_diagnostic_identifier(&value)),
        correlation_id: correlation_id.map(|value| redact_diagnostic_identifier(&value)),
        provenance,
    }
}

fn redact_diagnostic_identifier(value: &str) -> String {
    if value.is_empty()
        || value.len() > 128
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        "[REDACTED]".into()
    } else {
        value.into()
    }
}

fn redact_daemon_event(event: DaemonEvent) -> DaemonEvent {
    match event {
        DaemonEvent::Disconnected(_) => DaemonEvent::Disconnected("[REDACTED]".into()),
        DaemonEvent::NetworkOperation(mut operation) => {
            if operation.detail.is_some() {
                operation.detail = Some("daemon operation detail redacted".into());
            }
            DaemonEvent::NetworkOperation(operation)
        }
        DaemonEvent::Request(mut request) => {
            request.response = None;
            DaemonEvent::Request(request)
        }
        other => other,
    }
}

fn redact_endpoint(value: String) -> String {
    if value.len() > 256 || value.starts_with('/') || value.starts_with('~') {
        return "[REDACTED]".into();
    }
    let without_query = value.split_once('?').map_or(value.as_str(), |(endpoint, _)| endpoint);
    let lowered = without_query.to_ascii_lowercase();
    if ["token=", "password=", "secret="].iter().any(|marker| lowered.contains(marker)) {
        return "[REDACTED]".into();
    }
    if let Some((scheme, remainder)) = without_query.split_once("://")
        && let Some((_, host)) = remainder.rsplit_once('@')
    {
        return format!("{scheme}://[REDACTED]@{host}");
    }
    without_query.into()
}

fn link_info(event: styrene_ipc::types::LinkEvent) -> state::LinkInfo {
    state::LinkInfo {
        link_id: event.link_id,
        peer_hash: event.peer_hash,
        status: event.status,
        activity: event.activity,
        rtt_ms: event.rtt_ms,
        timestamp: event.timestamp,
        observation: event.observation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use styrene_ipc::types::{DeviceInfo, ObservationMetadata};

    fn stores() -> DomainStores {
        let mut stores = DomainStores::default();
        stores.begin_session("Fixture", ConnectionGeneration(2));
        stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::Connected);
        stores
    }

    fn message_status(id: &str, status: &str) -> DaemonEvent {
        let mut message = styrene_ipc::types::MessageInfo::default();
        message.id = id.into();
        message.status = status.into();
        DaemonEvent::MessageReceived(Box::new(message))
    }

    #[test]
    fn stale_generation_cannot_mutate_current_store() {
        let mut stores = stores();
        assert!(!stores.apply_daemon_event(
            ConnectionGeneration(1),
            DaemonEvent::Disconnected("old connection".into())
        ));
        assert!(stores.runtime.connected);
        assert!(stores.activity.entries.iter().all(|entry| !entry.summary.contains("old")));
    }

    #[test]
    fn page_close_only_clears_the_origin_generation() {
        let mut stores = stores();
        stores.set_page(
            ConnectionGeneration(2),
            state::PageView::loading("host".into(), "/page/index.mu".into()),
        );

        stores.clear_page(ConnectionGeneration(1));
        assert!(stores.content.page.is_some());

        stores.clear_page(ConnectionGeneration(2));
        assert!(stores.content.page.is_none());
        assert_eq!(stores.content.state, DataState::Empty);
    }

    #[test]
    fn local_page_inventory_is_generation_scoped_and_preserves_handler_truth() {
        let mut stores = stores();
        let mut page = styrene_ipc::types::PageInfo::default();
        page.path = "/page/index.mu".into();
        page.kind = "page".into();
        page.handler_active = true;

        stores.apply_daemon_event(
            ConnectionGeneration(1),
            DaemonEvent::LocalPageInventory(vec![page.clone()]),
        );
        assert!(stores.content.local_inventory.is_empty());

        stores.apply_daemon_event(
            ConnectionGeneration(2),
            DaemonEvent::LocalPageInventory(vec![page.clone()]),
        );
        assert_eq!(stores.content.local_inventory, [page]);
    }

    #[test]
    fn reconnect_replaces_capabilities_and_rejects_stale_status() {
        let mut stores = stores();
        let mut capabilities = styrene_ipc::types::ActiveCapabilitiesInfo::default();
        capabilities.version = 1;
        capabilities.runtime = vec!["runtime.lxmf.direct".into()];
        let mut status = styrene_ipc::types::DaemonStatusInfo::default();
        status.active_capabilities = Some(capabilities);
        status.connection_generation = Some(9);
        stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::Status(status.clone()));
        assert_eq!(stores.runtime.server_generation, Some(9));

        stores.begin_session("Reconnected", ConnectionGeneration(3));
        assert!(stores.runtime.capabilities.is_none());
        assert!(!stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::Status(status)));
        assert!(stores.runtime.capabilities.is_none());

        let mut replacement = styrene_ipc::types::DaemonStatusInfo::default();
        replacement.active_capabilities = Some(Default::default());
        replacement.connection_generation = Some(10);
        assert!(
            stores.apply_daemon_event(ConnectionGeneration(3), DaemonEvent::Status(replacement))
        );
        assert_eq!(stores.runtime.server_generation, Some(10));
        assert!(stores.runtime.capabilities.is_some());
    }

    #[test]
    fn snapshot_from_another_server_connection_cannot_replace_current_state() {
        let mut stores = stores();
        stores.runtime.server_generation = Some(9);

        let mut current_observation = ObservationMetadata::default();
        current_observation.connection_generation = Some(9);
        current_observation.correlation_id = Some("request-1".into());
        let current = PathTableEntry {
            destination_hash: "current".into(),
            hops: 1,
            next_hop: String::new(),
            interface: String::new(),
            expires: None,
            observation: current_observation,
        };
        stores.set_paths(ConnectionGeneration(2), vec![current]);
        assert_eq!(
            stores.network.paths[0].observation.correlation_id.as_deref(),
            Some("request-1")
        );

        let mut other_observation = ObservationMetadata::default();
        other_observation.connection_generation = Some(10);
        let other = PathTableEntry {
            destination_hash: "other".into(),
            hops: 1,
            next_hop: String::new(),
            interface: String::new(),
            expires: None,
            observation: other_observation,
        };
        stores.set_paths(ConnectionGeneration(2), vec![other]);

        assert_eq!(stores.network.paths.len(), 1);
        assert_eq!(stores.network.paths[0].destination_hash, "current");
    }

    #[test]
    fn route_loss_is_history_until_authoritative_snapshot_refreshes() {
        let mut stores = stores();
        stores.runtime.server_generation = Some(9);
        stores.network.paths.push(state::PathEntry {
            destination_hash: "peer".into(),
            hops: 1,
            next_hop: "peer".into(),
            interface: "tcp".into(),
            expires: Some(700),
            observation: ObservationMetadata::default(),
        });
        let mut event = styrene_ipc::types::RouteEventInfo::default();
        event.kind = styrene_ipc::types::RouteEventKind::Lost;
        event.route.destination_hash = "peer".into();
        event.observation.connection_generation = Some(9);
        event.observation.correlation_id = Some("path-request-1".into());

        stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::RouteLifecycle(event));

        assert_eq!(stores.network.paths.len(), 1, "events do not replace active snapshots");
        assert_eq!(stores.activity.entries.last().unwrap().kind, "route");
        assert_eq!(
            stores.activity.entries.last().unwrap().correlation_id.as_deref(),
            Some("path-request-1")
        );
    }

    #[test]
    fn newer_peer_event_reduces_over_snapshot_without_duplicates() {
        let mut stores = stores();
        let mut initial = DeviceInfo::default();
        initial.destination_hash = "peer".into();
        initial.name = "Initial".into();
        initial.status = "online".into();
        initial.last_announce = Some(10);
        stores.apply_daemon_event(
            ConnectionGeneration(2),
            DaemonEvent::PeerDiscovered(initial.clone()),
        );
        initial.name = "Newer".into();
        initial.last_announce = Some(11);
        stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::PeerDiscovered(initial));

        assert_eq!(stores.network.peers.len(), 1);
        assert_eq!(stores.network.peers[0].name.as_deref(), Some("Newer"));
        assert_eq!(stores.network.announces.len(), 2);
    }

    #[test]
    fn peer_inventory_preserves_identity_hash_for_administrative_actions() {
        let mut stores = stores();
        let mut device = DeviceInfo::default();
        device.destination_hash = "destination".into();
        device.identity_hash = "identity".into();
        stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::PeerDiscovered(device));

        assert_eq!(stores.network.peers[0].identity_hash.as_deref(), Some("identity"));
    }

    #[test]
    fn only_native_announce_capability_projects_a_page_host() {
        let mut stores = stores();
        let mut placeholder = DeviceInfo::default();
        placeholder.destination_hash = "placeholder".into();
        placeholder.name = "styrene:Legacy Peer:1.0:fleet,pages".into();
        stores
            .apply_daemon_event(ConnectionGeneration(2), DaemonEvent::PeerDiscovered(placeholder));

        let mut native = DeviceInfo::default();
        native.destination_hash = "0123456789abcdef0123456789abcdef".into();
        native.discovered_capabilities =
            vec![styrene_ipc::types::DiscoveredCapability::NativeNomadNetHost];
        stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::PeerDiscovered(native));

        assert_ne!(stores.network.peers[0].node_role, state::PeerRole::PageHost);
        assert_eq!(stores.network.peers[1].node_role, state::PeerRole::PageHost);
    }

    #[test]
    fn older_peer_event_cannot_replace_newer_observation() {
        let mut stores = stores();
        for (name, timestamp) in [("New", 20), ("Old", 10)] {
            let mut device = DeviceInfo::default();
            device.destination_hash = "peer".into();
            device.name = name.into();
            device.last_announce = Some(timestamp);
            stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::PeerDiscovered(device));
        }
        assert_eq!(stores.network.peers[0].name.as_deref(), Some("New"));
    }

    #[test]
    fn activity_and_announce_timelines_are_bounded() {
        let mut stores = stores();
        for index in 0..250 {
            let mut device = DeviceInfo::default();
            device.destination_hash = format!("peer-{index}");
            device.name = format!("Peer {index}");
            device.last_announce = Some(index);
            stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::PeerDiscovered(device));
        }
        assert_eq!(stores.activity.entries.len(), TIMELINE_CAPACITY);
        assert_eq!(stores.network.announces.len(), TIMELINE_CAPACITY);
    }

    #[test]
    fn activity_export_is_bounded_correlated_provenanced_and_redacted() {
        let mut stores = stores();
        stores.runtime.server_generation = Some(9);
        let mut event = styrene_ipc::types::RouteEventInfo::default();
        event.kind = styrene_ipc::types::RouteEventKind::Discovered;
        event.route.destination_hash = "peer".into();
        event.observation.source = styrene_ipc::types::ObservationSource::Fixture;
        event.observation.connection_generation = Some(9);
        event.observation.correlation_id = Some("token=structured-secret".into());
        stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::RouteLifecycle(event));
        for index in 0..TIMELINE_CAPACITY {
            stores.push_activity(state::ActivityEntry {
                timestamp: index as i64,
                severity: state::ActivitySeverity::Info,
                kind: "fixture",
                summary: "bounded fixture event".into(),
                entity: Some(format!("peer-{index}")),
                correlation_id: Some(format!("correlation-{index}")),
                provenance: "fixture".into(),
            });
        }

        let encoded = stores
            .export_activity(crate::daemon_bridge::BrokerDiagnostics::default())
            .expect("serialize diagnostics");
        assert_eq!(stores.activity.entries.len(), TIMELINE_CAPACITY);
        assert!(encoded.contains("\"severity\": \"info\""));
        assert!(encoded.contains("\"provenance\": \"fixture\""));
        assert!(encoded.contains("correlation-199"));
        assert!(!encoded.contains("structured-secret"));
    }

    #[test]
    fn error_fixture_projects_error_state_to_every_route() {
        let mut stores = DomainStores::default();
        stores.begin_session("Fixture", ConnectionGeneration(3));
        stores.fail_session(ConnectionGeneration(3), "deterministic Fixture error state");
        let capabilities = BackendCapabilities {
            messaging: true,
            content: true,
            fleet: true,
            propagation: true,
            scenarios: true,
            administration: true,
        };
        for route in [
            state::AppRoute::Command,
            state::AppRoute::Network,
            state::AppRoute::Messages,
            state::AppRoute::Fleet,
            state::AppRoute::Propagation,
            state::AppRoute::Content,
            state::AppRoute::Lab,
            state::AppRoute::System,
        ] {
            assert!(matches!(stores.route_state(&route, capabilities), DataState::Error { .. }));
        }
    }

    #[test]
    fn every_route_preserves_explicit_state_classes() {
        let routes = [
            state::AppRoute::Command,
            state::AppRoute::Network,
            state::AppRoute::Messages,
            state::AppRoute::Fleet,
            state::AppRoute::Propagation,
            state::AppRoute::Content,
            state::AppRoute::Lab,
            state::AppRoute::System,
        ];
        let capabilities = BackendCapabilities {
            messaging: true,
            content: true,
            fleet: true,
            propagation: true,
            scenarios: true,
            administration: true,
        };
        for expected in [
            DataState::Loading,
            DataState::Empty,
            DataState::Ready,
            DataState::Degraded { reason: "fixture degraded".into() },
            DataState::Error { message: "fixture error".into() },
        ] {
            let mut stores = DomainStores::default();
            stores.runtime.state = expected.clone();
            stores.network.state = expected.clone();
            stores.messages.state = expected.clone();
            stores.fleet.state = expected.clone();
            stores.propagation.state = expected.clone();
            stores.content.state = expected.clone();
            stores.scenario.state = expected.clone();
            for route in &routes {
                assert_eq!(stores.route_state(route, capabilities), expected, "route {route:?}");
            }
        }
    }

    #[test]
    fn missing_capability_is_degraded_without_discarding_domain_data() {
        let mut stores = stores();
        stores.network.peers.push(state::PeerEntry {
            hash: "peer".into(),
            identity_hash: None,
            name: Some("Peer".into()),
            status: "online".into(),
            node_role: state::PeerRole::Styrene,
            capabilities: Vec::new(),
            version: None,
            last_announce: None,
            announce_count: 1,
        });
        let route_state =
            stores.route_state(&state::AppRoute::Fleet, BackendCapabilities::default());
        assert!(matches!(route_state, DataState::Degraded { .. }));
        assert_eq!(stores.network.peers.len(), 1);
    }

    #[test]
    fn command_summary_is_derived_from_authoritative_stores() {
        let mut stores = stores();
        stores.network.status.transport_active = true;
        stores.network.status.interface_count = 2;
        stores.network.status.link_count = 1;
        stores.propagation.enabled = true;
        stores.network.paths.push(state::PathEntry {
            destination_hash: "peer".into(),
            hops: 1,
            next_hop: "peer".into(),
            interface: "tcp".into(),
            expires: None,
            observation: Default::default(),
        });
        let summary = stores.command_summary();
        assert!(summary.transport_active);
        assert_eq!(summary.interface_count, 2);
        assert_eq!(summary.route_count, 1);
        assert_eq!(summary.active_links, 1);
        assert!(summary.propagation_enabled);
    }

    #[test]
    fn sparse_status_waits_for_authoritative_resolution() {
        let mut stores = stores();
        for id in ["first", "second"] {
            let mut info = styrene_ipc::types::MessageInfo::default();
            info.projection_complete = true;
            info.id = id.into();
            info.timestamp = 1;
            stores.apply_daemon_event(
                ConnectionGeneration(2),
                DaemonEvent::MessageReceived(Box::new(info)),
            );
        }
        stores.apply_daemon_event(ConnectionGeneration(2), message_status("second", "delivered"));
        assert_eq!(stores.messages.messages[0].status, "");
        assert_eq!(stores.messages.messages[1].status, "");

        let mut resolved = styrene_ipc::types::MessageInfo::default();
        resolved.id = "second".into();
        resolved.status = "delivered".into();
        resolved.lifecycle_state = styrene_ipc::types::MessageLifecycleState::Delivered;
        assert!(!stores.resolve_message(ConnectionGeneration(1), "second", Some(resolved.clone())));
        assert_eq!(
            stores
                .messages
                .messages
                .iter()
                .find(|message| message.id == "second")
                .map(|message| message.status.as_str()),
            Some("")
        );
        assert!(stores.resolve_message(ConnectionGeneration(2), "second", Some(resolved)));
        assert_eq!(
            stores
                .messages
                .messages
                .iter()
                .find(|message| message.id == "second")
                .map(|message| message.status.as_str()),
            Some("delivered")
        );

        assert!(!stores.resolve_message(ConnectionGeneration(1), "second", None));
        assert!(stores.messages.messages.iter().any(|message| message.id == "second"));
        assert!(stores.resolve_message(ConnectionGeneration(2), "second", None));
        assert!(!stores.messages.messages.iter().any(|message| message.id == "second"));
    }

    fn chat(id: &str, timestamp: i64, status: &str) -> state::ChatMessage {
        let mut info = styrene_ipc::types::MessageInfo::default();
        info.id = id.into();
        info.source_hash = "peer".into();
        info.timestamp = timestamp;
        info.status = status.into();
        state::ChatMessage::from(info)
    }

    #[test]
    fn message_pages_and_live_events_merge_without_loss_or_duplication() {
        let generation = ConnectionGeneration(2);
        let mut stores = stores();
        let mut live = styrene_ipc::types::MessageInfo::default();
        live.id = "same".into();
        live.source_hash = "peer".into();
        live.timestamp = 10;
        live.status = "delivered".into();
        live.projection_complete = true;
        stores.apply_daemon_event(generation, DaemonEvent::MessageReceived(Box::new(live)));
        stores.merge_peer_message_page(
            generation,
            "peer",
            vec![chat("same", 10, "pending"), chat("older", 1, "")],
            Some("next".into()),
        );
        assert_eq!(stores.messages.messages.len(), 2);
        assert_eq!(stores.messages.messages[1].id, "same");
        assert_eq!(stores.messages.messages[1].status, "delivered");
        assert_eq!(stores.messages.message_cursors.get("peer").map(String::as_str), Some("next"));

        let mut update = styrene_ipc::types::MessageInfo::default();
        update.id = "older".into();
        update.source_hash = "peer".into();
        update.timestamp = -5;
        update.content = "complete live record".into();
        update.projection_complete = true;
        stores.apply_daemon_event(generation, DaemonEvent::MessageReceived(Box::new(update)));
        assert_eq!(stores.messages.messages[0].id, "older");
        assert_eq!(stores.messages.messages[0].timestamp, -5);
        assert_eq!(stores.messages.messages[0].content, "complete live record");
    }

    #[test]
    fn unknown_sparse_patch_requires_requery_and_page_remains_authoritative() {
        let generation = ConnectionGeneration(2);
        let mut stores = stores();
        stores.apply_daemon_event(generation, message_status("b", "delivered"));
        stores.merge_peer_message_page(
            generation,
            "peer",
            vec![chat("b", 7, "pending"), chat("a", 7, "")],
            None,
        );
        assert_eq!(
            stores.messages.messages.iter().map(|message| message.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(stores.messages.messages[1].status, "pending");
    }

    #[test]
    fn exact_statuses_survive_stale_normal_pages_without_discarding_page_fields() {
        let generation = ConnectionGeneration(2);
        for status in ["delivered", "cancelled", "failed: no route"] {
            let mut stores = stores();
            stores.merge_peer_message_page(
                generation,
                "peer",
                vec![chat("message", 1, "sending")],
                None,
            );
            stores.apply_daemon_event(generation, message_status("message", status));
            let mut stale = chat("message", 2, "sending");
            stale.content = "richer page content".into();
            stale.lifecycle.actual_method = Some("direct".into());
            stale.lifecycle.state = styrene_ipc::types::MessageLifecycleState::Failed;
            stale.lifecycle.terminal_detail = Some("page error metadata".into());
            stores.merge_peer_message_page(generation, "peer", vec![stale], None);
            let message = &stores.messages.messages[0];
            assert_eq!(message.status, "sending");
            assert_eq!(message.content, "richer page content");
            assert_eq!(message.timestamp, 2);
            assert_eq!(message.lifecycle.actual_method.as_deref(), Some("direct"));
            assert_eq!(message.lifecycle.state, styrene_ipc::types::MessageLifecycleState::Failed);
            assert_eq!(message.lifecycle.terminal_detail.as_deref(), Some("page error metadata"));

            let mut later_stale = chat("message", 3, "sending");
            later_stale.content = "later page content".into();
            stores.merge_peer_message_page(generation, "peer", vec![later_stale], None);
            assert_eq!(stores.messages.messages[0].status, "sending");
            assert_eq!(stores.messages.messages[0].content, "later page content");
        }
    }

    #[test]
    fn exact_statuses_survive_stale_reconciliation_pages() {
        let generation = ConnectionGeneration(2);
        for status in ["delivered", "cancelled", "failed: no route"] {
            let mut stores = stores();
            stores.apply_daemon_event(generation, DaemonEvent::EventGeneration(17));
            stores.merge_peer_message_page(
                generation,
                "peer",
                vec![chat("message", 1, "sending")],
                None,
            );
            stores.apply_daemon_event(generation, message_status("message", status));
            assert!(stores.apply_daemon_event(
                generation,
                DaemonEvent::ReconcileRequired { dropped: 1, connection_generation: 17 },
            ));

            let mut stale = chat("message", 3, "sending");
            stale.content = "reconciled page content".into();
            stale.lifecycle.correlation_id = Some("page-request".into());
            stores.merge_peer_message_page(generation, "peer", vec![stale], None);

            let message = &stores.messages.messages[0];
            assert_eq!(message.status, "sending");
            assert_eq!(message.content, "reconciled page content");
            assert_eq!(message.lifecycle.correlation_id.as_deref(), Some("page-request"));
        }
    }

    #[test]
    fn sparse_status_does_not_patch_known_record_before_authoritative_resolution() {
        let generation = ConnectionGeneration(2);
        let mut stores = stores();
        stores.merge_peer_message_page(
            generation,
            "peer",
            vec![chat("message", 1, "sending")],
            None,
        );
        stores.messages.messages[0].lifecycle.correlation_id = Some("stale-correlation".into());
        stores.messages.messages[0]
            .lifecycle
            .attempts
            .push(styrene_ipc::types::MessageAttemptInfo::default());
        stores.messages.messages[0]
            .lifecycle
            .evidence
            .push(styrene_ipc::types::MessageDeliveryEvidenceInfo::default());
        stores.messages.messages[0]
            .lifecycle
            .attachments
            .push(styrene_ipc::types::AttachmentInfo::default());
        stores.messages.messages[0]
            .lifecycle
            .propagation
            .push(styrene_ipc::types::MessagePropagationCorrelationInfo::default());
        for status in ["delivered", "failed: newer failure"] {
            stores.apply_daemon_event(generation, message_status("message", status));
        }
        assert_eq!(stores.messages.messages[0].status, "sending");

        let mut resolved = styrene_ipc::types::MessageInfo::default();
        resolved.id = "message".into();
        resolved.source_hash = "peer".into();
        resolved.timestamp = 2;
        resolved.status = "failed: newer failure".into();
        resolved.lifecycle_state = styrene_ipc::types::MessageLifecycleState::Failed;
        resolved.terminal_detail = Some("newer failure".into());
        assert!(stores.resolve_message(generation, "message", Some(resolved)));
        assert_eq!(stores.messages.messages[0].status, "failed: newer failure");
        assert_eq!(
            stores.messages.messages[0].lifecycle.terminal_detail.as_deref(),
            Some("newer failure")
        );
        assert!(stores.messages.messages[0].lifecycle.correlation_id.is_none());
        assert!(stores.messages.messages[0].lifecycle.attempts.is_empty());
        assert!(stores.messages.messages[0].lifecycle.evidence.is_empty());
        assert!(stores.messages.messages[0].lifecycle.attachments.is_empty());
        assert!(stores.messages.messages[0].lifecycle.propagation.is_empty());
    }

    #[test]
    fn reconciliation_recovers_dropped_new_and_status_without_losing_concurrent_live_records() {
        let generation = ConnectionGeneration(2);
        let mut stores = stores();
        stores.apply_daemon_event(generation, DaemonEvent::EventGeneration(17));
        stores.merge_peer_message_page(
            generation,
            "peer",
            vec![chat("known", 1, "sending")],
            Some("stale-cursor".into()),
        );
        assert!(stores.apply_daemon_event(
            generation,
            DaemonEvent::ReconcileRequired { dropped: 2, connection_generation: 17 },
        ));
        assert!(stores.messages.message_cursors.is_empty());
        assert_eq!(stores.loaded_message_peers(), vec!["peer"]);

        let mut concurrent = styrene_ipc::types::MessageInfo::default();
        concurrent.id = "concurrent".into();
        concurrent.source_hash = "peer".into();
        concurrent.timestamp = 3;
        concurrent.content = "arrived during reload".into();
        concurrent.status = "received".into();
        concurrent.projection_complete = true;
        stores.apply_daemon_event(generation, DaemonEvent::MessageReceived(Box::new(concurrent)));
        stores.merge_peer_message_page(
            generation,
            "peer",
            vec![chat("dropped-new", 2, "received"), chat("known", 1, "delivered")],
            None,
        );

        assert_eq!(stores.messages.messages.len(), 3);
        assert_eq!(
            stores.messages.messages.iter().find(|message| message.id == "known").unwrap().status,
            "delivered"
        );
        assert_eq!(
            stores
                .messages
                .messages
                .iter()
                .find(|message| message.id == "concurrent")
                .unwrap()
                .content,
            "arrived during reload"
        );
    }

    #[test]
    fn transient_message_tracking_is_bounded_and_unknown_sparse_patches_require_requery() {
        let generation = ConnectionGeneration(2);
        let mut stores = stores();
        stores.merge_peer_message_page(
            generation,
            "peer",
            vec![chat("retained", 1, "sending")],
            None,
        );
        stores.apply_daemon_event(generation, message_status("retained", "delivered"));
        for index in 0..(MESSAGE_TRANSIENT_CAPACITY + 100) {
            stores.apply_daemon_event(
                generation,
                message_status(&format!("pending-{index}"), &format!("failed: detail-{index}")),
            );
        }
        assert!(matches!(stores.messages.state, DataState::Degraded { .. }));
        stores.merge_peer_message_page(
            generation,
            "peer",
            vec![chat("retained", 2, "sending")],
            None,
        );
        assert_eq!(stores.messages.messages[0].status, "sending");

        for index in 0..(MESSAGE_TRANSIENT_CAPACITY + 100) {
            let mut message = styrene_ipc::types::MessageInfo::default();
            message.id = format!("live-{index}");
            message.source_hash = "peer".into();
            message.timestamp = index as i64;
            message.status = "sending".into();
            message.projection_complete = true;
            stores.apply_daemon_event(generation, DaemonEvent::MessageReceived(Box::new(message)));
        }
        assert!(stores.messages.live_message_ids.len() <= MESSAGE_TRANSIENT_CAPACITY);
        stores.apply_daemon_event(
            generation,
            message_status(&format!("live-{}", MESSAGE_TRANSIENT_CAPACITY + 99), "cancelled"),
        );
        assert_eq!(
            stores.messages.messages.last().map(|message| message.status.as_str()),
            Some("sending")
        );
        let id = format!("live-{}", MESSAGE_TRANSIENT_CAPACITY + 99);
        let mut resolved = styrene_ipc::types::MessageInfo::default();
        resolved.id = id.clone();
        resolved.source_hash = "peer".into();
        resolved.status = "cancelled".into();
        resolved.lifecycle_state = styrene_ipc::types::MessageLifecycleState::Cancelled;
        assert!(stores.resolve_message(generation, &id, Some(resolved)));
        assert_eq!(
            stores
                .messages
                .messages
                .iter()
                .find(|message| message.id == id)
                .map(|message| message.status.as_str()),
            Some("cancelled")
        );
    }

    #[test]
    fn fleet_failures_have_explicit_terminal_outcomes() {
        let mut stores = stores();
        for (error, expected) in [
            ("permission denied for fleet.exec", FleetJobState::Denied),
            ("operation unsupported", FleetJobState::Unsupported),
            ("request timed out", FleetJobState::TimedOut),
            ("remote command failed: exit=1", FleetJobState::Failed),
        ] {
            let id = stores
                .begin_fleet_job(
                    ConnectionGeneration(2),
                    "peer".into(),
                    FleetOperation::Execute { command: "false".into(), args: Vec::new() },
                )
                .unwrap();
            stores.finish_fleet_job(ConnectionGeneration(2), &id, Err(error.into()));
            assert_eq!(stores.fleet.jobs.last().unwrap().state, expected);
        }
    }

    #[test]
    fn propagation_snapshot_preserves_queue_and_unsupported_domains() {
        let mut stores = stores();
        let mut snapshot = styrene_ipc::types::PropagationSnapshot::default();
        snapshot.enabled = true;
        snapshot.queue_count = 1;
        snapshot.queue_size_bytes = 128;
        snapshot.peer_state_supported = false;
        snapshot.sync_state_supported = false;
        stores.set_propagation_snapshot(ConnectionGeneration(2), snapshot, false);
        assert!(stores.propagation.enabled);
        assert_eq!(stores.propagation.snapshot.as_ref().unwrap().queue_count, 1);
        assert!(matches!(stores.propagation.state, DataState::Ready));
    }

    #[test]
    fn propagation_pages_append_without_duplicate_queue_entries() {
        let mut stores = stores();
        let mut first = styrene_ipc::types::PropagationSnapshot::default();
        first.enabled = true;
        first.next_cursor = Some("1".into());
        let mut entry = styrene_ipc::types::PropagationQueueEntry::default();
        entry.id = "first".into();
        first.queue.push(entry.clone());
        stores.set_propagation_snapshot(ConnectionGeneration(2), first, false);

        let mut second = styrene_ipc::types::PropagationSnapshot::default();
        second.enabled = true;
        second.queue.extend([entry, {
            let mut item = styrene_ipc::types::PropagationQueueEntry::default();
            item.id = "second".into();
            item
        }]);
        stores.set_propagation_snapshot(ConnectionGeneration(2), second, true);

        assert_eq!(stores.propagation.snapshot.as_ref().unwrap().queue.len(), 2);
    }

    #[test]
    fn propagation_failure_is_degraded_without_erasing_last_snapshot() {
        let mut stores = stores();
        let mut snapshot = styrene_ipc::types::PropagationSnapshot::default();
        snapshot.enabled = true;
        stores.set_propagation_snapshot(ConnectionGeneration(2), snapshot, false);
        stores.fail_propagation(ConnectionGeneration(2), "permission denied");
        assert!(matches!(stores.propagation.state, DataState::Degraded { .. }));
        assert!(stores.propagation.snapshot.is_some());
    }

    #[test]
    fn standard_propagation_event_query_result_refreshes_state_and_errors_degrade_it() {
        let mut stores = stores();
        stores.runtime.server_generation = Some(11);
        stores.runtime.event_server_generation = Some(11);
        assert!(stores.apply_daemon_event(
            ConnectionGeneration(2),
            DaemonEvent::StandardPropagationChanged { connection_generation: 11 },
        ));
        let mut snapshot = styrene_ipc::types::StandardPropagationSnapshot::default();
        snapshot.version = styrene_ipc::types::STANDARD_PROPAGATION_SNAPSHOT_VERSION;
        snapshot.active = true;
        snapshot.observed_at = Some(42);
        snapshot.connection_generation = Some(11);
        stores.set_standard_propagation_snapshot(ConnectionGeneration(2), snapshot);
        assert_eq!(
            stores.propagation.standard_snapshot.as_ref().and_then(|value| value.observed_at),
            Some(42)
        );
        assert!(matches!(stores.propagation.standard_state, DataState::Ready));

        let mut unsupported = styrene_ipc::types::StandardPropagationSnapshot::default();
        unsupported.connection_generation = Some(11);
        unsupported.observed_at = Some(77);
        stores.set_standard_propagation_snapshot(ConnectionGeneration(2), unsupported);
        assert!(matches!(
            stores.propagation.standard_state,
            DataState::Degraded { ref reason } if reason.contains("unsupported")
        ));
        assert_eq!(
            stores.propagation.standard_snapshot.as_ref().and_then(|value| value.observed_at),
            Some(42)
        );

        stores.fail_standard_propagation(ConnectionGeneration(2), "query unavailable");
        assert!(matches!(
            stores.propagation.standard_state,
            DataState::Degraded { ref reason } if reason == "standard propagation refresh failed"
        ));
        assert!(stores.propagation.standard_snapshot.is_some());

        let mut stale = styrene_ipc::types::StandardPropagationSnapshot::default();
        stale.version = styrene_ipc::types::STANDARD_PROPAGATION_SNAPSHOT_VERSION;
        stale.connection_generation = Some(10);
        stale.observed_at = Some(99);
        stores.set_standard_propagation_snapshot(ConnectionGeneration(2), stale);
        assert_eq!(
            stores.propagation.standard_snapshot.as_ref().and_then(|value| value.observed_at),
            Some(42)
        );
    }

    #[test]
    fn mutation_capabilities_fail_closed_and_are_generation_scoped() {
        let mut stores = stores();
        assert!(stores.mutation_availability("network.probe").unwrap_err().contains("unknown"));

        let mut active = styrene_ipc::types::ActiveCapabilitiesInfo::default();
        active.version = styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION;
        active.authorized_operations = vec!["network.announce".into()];
        let mut status = styrene_ipc::types::DaemonStatusInfo::default();
        status.connection_generation = Some(11);
        status.active_capabilities = Some(active);
        stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::Status(status));
        assert!(stores.mutation_availability("network.announce").is_ok());
        assert!(stores.mutation_availability("network.probe").unwrap_err().contains("denied"));

        let mut degraded = styrene_ipc::types::DegradedCapabilityInfo::default();
        degraded.id = "network.announce".into();
        degraded.reason = "transport worker unavailable".into();
        stores.runtime.capabilities.as_mut().unwrap().degraded.push(degraded);
        assert!(
            stores
                .mutation_availability("network.announce")
                .unwrap_err()
                .contains("transport worker unavailable")
        );

        stores.apply_daemon_event(
            ConnectionGeneration(2),
            DaemonEvent::Disconnected("closed".into()),
        );
        assert!(
            stores.mutation_availability("network.announce").unwrap_err().contains("disconnected")
        );
        stores.begin_session("new", ConnectionGeneration(3));
        assert!(stores.runtime.capabilities.is_none());
    }

    #[test]
    fn operator_fixture_catalog_exercises_each_universal_execution_gate() {
        use styrene_ipc::operator_fixtures::{
            OPERATOR_FIXTURE_OPERATIONS, OperatorFixtureState, operator_fixture_evidence,
        };

        for operation in OPERATOR_FIXTURE_OPERATIONS {
            let disconnected = DomainStores::default();
            assert!(
                disconnected
                    .mutation_availability(operation.capability)
                    .unwrap_err()
                    .contains("disconnected"),
                "{} disconnected fixture",
                operation.id
            );

            let mut current = stores();
            let mut active = styrene_ipc::types::ActiveCapabilitiesInfo::default();
            active.version = styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION;
            active.authorized_operations = vec![operation.capability.into()];
            let mut status = styrene_ipc::types::DaemonStatusInfo::default();
            status.connection_generation = Some(11);
            status.active_capabilities = Some(active);
            current.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::Status(status));
            assert!(
                current
                    .mutation_availability_at(ConnectionGeneration(1), operation.capability)
                    .unwrap_err()
                    .contains("stale"),
                "{} stale fixture",
                operation.id
            );

            current.runtime.capabilities.as_mut().unwrap().authorized_operations.clear();
            assert!(
                current
                    .mutation_availability(operation.capability)
                    .unwrap_err()
                    .contains("permission denied"),
                "{} denied fixture",
                operation.id
            );

            current
                .runtime
                .capabilities
                .as_mut()
                .unwrap()
                .authorized_operations
                .push(operation.capability.into());
            let mut degraded = styrene_ipc::types::DegradedCapabilityInfo::default();
            degraded.id = operation.capability.into();
            degraded.reason = "fixture: operation unsupported".into();
            current.runtime.capabilities.as_mut().unwrap().degraded.push(degraded);
            assert!(
                current
                    .mutation_availability(operation.capability)
                    .unwrap_err()
                    .contains("operation unsupported"),
                "{} unsupported fixture",
                operation.id
            );

            for state in [
                OperatorFixtureState::TimedOut,
                OperatorFixtureState::Cancelled,
                OperatorFixtureState::PartialFailure,
            ] {
                match operator_fixture_evidence(*operation, state) {
                    Some(evidence) => {
                        assert_eq!(evidence.source, styrene_ipc::types::ObservationSource::Fixture);
                        assert_eq!(evidence.connection_generation, 7);
                        assert!(evidence.terminal_outcome.is_some());
                        assert!(evidence.correlation_id.starts_with("fixture:"));
                    }
                    None => assert!(operation.not_applicable_reason(state).is_some()),
                }
            }
        }
    }

    #[test]
    fn execution_gate_requires_exact_capability_and_current_frontend_generation() {
        let mut stores = stores();
        let mut active = styrene_ipc::types::ActiveCapabilitiesInfo::default();
        active.version = styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION;
        active.authorized_operations = vec!["page.browse".into()];
        let mut status = styrene_ipc::types::DaemonStatusInfo::default();
        status.connection_generation = Some(11);
        status.active_capabilities = Some(active);
        stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::Status(status));

        assert!(stores.mutation_availability_at(ConnectionGeneration(2), "page.browse").is_ok());
        assert!(
            stores
                .mutation_availability_at(ConnectionGeneration(1), "page.browse")
                .unwrap_err()
                .contains("stale")
        );
        assert!(
            stores
                .mutation_availability_at(ConnectionGeneration(2), "rpc.status")
                .unwrap_err()
                .contains("denied")
        );
    }

    #[test]
    fn terminal_scenario_results_only_update_the_active_run() {
        let mut stores = stores();
        stores.set_scenario_run(ScenarioRun {
            run_id: "active-run".into(),
            scenario_id: "direct",
            status: crate::scenario::ScenarioStatus::Running,
            milestones: Vec::new(),
            evidence: Vec::new(),
            runner_evidence: None,
        });
        stores.update_scenario_run(ScenarioRun {
            run_id: "stale-run".into(),
            scenario_id: "direct",
            status: crate::scenario::ScenarioStatus::Passed,
            milestones: Vec::new(),
            evidence: Vec::new(),
            runner_evidence: None,
        });
        assert_eq!(
            stores.scenario.run.as_ref().map(|run| run.status),
            Some(crate::scenario::ScenarioStatus::Running)
        );

        stores.update_scenario_run(ScenarioRun {
            run_id: "active-run".into(),
            scenario_id: "direct",
            status: crate::scenario::ScenarioStatus::TimedOut,
            milestones: Vec::new(),
            evidence: vec!["failure:fixture timeout".into()],
            runner_evidence: None,
        });
        assert_eq!(
            stores.scenario.run.as_ref().map(|run| run.status),
            Some(crate::scenario::ScenarioStatus::TimedOut)
        );
    }

    #[test]
    fn authoritative_timeout_and_cancellation_outcomes_are_preserved() {
        let mut stores = stores();
        stores.runtime.server_generation = Some(9);
        for (id, outcome) in [
            ("timeout", styrene_ipc::types::NetworkOperationOutcome::TimedOut),
            ("cancelled", styrene_ipc::types::NetworkOperationOutcome::Cancelled),
        ] {
            let mut operation = styrene_ipc::types::NetworkOperationInfo::default();
            operation.operation_id = id.into();
            operation.kind = styrene_ipc::types::NetworkOperationKind::Probe;
            operation.outcome = Some(outcome);
            operation.observation.connection_generation = Some(9);
            stores.apply_daemon_event(
                ConnectionGeneration(2),
                DaemonEvent::NetworkOperation(operation),
            );
        }
        assert_eq!(
            stores.network.operations[0].outcome,
            Some(styrene_ipc::types::NetworkOperationOutcome::TimedOut)
        );
        assert_eq!(
            stores.network.operations[1].outcome,
            Some(styrene_ipc::types::NetworkOperationOutcome::Cancelled)
        );
    }

    #[test]
    fn narrow_layout_prevents_clipping_and_stacks_graph_inspector_after_content() {
        let css = include_str!("assets/style.css");
        let graph = include_str!("components/network_graph.rs");
        let network_page = include_str!("components/network_page.rs");
        let fleet_page = include_str!("components/fleet_page.rs");
        let page_browser = include_str!("components/page_browser.rs");
        let app = include_str!("main.rs");
        let narrow = css
            .rsplit_once("@media (max-width: 720px)")
            .map(|(_, rules)| rules)
            .expect("narrow network breakpoint");

        assert!(css.contains("@media (max-width: 520px)"));
        assert!(css.contains(".body > .sidebar"));
        assert!(css.contains("display: none"));
        assert!(css.contains(".network-filter-bar input:first-child"));
        assert!(
            narrow.contains(
                ".network-page {\n    min-height: 0;\n    overflow-x: hidden;\n    overflow-y: auto;\n  }"
            )
        );
        assert!(narrow.contains(
            ".network-view { flex: 0 0 auto; flex-direction: column; min-height: 0; height: auto; overflow: visible; }"
        ));
        assert!(
            narrow.contains(".graph-container { flex: 0 0 auto; width: 100%; min-height: 360px; }")
        );
        assert!(
            narrow.contains(".graph-sidebar {\n    order: 2;\n    width: 100%;\n    min-width: 0;")
        );
        assert!(narrow.contains("border-left: 0;\n    overflow: visible;"));
        assert!(css.contains("overflow-wrap: anywhere;"));
        assert!(css.contains("word-break: break-word;"));
        assert!(network_page.contains("aria_label: format!(\"Cancel request {}\""));
        assert!(network_page.contains("aria_label: format!(\"Cancel resource {}\""));
        for source in [network_page, fleet_page, page_browser, app] {
            assert!(source.contains("aria_describedby"));
            assert!(source.contains("control-disabled-reason"));
        }
        assert!(
            graph.find("class: \"graph-container\"").expect("graph content")
                < graph.find("NetworkInspector {").expect("graph inspector")
        );
    }

    #[test]
    fn same_session_generation_changes_are_rejected() {
        let mut stores = stores();
        let mut status = styrene_ipc::types::DaemonStatusInfo::default();
        status.connection_generation = Some(9);
        status.active_capabilities = Some(Default::default());
        assert!(stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::Status(status)));

        let mut mismatched = styrene_ipc::types::DaemonStatusInfo::default();
        mismatched.connection_generation = Some(10);
        mismatched.active_capabilities = Some(Default::default());
        assert!(
            !stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::Status(mismatched))
        );
        assert!(stores.runtime.server_generation.is_none());
        assert!(stores.runtime.capabilities.is_none());

        assert!(
            stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::EventGeneration(11))
        );
        assert!(!stores.apply_daemon_event(
            ConnectionGeneration(2),
            DaemonEvent::ReconcileRequests { dropped: 1, connection_generation: 10 }
        ));
        assert!(
            !stores.apply_daemon_event(ConnectionGeneration(2), DaemonEvent::EventGeneration(12))
        );
        assert!(stores.runtime.event_server_generation.is_none());
    }

    #[test]
    fn typed_observations_without_a_negotiated_generation_are_rejected() {
        let mut stores = stores();
        let mut operation = styrene_ipc::types::NetworkOperationInfo::default();
        operation.operation_id = "operation".into();

        assert!(
            !stores.apply_daemon_event(
                ConnectionGeneration(2),
                DaemonEvent::NetworkOperation(operation)
            )
        );
        assert!(stores.network.operations.is_empty());
    }

    #[test]
    fn paper_send_installs_exact_uri_before_accepting_compose() {
        let mut stores = stores();
        let mut outcome = styrene_ipc::types::SendChatOutcome::default();
        outcome.disposition = styrene_ipc::types::SendChatDisposition::PaperExported;
        outcome.message_id = "paper-id".into();
        outcome.message.id = outcome.message_id.clone();
        outcome.message.destination_hash = "peer".into();
        outcome.message.is_outgoing = true;
        outcome.paper_uri = Some("lxm://exact-dx-paper".into());

        assert!(stores.apply_send_outcome(
            ConnectionGeneration(2),
            "peer".into(),
            "compose".into(),
            outcome,
        ));
        let export = stores.messages.paper_export.as_ref().unwrap();
        assert_eq!(export.uri, "lxm://exact-dx-paper");
        assert_eq!(stores.messages.accepted_compose, Some(("peer".into(), "compose".into())));
        assert!(!format!("{export:?}").contains("exact-dx-paper"));

        stores.messages.accepted_compose = None;
        let mut missing = styrene_ipc::types::SendChatOutcome::default();
        missing.disposition = styrene_ipc::types::SendChatDisposition::PaperExported;
        missing.message_id = "missing-paper-id".into();
        missing.message.id = missing.message_id.clone();
        missing.message.destination_hash = "peer".into();
        missing.message.is_outgoing = true;
        assert!(!stores.apply_send_outcome(
            ConnectionGeneration(2),
            "peer".into(),
            "retain".into(),
            missing,
        ));
        assert!(stores.messages.accepted_compose.is_none());
    }

    #[test]
    fn every_lifecycle_disposition_merges_or_removes_authoritative_projection() {
        let mut stores = stores();
        let message = |status: &str| {
            let mut message = styrene_ipc::types::MessageInfo::default();
            message.id = "message".into();
            message.destination_hash = "peer".into();
            message.content = "body".into();
            message.status = status.into();
            message.is_outgoing = true;
            message
        };
        stores.push_outgoing(ConnectionGeneration(2), state::ChatMessage::from(message("sending")));
        for disposition in [
            styrene_ipc::types::MessagingDisposition::Applied,
            styrene_ipc::types::MessagingDisposition::Unchanged,
            styrene_ipc::types::MessagingDisposition::AlreadyCancelled,
            styrene_ipc::types::MessagingDisposition::TerminalConflict,
        ] {
            let mut outcome = styrene_ipc::types::MessagingOperationOutcome::default();
            outcome.disposition = disposition;
            outcome.target_id = "message".into();
            outcome.correlated_id = (disposition
                == styrene_ipc::types::MessagingDisposition::Unchanged)
                .then(|| "message".into());
            outcome.terminal_state = (disposition
                == styrene_ipc::types::MessagingDisposition::TerminalConflict)
                .then(|| "delivered".into());
            outcome.message = (disposition
                != styrene_ipc::types::MessagingDisposition::TerminalConflict)
                .then(|| message("delivered"));
            assert!(stores.apply_lifecycle_outcome(ConnectionGeneration(2), outcome).is_none());
            assert_eq!(stores.messages.messages[0].status, "delivered");
        }

        let mut missing = styrene_ipc::types::MessagingOperationOutcome::default();
        missing.disposition = styrene_ipc::types::MessagingDisposition::NotFound;
        missing.target_id = "message".into();
        assert_eq!(
            stores.apply_lifecycle_outcome(ConnectionGeneration(2), missing).as_deref(),
            Some("peer")
        );
        assert!(stores.messages.messages.is_empty());
    }

    #[test]
    fn untrusted_daemon_diagnostics_are_redacted_before_ui_state() {
        let mut disconnected_stores = stores();
        assert!(disconnected_stores.apply_daemon_event(
            ConnectionGeneration(2),
            DaemonEvent::Disconnected("token=structured-secret /Users/operator/key".into()),
        ));
        let rendered =
            format!("{:?}{:?}", disconnected_stores.runtime, disconnected_stores.activity.entries);
        assert!(!rendered.contains("structured-secret"));
        assert!(!rendered.contains("/Users/operator"));

        let mut stores = stores();
        stores.runtime.server_generation = Some(9);
        let mut operation = styrene_ipc::types::NetworkOperationInfo::default();
        operation.operation_id = "operation".into();
        operation.detail = Some("password=structured-secret /private/key".into());
        operation.observation.connection_generation = Some(9);
        assert!(
            stores.apply_daemon_event(
                ConnectionGeneration(2),
                DaemonEvent::NetworkOperation(operation),
            )
        );
        assert_eq!(
            stores.network.operations[0].detail.as_deref(),
            Some("daemon operation detail redacted")
        );
    }

    #[test]
    fn fleet_audit_state_never_retains_commands_arguments_or_output() {
        let mut stores = stores();
        let id = stores
            .begin_fleet_job(
                ConnectionGeneration(2),
                "peer".into(),
                FleetOperation::Execute {
                    command: "print structured-secret".into(),
                    args: vec!["--token=structured-secret".into()],
                },
            )
            .expect("current Fleet job");
        stores.finish_fleet_job(
            ConnectionGeneration(2),
            &id,
            Ok("stdout=structured-secret /private/key".into()),
        );
        let rendered = format!("{:?}", stores.fleet.jobs);
        assert!(!rendered.contains("structured-secret"));
        assert!(!rendered.contains("/private/key"));
        assert!(rendered.contains("[REDACTED]"));
    }
}
