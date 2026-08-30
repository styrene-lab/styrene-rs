use crate::storage::messages::{
    AttemptRouteObservationRecord, MessageRecord, MessagesStore, OutboundAttemptRecord,
    OutboundRouteRecord,
};
use rns_core::packet::LXMF_MAX_PAYLOAD;
use rns_core::transport::resource::LINK_PACKET_MDU;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_OUTBOUND_MESSAGES: usize = 4096;
const MAX_ATTEMPTS_PER_MESSAGE: usize = 32;
const DIRECT_DEADLINE: Duration = Duration::from_secs(32);
const OPPORTUNISTIC_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMethod {
    Direct,
    Opportunistic,
    Propagated,
    Paper,
}

impl DeliveryMethod {
    fn parse(value: Option<&str>) -> Result<Self, std::io::Error> {
        match value.unwrap_or("direct").trim().to_ascii_lowercase().as_str() {
            "direct" => Ok(Self::Direct),
            "opportunistic" => Ok(Self::Opportunistic),
            "propagated" => Ok(Self::Propagated),
            "paper" => Ok(Self::Paper),
            other => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("unsupported LXMF delivery method: {other}"),
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Opportunistic => "opportunistic",
            Self::Propagated => "propagated",
            Self::Paper => "paper",
        }
    }

    fn from_persisted(value: &str) -> Result<Self, std::io::Error> {
        Self::parse(Some(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireRepresentation {
    Packet,
    Resource,
    Paper,
}

impl WireRepresentation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Packet => "packet",
            Self::Resource => "resource",
            Self::Paper => "paper",
        }
    }

    fn from_persisted(value: &str) -> Result<Self, std::io::Error> {
        match value {
            "packet" => Ok(Self::Packet),
            "resource" => Ok(Self::Resource),
            "paper" => Ok(Self::Paper),
            _ => Err(std::io::Error::other("invalid persisted LXMF representation")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundState {
    Queued,
    Sending,
    Sent,
    Delivered,
    Failed,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleEvidence {
    PacketDeliveryReceipt,
    ResourceDeliveryComplete,
    Cancelled,
    Expired,
    Failed(String),
}

impl OutboundState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Delivered | Self::Failed | Self::Cancelled | Self::Expired)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Sending => "sending",
            Self::Sent => "sent",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    fn from_persisted(value: &str) -> Result<Self, std::io::Error> {
        match value {
            "queued" => Ok(Self::Queued),
            "sending" => Ok(Self::Sending),
            "sent" => Ok(Self::Sent),
            "delivered" => Ok(Self::Delivered),
            "failed" | "rejected" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "expired" => Ok(Self::Expired),
            _ => Err(std::io::Error::other("invalid persisted LXMF router state")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutboundAttempt {
    pub number: u32,
    pub started_at: Instant,
    pub deadline: Instant,
}

#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub message_id: String,
    pub requested_method: DeliveryMethod,
    pub actual_method: DeliveryMethod,
    pub representation: WireRepresentation,
    pub fallback_reason: Option<String>,
    pub correlation_id: String,
    pub attempts: Vec<OutboundAttempt>,
    pub total_attempts: usize,
    pub deadline: Instant,
    pub deadline_unix_ms: i64,
    pub state: OutboundState,
}

#[derive(Debug, Clone)]
pub struct DeliveryPlan {
    pub requested_method: DeliveryMethod,
    pub actual_method: DeliveryMethod,
    pub representation: WireRepresentation,
    pub fallback_reason: Option<String>,
    pub correlation_id: String,
    pub deadline: Instant,
    pub deadline_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub enum RetryQueueResult {
    Queued(DeliveryPlan),
    Existing(OutboundRouteRecord),
}

#[derive(Debug, Clone)]
pub enum RetryStartResult {
    Started(DeliveryPlan),
    Existing(OutboundRouteRecord),
    MissingCanonicalWire,
}

pub trait RouterClock: Send + Sync {
    fn monotonic_now(&self) -> Instant;
    fn unix_time_ms(&self) -> i64;
}

#[derive(Default)]
pub struct SystemRouterClock;

impl RouterClock for SystemRouterClock {
    fn monotonic_now(&self) -> Instant {
        Instant::now()
    }

    fn unix_time_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or(0)
    }
}

#[derive(Default)]
struct CoordinatorState {
    messages: HashMap<String, OutboundMessage>,
    insertion_order: VecDeque<String>,
}

pub struct RouterCoordinator {
    store: Arc<Mutex<MessagesStore>>,
    clock: Arc<dyn RouterClock>,
    state: Mutex<CoordinatorState>,
    initialization_error: Mutex<Option<String>>,
}

impl RouterCoordinator {
    fn lock_store(&self) -> Result<std::sync::MutexGuard<'_, MessagesStore>, std::io::Error> {
        self.store.lock().map_err(|_| std::io::Error::other("LXMF router store lock poisoned"))
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, CoordinatorState>, std::io::Error> {
        self.state.lock().map_err(|_| std::io::Error::other("LXMF router state lock poisoned"))
    }

    pub fn new(store: Arc<Mutex<MessagesStore>>) -> Self {
        Self::with_clock(store, Arc::new(SystemRouterClock))
    }

    pub fn with_clock(store: Arc<Mutex<MessagesStore>>, clock: Arc<dyn RouterClock>) -> Self {
        let coordinator = Self {
            store,
            clock,
            state: Mutex::new(CoordinatorState::default()),
            initialization_error: Mutex::new(None),
        };
        if let Err(error) = coordinator.reconcile_startup() {
            coordinator.record_initialization_error(error.to_string());
        }
        coordinator
    }

    pub fn record_initialization_error(&self, error: impl Into<String>) {
        if let Ok(mut initialization_error) = self.initialization_error.lock()
            && initialization_error.is_none()
        {
            *initialization_error = Some(error.into());
        }
    }

    fn ensure_ready(&self) -> Result<(), std::io::Error> {
        let initialization_error = self
            .initialization_error
            .lock()
            .map_err(|_| std::io::Error::other("LXMF router initialization lock poisoned"))?;
        match initialization_error.as_deref() {
            Some(error) => {
                Err(std::io::Error::other(format!("LXMF router initialization failed: {error}")))
            }
            None => Ok(()),
        }
    }

    fn reload_state_locked(&self, state: &mut CoordinatorState) -> Result<(), std::io::Error> {
        let now_unix_ms = self.clock.unix_time_ms();
        let now = self.clock.monotonic_now();
        let store = self.lock_store()?;
        let routes = store.outbound_routes().map_err(std::io::Error::other)?;
        let mut messages = HashMap::new();
        let mut insertion_order = VecDeque::new();
        for route in routes
            .into_iter()
            .rev()
            .take(MAX_OUTBOUND_MESSAGES)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            let persisted_attempts =
                store.outbound_attempts(&route.message_id).map_err(std::io::Error::other)?;
            let total_attempts = persisted_attempts.len();
            let deadline = if route.deadline_unix_ms <= now_unix_ms {
                now
            } else {
                now + Duration::from_millis(
                    u64::try_from(route.deadline_unix_ms - now_unix_ms).unwrap_or(u64::MAX),
                )
            };
            let attempts = persisted_attempts
                .into_iter()
                .rev()
                .take(MAX_ATTEMPTS_PER_MESSAGE)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|attempt| {
                    let elapsed_ms = now_unix_ms.saturating_sub(attempt.started_unix_ms);
                    OutboundAttempt {
                        number: attempt.attempt_number,
                        started_at: now
                            .checked_sub(Duration::from_millis(
                                u64::try_from(elapsed_ms).unwrap_or(u64::MAX),
                            ))
                            .unwrap_or(now),
                        deadline,
                    }
                })
                .collect();
            let message = OutboundMessage {
                message_id: route.message_id.clone(),
                requested_method: DeliveryMethod::from_persisted(&route.requested_method)?,
                actual_method: DeliveryMethod::from_persisted(&route.actual_method)?,
                representation: WireRepresentation::from_persisted(&route.representation)?,
                fallback_reason: route.fallback_reason,
                correlation_id: route.correlation_id,
                attempts,
                total_attempts,
                deadline,
                deadline_unix_ms: route.deadline_unix_ms,
                state: OutboundState::from_persisted(&route.state)?,
            };
            insertion_order.push_back(message.message_id.clone());
            messages.insert(message.message_id.clone(), message);
        }
        state.messages = messages;
        state.insertion_order = insertion_order;
        Ok(())
    }

    pub fn reconcile_startup(&self) -> Result<(), std::io::Error> {
        self.lock_store()?
            .reconcile_outbound_startup(self.clock.unix_time_ms())
            .map_err(std::io::Error::other)?;
        let mut state = self.lock_state()?;
        self.reload_state_locked(&mut state)
    }

    pub fn reconcile_deadlines(&self) -> Result<Vec<String>, std::io::Error> {
        self.ensure_ready()?;
        let expired = self
            .lock_store()?
            .expire_outbound_routes(self.clock.unix_time_ms())
            .map_err(std::io::Error::other)?;
        let mut state = self.lock_state()?;
        self.reload_state_locked(&mut state)?;
        Ok(expired)
    }

    pub fn due_resource_evidence(&self) -> Result<Vec<(String, String)>, std::io::Error> {
        self.ensure_ready()?;
        self.lock_store()?
            .due_outbound_resource_evidence(self.clock.unix_time_ms())
            .map_err(std::io::Error::other)
    }

    pub fn queue(
        &self,
        message: &MessageRecord,
        requested_method: Option<&str>,
        encoded_wire_size: usize,
        opportunistic_wire_size: usize,
        correlation_id: Option<&str>,
    ) -> Result<DeliveryPlan, std::io::Error> {
        self.queue_with_retry(
            message,
            requested_method,
            encoded_wire_size,
            opportunistic_wire_size,
            correlation_id,
            None,
            None,
            &[],
            None,
            None,
        )
    }

    pub fn queue_with_ticket_offer(
        &self,
        message: &MessageRecord,
        requested_method: Option<&str>,
        encoded_wire_size: usize,
        opportunistic_wire_size: usize,
        correlation_id: Option<&str>,
        ticket: Option<&crate::storage::messages::LxmfTicketOfferReservation>,
    ) -> Result<DeliveryPlan, std::io::Error> {
        self.queue_with_retry(
            message,
            requested_method,
            encoded_wire_size,
            opportunistic_wire_size,
            correlation_id,
            None,
            ticket,
            &[],
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn queue_with_ticket_offer_and_attachments(
        &self,
        message: &MessageRecord,
        requested_method: Option<&str>,
        encoded_wire_size: usize,
        opportunistic_wire_size: usize,
        correlation_id: Option<&str>,
        ticket: Option<&crate::storage::messages::LxmfTicketOfferReservation>,
        attachments: &[crate::storage::messages::AttachmentBlobInput],
        canonical_wire: &[u8],
    ) -> Result<DeliveryPlan, std::io::Error> {
        self.queue_with_retry(
            message,
            requested_method,
            encoded_wire_size,
            opportunistic_wire_size,
            correlation_id,
            None,
            ticket,
            attachments,
            None,
            Some(canonical_wire),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn queue_propagated_with_ticket_offer_and_attachments(
        &self,
        message: &MessageRecord,
        requested_method: Option<&str>,
        encoded_wire_size: usize,
        opportunistic_wire_size: usize,
        correlation_id: Option<&str>,
        ticket: Option<&crate::storage::messages::LxmfTicketOfferReservation>,
        attachments: &[crate::storage::messages::AttachmentBlobInput],
        propagation: &crate::storage::standard_propagation::StandardPropagationClientJob,
        canonical_wire: &[u8],
    ) -> Result<DeliveryPlan, std::io::Error> {
        self.queue_with_retry(
            message,
            requested_method,
            encoded_wire_size,
            opportunistic_wire_size,
            correlation_id,
            None,
            ticket,
            attachments,
            Some(propagation),
            Some(canonical_wire),
        )
    }

    pub fn queue_retry(
        &self,
        message: &MessageRecord,
        requested_method: Option<&str>,
        encoded_wire_size: usize,
        opportunistic_wire_size: usize,
        correlation_id: &str,
        retry_of: &str,
    ) -> Result<RetryQueueResult, std::io::Error> {
        match self.queue_with_retry(
            message,
            requested_method,
            encoded_wire_size,
            opportunistic_wire_size,
            Some(correlation_id),
            Some(retry_of),
            None,
            &[],
            None,
            None,
        ) {
            Ok(plan) => Ok(RetryQueueResult::Queued(plan)),
            Err(error) => {
                let winner = {
                    self.lock_store()?
                        .outbound_retry_for(retry_of)
                        .map_err(std::io::Error::other)?
                };
                if let Some(winner) = winner {
                    let mut state = self.lock_state()?;
                    self.reload_state_locked(&mut state)?;
                    Ok(RetryQueueResult::Existing(winner))
                } else {
                    Err(error)
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn queue_retry_with_ticket_offer(
        &self,
        message: &MessageRecord,
        requested_method: Option<&str>,
        encoded_wire_size: usize,
        opportunistic_wire_size: usize,
        correlation_id: &str,
        retry_of: &str,
        ticket: Option<&crate::storage::messages::LxmfTicketOfferReservation>,
    ) -> Result<RetryQueueResult, std::io::Error> {
        match self.queue_with_retry(
            message,
            requested_method,
            encoded_wire_size,
            opportunistic_wire_size,
            Some(correlation_id),
            Some(retry_of),
            ticket,
            &[],
            None,
            None,
        ) {
            Ok(plan) => Ok(RetryQueueResult::Queued(plan)),
            Err(error) => {
                let winner = self
                    .lock_store()?
                    .outbound_retry_for(retry_of)
                    .map_err(std::io::Error::other)?;
                if let Some(winner) = winner {
                    let mut state = self.lock_state()?;
                    self.reload_state_locked(&mut state)?;
                    Ok(RetryQueueResult::Existing(winner))
                } else {
                    Err(error)
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn queue_retry_with_ticket_offer_and_attachments(
        &self,
        message: &MessageRecord,
        requested_method: Option<&str>,
        encoded_wire_size: usize,
        opportunistic_wire_size: usize,
        correlation_id: &str,
        retry_of: &str,
        ticket: Option<&crate::storage::messages::LxmfTicketOfferReservation>,
        attachments: &[crate::storage::messages::AttachmentBlobInput],
    ) -> Result<RetryQueueResult, std::io::Error> {
        match self.queue_with_retry(
            message,
            requested_method,
            encoded_wire_size,
            opportunistic_wire_size,
            Some(correlation_id),
            Some(retry_of),
            ticket,
            attachments,
            None,
            None,
        ) {
            Ok(plan) => Ok(RetryQueueResult::Queued(plan)),
            Err(error) => {
                let winner = self
                    .lock_store()?
                    .outbound_retry_for(retry_of)
                    .map_err(std::io::Error::other)?;
                if let Some(winner) = winner {
                    let mut state = self.lock_state()?;
                    self.reload_state_locked(&mut state)?;
                    Ok(RetryQueueResult::Existing(winner))
                } else {
                    Err(error)
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn queue_retry_propagated_with_ticket_offer_and_attachments(
        &self,
        message: &MessageRecord,
        requested_method: Option<&str>,
        encoded_wire_size: usize,
        opportunistic_wire_size: usize,
        correlation_id: &str,
        retry_of: &str,
        ticket: Option<&crate::storage::messages::LxmfTicketOfferReservation>,
        attachments: &[crate::storage::messages::AttachmentBlobInput],
        propagation: &crate::storage::standard_propagation::StandardPropagationClientJob,
    ) -> Result<RetryQueueResult, std::io::Error> {
        match self.queue_with_retry(
            message,
            requested_method,
            encoded_wire_size,
            opportunistic_wire_size,
            Some(correlation_id),
            Some(retry_of),
            ticket,
            attachments,
            Some(propagation),
            None,
        ) {
            Ok(plan) => Ok(RetryQueueResult::Queued(plan)),
            Err(error) => {
                let winner = self
                    .lock_store()?
                    .outbound_retry_for(retry_of)
                    .map_err(std::io::Error::other)?;
                if let Some(winner) = winner {
                    let mut state = self.lock_state()?;
                    self.reload_state_locked(&mut state)?;
                    Ok(RetryQueueResult::Existing(winner))
                } else {
                    Err(error)
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn queue_with_retry(
        &self,
        message: &MessageRecord,
        requested_method: Option<&str>,
        encoded_wire_size: usize,
        opportunistic_wire_size: usize,
        correlation_id: Option<&str>,
        retry_of: Option<&str>,
        ticket: Option<&crate::storage::messages::LxmfTicketOfferReservation>,
        attachments: &[crate::storage::messages::AttachmentBlobInput],
        propagation: Option<&crate::storage::standard_propagation::StandardPropagationClientJob>,
        canonical_wire: Option<&[u8]>,
    ) -> Result<DeliveryPlan, std::io::Error> {
        self.ensure_ready()?;
        let requested_method = DeliveryMethod::parse(requested_method)?;
        let (actual_method, fallback_reason) = if requested_method == DeliveryMethod::Opportunistic
            && opportunistic_wire_size > LXMF_MAX_PAYLOAD
        {
            (
                DeliveryMethod::Direct,
                Some(format!(
                    "encoded opportunistic payload is {opportunistic_wire_size} bytes; packet limit is {LXMF_MAX_PAYLOAD}"
                )),
            )
        } else {
            (requested_method, None)
        };
        let representation = match actual_method {
            DeliveryMethod::Opportunistic => WireRepresentation::Packet,
            DeliveryMethod::Direct | DeliveryMethod::Propagated => {
                if encoded_wire_size <= LINK_PACKET_MDU {
                    WireRepresentation::Packet
                } else {
                    WireRepresentation::Resource
                }
            }
            DeliveryMethod::Paper => WireRepresentation::Paper,
        };
        let duration = if actual_method == DeliveryMethod::Opportunistic {
            OPPORTUNISTIC_DEADLINE
        } else {
            DIRECT_DEADLINE
        };
        let now = self.clock.monotonic_now();
        let deadline = now + duration;
        let deadline_unix_ms = self
            .clock
            .unix_time_ms()
            .saturating_add(i64::try_from(duration.as_millis()).unwrap_or(i64::MAX));
        let correlation_id = correlation_id.unwrap_or(&message.id).to_string();
        let plan = DeliveryPlan {
            requested_method,
            actual_method,
            representation,
            fallback_reason,
            correlation_id,
            deadline,
            deadline_unix_ms,
        };

        let mut state = self.lock_state()?;
        let correlated_attempts = self
            .lock_store()?
            .outbound_attempts_for_correlation(&plan.correlation_id)
            .map_err(std::io::Error::other)?
            .len();
        if correlated_attempts >= MAX_ATTEMPTS_PER_MESSAGE {
            return Err(std::io::Error::other("outbound LXMF attempt limit reached"));
        }
        if state.messages.contains_key(&message.id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "outbound LXMF message is already queued",
            ));
        }
        while state.messages.len() >= MAX_OUTBOUND_MESSAGES {
            let Some(oldest) = state.insertion_order.pop_front() else {
                break;
            };
            let terminal =
                state.messages.get(&oldest).is_some_and(|message| message.state.is_terminal());
            if terminal {
                state.messages.remove(&oldest);
            } else {
                state.insertion_order.push_front(oldest);
                return Err(std::io::Error::other("outbound LXMF router queue is full"));
            }
        }

        let route = OutboundRouteRecord {
            message_id: message.id.clone(),
            requested_method: requested_method.as_str().into(),
            actual_method: actual_method.as_str().into(),
            representation: representation.as_str().into(),
            fallback_reason: plan.fallback_reason.clone(),
            correlation_id: plan.correlation_id.clone(),
            retry_of: retry_of.map(str::to_string),
            deadline_unix_ms,
            state: OutboundState::Queued.as_str().into(),
            attempt_count: 0,
        };
        self.lock_store()?
            .insert_outbound_message_with_canonical_wire(
                message,
                &route,
                ticket,
                attachments,
                encoded_wire_size,
                propagation,
                canonical_wire,
            )
            .map_err(std::io::Error::other)?;

        self.reload_state_locked(&mut state)?;
        Ok(plan)
    }

    pub fn retry_for(
        &self,
        message_id: &str,
    ) -> Result<Option<OutboundRouteRecord>, std::io::Error> {
        self.ensure_ready()?;
        self.lock_store()?.outbound_retry_for(message_id).map_err(std::io::Error::other)
    }

    pub fn begin_retry(&self, message_id: &str) -> Result<RetryStartResult, std::io::Error> {
        self.begin_retry_with_route(message_id, None)
    }

    pub fn begin_retry_with_route(
        &self,
        message_id: &str,
        route_observation: Option<&AttemptRouteObservationRecord>,
    ) -> Result<RetryStartResult, std::io::Error> {
        self.ensure_ready()?;
        let mut state = self.lock_state()?;
        let route = self
            .lock_store()?
            .outbound_route(message_id)
            .map_err(std::io::Error::other)?
            .ok_or_else(|| std::io::Error::other("outbound LXMF route is unavailable"))?;
        if self
            .lock_store()?
            .canonical_outbound_wire(message_id)
            .map_err(std::io::Error::other)?
            .is_none()
        {
            return Ok(RetryStartResult::MissingCanonicalWire);
        }
        let requested_method = DeliveryMethod::from_persisted(&route.requested_method)?;
        let actual_method = DeliveryMethod::from_persisted(&route.actual_method)?;
        let representation = WireRepresentation::from_persisted(&route.representation)?;
        let duration = if actual_method == DeliveryMethod::Opportunistic {
            OPPORTUNISTIC_DEADLINE
        } else {
            DIRECT_DEADLINE
        };
        let deadline = self.clock.monotonic_now() + duration;
        let deadline_unix_ms = self
            .clock
            .unix_time_ms()
            .saturating_add(i64::try_from(duration.as_millis()).unwrap_or(i64::MAX));
        let attempt_number = route.attempt_count.saturating_add(1);
        if attempt_number as usize > MAX_ATTEMPTS_PER_MESSAGE {
            return Err(std::io::Error::other("outbound LXMF attempt limit reached"));
        }
        let attempt = OutboundAttemptRecord {
            message_id: message_id.into(),
            attempt_number,
            started_unix_ms: self.clock.unix_time_ms(),
            deadline_unix_ms,
            state: OutboundState::Sending.as_str().into(),
            route_observation: route_observation.cloned(),
        };
        let started = self
            .lock_store()?
            .begin_outbound_retry_with_route(&attempt, route_observation)
            .map_err(std::io::Error::other)?;
        self.reload_state_locked(&mut state)?;
        if !started {
            let winner = self
                .lock_store()?
                .outbound_route(message_id)
                .map_err(std::io::Error::other)?
                .ok_or_else(|| std::io::Error::other("outbound LXMF route disappeared"))?;
            return Ok(RetryStartResult::Existing(winner));
        }
        Ok(RetryStartResult::Started(DeliveryPlan {
            requested_method,
            actual_method,
            representation,
            fallback_reason: route.fallback_reason,
            correlation_id: route.correlation_id,
            deadline,
            deadline_unix_ms,
        }))
    }

    pub fn track_evidence(
        &self,
        evidence_id: &str,
        message_id: &str,
        kind: &str,
    ) -> Result<bool, std::io::Error> {
        self.ensure_ready()?;
        self.lock_store()?
            .track_outbound_evidence(evidence_id, message_id, kind)
            .map_err(std::io::Error::other)
    }

    pub fn resolve_evidence(
        &self,
        evidence_id: &str,
    ) -> Result<Option<(String, String)>, std::io::Error> {
        self.ensure_ready()?;
        self.lock_store()?.outbound_evidence(evidence_id).map_err(std::io::Error::other)
    }

    pub fn apply_evidence(
        &self,
        message_id: &str,
        evidence: LifecycleEvidence,
    ) -> Result<bool, std::io::Error> {
        if matches!(
            &evidence,
            LifecycleEvidence::PacketDeliveryReceipt | LifecycleEvidence::ResourceDeliveryComplete
        ) {
            return Err(std::io::Error::other(
                "delivery evidence requires an exact authenticated hash",
            ));
        }
        let (state, status, detail) = match evidence {
            LifecycleEvidence::PacketDeliveryReceipt => (
                OutboundState::Delivered,
                "delivered: packet-receipt".into(),
                Some("authenticated packet receipt".to_string()),
            ),
            LifecycleEvidence::ResourceDeliveryComplete => (
                OutboundState::Delivered,
                "delivered: resource-complete".into(),
                Some("verified resource completion".to_string()),
            ),
            LifecycleEvidence::Cancelled => {
                (OutboundState::Cancelled, "cancelled".into(), Some("cancelled".into()))
            }
            LifecycleEvidence::Expired => {
                (OutboundState::Expired, "expired".into(), Some("delivery deadline expired".into()))
            }
            LifecycleEvidence::Failed(reason) => {
                (OutboundState::Failed, format!("failed: {reason}"), Some(reason))
            }
        };
        self.finish_with_detail(message_id, state, &status, detail.as_deref())
    }

    pub fn apply_correlated_evidence(
        &self,
        evidence_hash: &str,
        expected_kind: &str,
        evidence: LifecycleEvidence,
    ) -> Result<Option<String>, std::io::Error> {
        if !matches!(
            &evidence,
            LifecycleEvidence::PacketDeliveryReceipt
                | LifecycleEvidence::ResourceDeliveryComplete
                | LifecycleEvidence::Failed(_)
                | LifecycleEvidence::Cancelled
        ) {
            return Ok(None);
        }
        let Some((message_id, kind)) = self.resolve_evidence(evidence_hash)? else {
            return Ok(None);
        };
        if kind != expected_kind {
            return Ok(None);
        }
        let (state, status, detail) = match evidence {
            LifecycleEvidence::PacketDeliveryReceipt => (
                OutboundState::Delivered,
                "delivered: packet-receipt".into(),
                Some("authenticated packet receipt".to_string()),
            ),
            LifecycleEvidence::ResourceDeliveryComplete => (
                OutboundState::Delivered,
                "delivered: resource-complete".into(),
                Some("verified resource completion".to_string()),
            ),
            LifecycleEvidence::Cancelled => {
                (OutboundState::Cancelled, "cancelled".into(), Some("cancelled".into()))
            }
            LifecycleEvidence::Failed(reason) => {
                (OutboundState::Failed, format!("failed: {reason}"), Some(reason))
            }
            LifecycleEvidence::Expired => return Ok(None),
        };
        let changed = self.finish_with_exact_evidence(
            &message_id,
            state,
            &status,
            detail.as_deref(),
            evidence_hash,
            expected_kind,
        )?;
        Ok(changed.then_some(message_id))
    }

    pub fn begin_attempt(&self, message_id: &str) -> Result<OutboundAttempt, std::io::Error> {
        self.begin_attempt_with_route(message_id, None)
    }

    pub fn begin_attempt_with_route(
        &self,
        message_id: &str,
        route: Option<&AttemptRouteObservationRecord>,
    ) -> Result<OutboundAttempt, std::io::Error> {
        self.ensure_ready()?;
        let mut state = self.lock_state()?;
        let message = state
            .messages
            .get_mut(message_id)
            .ok_or_else(|| std::io::Error::other("outbound LXMF message is not queued"))?;
        if message.state.is_terminal() {
            return Err(std::io::Error::other("outbound LXMF message is already terminal"));
        }
        if message.total_attempts >= MAX_ATTEMPTS_PER_MESSAGE {
            return Err(std::io::Error::other("outbound LXMF attempt limit reached"));
        }
        let number = u32::try_from(message.total_attempts + 1).unwrap_or(u32::MAX);
        let attempt = OutboundAttempt {
            number,
            started_at: self.clock.monotonic_now(),
            deadline: message.deadline,
        };
        let persisted = OutboundAttemptRecord {
            message_id: message_id.into(),
            attempt_number: number,
            started_unix_ms: self.clock.unix_time_ms(),
            deadline_unix_ms: message.deadline_unix_ms,
            state: OutboundState::Sending.as_str().into(),
            route_observation: route.cloned(),
        };
        if !self
            .lock_store()?
            .begin_outbound_attempt_with_route(&persisted, route)
            .map_err(std::io::Error::other)?
        {
            return Err(std::io::Error::other("outbound LXMF message is already terminal"));
        }
        self.reload_state_locked(&mut state)?;
        Ok(attempt)
    }

    pub fn finish(
        &self,
        message_id: &str,
        terminal_state: OutboundState,
        receipt_status: &str,
    ) -> Result<bool, std::io::Error> {
        self.finish_with_detail(message_id, terminal_state, receipt_status, None)
    }

    fn finish_with_detail(
        &self,
        message_id: &str,
        terminal_state: OutboundState,
        receipt_status: &str,
        terminal_detail: Option<&str>,
    ) -> Result<bool, std::io::Error> {
        self.ensure_ready()?;
        let mut state = self.lock_state()?;
        let message = state
            .messages
            .get_mut(message_id)
            .ok_or_else(|| std::io::Error::other("outbound LXMF message is not queued"))?;
        if message.state.is_terminal() {
            return Ok(false);
        }
        let changed = self
            .lock_store()?
            .finish_outbound_with_detail(
                message_id,
                terminal_state.as_str(),
                receipt_status,
                terminal_detail,
            )
            .map_err(std::io::Error::other)?;
        self.reload_state_locked(&mut state)?;
        Ok(changed)
    }

    fn finish_with_exact_evidence(
        &self,
        message_id: &str,
        terminal_state: OutboundState,
        receipt_status: &str,
        terminal_detail: Option<&str>,
        evidence_hash: &str,
        evidence_kind: &str,
    ) -> Result<bool, std::io::Error> {
        self.ensure_ready()?;
        let mut state = self.lock_state()?;
        let message = state
            .messages
            .get_mut(message_id)
            .ok_or_else(|| std::io::Error::other("outbound LXMF message is not queued"))?;
        if message.state.is_terminal() {
            return Ok(false);
        }
        let changed = self
            .lock_store()?
            .finish_outbound_with_exact_evidence(
                message_id,
                terminal_state.as_str(),
                receipt_status,
                terminal_detail,
                evidence_hash,
                evidence_kind,
            )
            .map_err(std::io::Error::other)?;
        self.reload_state_locked(&mut state)?;
        Ok(changed)
    }

    pub fn remaining(&self, deadline: Instant) -> Result<Duration, std::io::Error> {
        let remaining = deadline.saturating_duration_since(self.clock.monotonic_now());
        if remaining.is_zero() {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "outbound LXMF router deadline expired",
            ))
        } else {
            Ok(remaining)
        }
    }

    pub fn cap_deadline(&self, deadline: Instant, duration: Duration) -> Instant {
        deadline.min(self.clock.monotonic_now() + duration)
    }

    pub fn message(&self, message_id: &str) -> Option<OutboundMessage> {
        self.state.lock().ok()?.messages.get(message_id).cloned()
    }

    pub fn route(&self, message_id: &str) -> Result<Option<OutboundRouteRecord>, std::io::Error> {
        self.ensure_ready()?;
        self.lock_store()?.outbound_route(message_id).map_err(std::io::Error::other)
    }

    pub fn delete_message_outcome(
        &self,
        message_id: &str,
    ) -> Result<crate::storage::messages::MessageMutationOutcome, std::io::Error> {
        self.ensure_ready()?;
        let mut state = self.lock_state()?;
        let store = self.lock_store()?;
        let outcome = store.delete_message_outcome(message_id).map_err(std::io::Error::other)?;
        if outcome.disposition == crate::storage::messages::MutationDisposition::Applied {
            state.messages.remove(message_id);
            state.insertion_order.retain(|candidate| candidate != message_id);
        }
        Ok(outcome)
    }

    pub fn delete_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<(crate::storage::messages::ConversationMutationOutcome, Vec<String>), std::io::Error>
    {
        self.ensure_ready()?;
        let mut state = self.lock_state()?;
        let store = self.lock_store()?;
        let (outcome, message_ids) =
            store.delete_conversation_outcome_with_ids(peer_hash).map_err(std::io::Error::other)?;
        if outcome.disposition == crate::storage::messages::MutationDisposition::Applied {
            for message_id in &message_ids {
                state.messages.remove(message_id);
            }
            state.insertion_order.retain(|message_id| !message_ids.contains(message_id));
        }
        Ok((outcome, message_ids))
    }

    pub fn reconcile_after_delete(&self) -> Result<(), std::io::Error> {
        let mut state = self.lock_state()?;
        self.reload_state_locked(&mut state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    fn coordinator() -> RouterCoordinator {
        RouterCoordinator::new(Arc::new(Mutex::new(MessagesStore::in_memory().unwrap())))
    }

    #[test]
    fn deletion_failure_preserves_persisted_and_coordinator_state() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let message = MessageRecord {
            id: "delete-failure".into(),
            source: "11".repeat(16),
            destination: "22".repeat(16),
            title: String::new(),
            content: "retained".into(),
            timestamp: 1,
            direction: "out".into(),
            fields: None,
            receipt_status: Some("failed".into()),
            read: true,
        };
        let route = OutboundRouteRecord {
            message_id: message.id.clone(),
            requested_method: "direct".into(),
            actual_method: "direct".into(),
            representation: "packet".into(),
            fallback_reason: None,
            correlation_id: message.id.clone(),
            retry_of: None,
            deadline_unix_ms: i64::MAX,
            state: "failed".into(),
            attempt_count: 1,
        };
        store.lock().unwrap().insert_outbound_message(&message, &route).unwrap();
        let coordinator = RouterCoordinator::new(Arc::clone(&store));
        store.lock().unwrap().fail_message_deletes_for_test().unwrap();

        assert!(coordinator.delete_message_outcome(&message.id).is_err());
        assert!(coordinator.message(&message.id).is_some());
        assert!(store.lock().unwrap().get_message(&message.id).unwrap().is_some());
        assert!(store.lock().unwrap().outbound_route(&message.id).unwrap().is_some());
    }

    fn poison_store(store: &Arc<Mutex<MessagesStore>>) {
        let store = store.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = store.lock().expect("router test store lock");
                panic!("poison router store");
            })
            .join()
            .is_err()
        );
    }

    #[test]
    fn poisoned_store_is_reported_during_startup_outbound_and_receipt_paths() {
        let startup_store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        poison_store(&startup_store);
        let startup = RouterCoordinator::new(startup_store);
        assert!(
            startup.route("missing").unwrap_err().to_string().contains("initialization failed")
        );

        let outbound_store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let outbound = RouterCoordinator::new(outbound_store.clone());
        poison_store(&outbound_store);
        assert!(outbound.queue(&message("poison-outbound"), None, 1, 1, None).is_err());

        let receipt_store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let receipt = RouterCoordinator::new(receipt_store.clone());
        receipt.queue(&message("poison-receipt"), None, 1, 1, None).unwrap();
        poison_store(&receipt_store);
        assert!(receipt.track_evidence("evidence", "poison-receipt", "packet").is_err());
        assert!(
            receipt
                .apply_evidence("poison-receipt", LifecycleEvidence::PacketDeliveryReceipt)
                .is_err()
        );
    }

    fn message(id: &str) -> MessageRecord {
        MessageRecord {
            id: id.into(),
            source: "source".into(),
            destination: "destination".into(),
            title: String::new(),
            content: String::new(),
            timestamp: 1,
            direction: "out".into(),
            fields: None,
            receipt_status: Some("queued".into()),
            read: true,
        }
    }

    #[test]
    fn encoded_packet_and_resource_boundaries_are_exact() {
        let router = coordinator();
        let packet =
            router.queue(&message("packet"), Some("direct"), LINK_PACKET_MDU, 0, None).unwrap();
        let resource = router
            .queue(&message("resource"), Some("direct"), LINK_PACKET_MDU + 1, 0, None)
            .unwrap();
        let opportunistic = router
            .queue(
                &message("opportunistic"),
                Some("opportunistic"),
                LINK_PACKET_MDU,
                LXMF_MAX_PAYLOAD,
                None,
            )
            .unwrap();
        let fallback = router
            .queue(
                &message("fallback"),
                Some("opportunistic"),
                LINK_PACKET_MDU + 1,
                LXMF_MAX_PAYLOAD + 1,
                None,
            )
            .unwrap();

        assert_eq!(packet.representation, WireRepresentation::Packet);
        assert_eq!(resource.representation, WireRepresentation::Resource);
        assert_eq!(opportunistic.actual_method, DeliveryMethod::Opportunistic);
        assert_eq!(fallback.actual_method, DeliveryMethod::Direct);
        assert_eq!(fallback.representation, WireRepresentation::Resource);
        let persisted = router.route("fallback").unwrap().unwrap();
        assert_eq!(persisted.requested_method, "opportunistic");
        assert_eq!(persisted.actual_method, "direct");
        assert_eq!(persisted.representation, "resource");
        assert!(persisted.fallback_reason.is_some());
        assert_eq!(persisted.correlation_id, "fallback");
        assert_eq!(persisted.deadline_unix_ms, fallback.deadline_unix_ms);
    }

    #[test]
    fn duplicate_ids_and_attempt_vectors_are_bounded() {
        let router = coordinator();
        router.queue(&message("message"), Some("direct"), 1, 1, None).unwrap();
        assert_eq!(
            router.queue(&message("message"), Some("direct"), 1, 1, None).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        for _ in 0..MAX_ATTEMPTS_PER_MESSAGE {
            router.begin_attempt("message").unwrap();
        }
        assert!(router.begin_attempt("message").is_err());
        assert_eq!(router.message("message").unwrap().attempts.len(), MAX_ATTEMPTS_PER_MESSAGE);
    }

    #[test]
    fn terminal_state_is_sticky_and_persisted() {
        let router = coordinator();
        router.queue(&message("message"), Some("direct"), 1, 1, None).unwrap();
        router.begin_attempt("message").unwrap();
        assert!(router.finish("message", OutboundState::Failed, "failed: test").unwrap());
        assert!(!router.finish("message", OutboundState::Sent, "sent: direct").unwrap());
        assert_eq!(router.message("message").unwrap().state, OutboundState::Failed);
        assert_eq!(router.route("message").unwrap().unwrap().state, "failed");
    }

    #[test]
    fn transport_send_alone_is_not_delivery_but_packet_receipt_is() {
        let router = coordinator();
        let packet_hash = "ab".repeat(32);
        router.queue(&message("message"), Some("direct"), 1, 1, None).unwrap();
        router.begin_attempt("message").unwrap();
        assert!(router.track_evidence(&packet_hash, "message", "packet").unwrap());

        assert!(router.finish("message", OutboundState::Sent, "sent: direct").unwrap());
        assert_eq!(router.message("message").unwrap().state, OutboundState::Sent);
        assert_eq!(
            router
                .apply_correlated_evidence(
                    &packet_hash,
                    "packet",
                    LifecycleEvidence::PacketDeliveryReceipt,
                )
                .unwrap(),
            Some("message".into())
        );
        assert_eq!(router.message("message").unwrap().state, OutboundState::Delivered);
    }

    #[test]
    fn verified_resource_completion_is_delivery() {
        let router = coordinator();
        let resource_hash = "cd".repeat(32);
        router.queue(&message("message"), Some("direct"), 1_000, 1_000, None).unwrap();
        router.begin_attempt("message").unwrap();
        assert!(router.track_evidence(&resource_hash, "message", "resource").unwrap());
        assert!(router.finish("message", OutboundState::Sent, "sent: direct").unwrap());

        assert_eq!(
            router
                .apply_correlated_evidence(
                    &resource_hash,
                    "resource",
                    LifecycleEvidence::ResourceDeliveryComplete,
                )
                .unwrap(),
            Some("message".into())
        );
        assert_eq!(router.message("message").unwrap().state, OutboundState::Delivered);
    }

    #[test]
    fn concurrent_receipt_resource_and_cancel_choose_one_sticky_terminal_outcome() {
        for (message_id, evidence) in [
            ("packet", LifecycleEvidence::PacketDeliveryReceipt),
            ("resource", LifecycleEvidence::ResourceDeliveryComplete),
        ] {
            let router = Arc::new(coordinator());
            let encoded_sizes = if message_id == "packet" { (1, 1) } else { (1_000, 1_000) };
            router
                .queue(&message(message_id), Some("direct"), encoded_sizes.0, encoded_sizes.1, None)
                .unwrap();
            router.begin_attempt(message_id).unwrap();
            let evidence_hash =
                if message_id == "packet" { "ef".repeat(32) } else { "01".repeat(32) };
            let evidence_kind = if message_id == "packet" { "packet" } else { "resource" };
            assert!(router.track_evidence(&evidence_hash, message_id, evidence_kind).unwrap());
            let barrier = Arc::new(std::sync::Barrier::new(3));
            let delivery = {
                let router = router.clone();
                let barrier = barrier.clone();
                let evidence_hash = evidence_hash.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    router
                        .apply_correlated_evidence(&evidence_hash, evidence_kind, evidence)
                        .unwrap()
                        .is_some()
                })
            };
            let cancellation = {
                let router = router.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    router.apply_evidence(message_id, LifecycleEvidence::Cancelled).unwrap()
                })
            };
            barrier.wait();

            assert_ne!(delivery.join().unwrap(), cancellation.join().unwrap());
            let terminal = router.message(message_id).unwrap().state;
            assert!(matches!(terminal, OutboundState::Delivered | OutboundState::Cancelled));
            assert!(
                !router
                    .apply_evidence(message_id, LifecycleEvidence::Failed("late".into()))
                    .unwrap()
            );
            assert_eq!(router.message(message_id).unwrap().state, terminal);
        }
    }

    #[test]
    fn evidence_correlation_survives_restart() {
        let packet_hash = "ab".repeat(32);
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        {
            let router = RouterCoordinator::new(store.clone());
            router.queue(&message("message"), Some("direct"), 1, 1, None).unwrap();
            router.begin_attempt("message").unwrap();
            router.track_evidence(&packet_hash, "message", "packet").unwrap();
        }

        let recovered = RouterCoordinator::new(store);
        assert_eq!(
            recovered.resolve_evidence(&packet_hash).unwrap(),
            Some(("message".into(), "packet".into()))
        );
        assert_eq!(
            recovered
                .apply_correlated_evidence(
                    &packet_hash,
                    "packet",
                    LifecycleEvidence::PacketDeliveryReceipt,
                )
                .unwrap(),
            Some("message".into())
        );
        assert_eq!(recovered.message("message").unwrap().state, OutboundState::Delivered);
    }

    #[test]
    fn retry_parent_is_idempotent_and_attempts_remain_bounded() {
        let router = coordinator();
        router.queue(&message("original"), Some("direct"), 1, 1, None).unwrap();
        router.begin_attempt("original").unwrap();
        router.finish("original", OutboundState::Failed, "failed: offline").unwrap();
        assert!(matches!(
            router
                .queue_retry(&message("retry"), Some("direct"), 1, 1, "original", "original")
                .unwrap(),
            RetryQueueResult::Queued(_)
        ));

        assert_eq!(router.retry_for("original").unwrap().unwrap().message_id, "retry");
        assert!(matches!(
            router
                .queue_retry(
                    &message("duplicate-retry"),
                    Some("direct"),
                    1,
                    1,
                    "original",
                    "original",
                )
                .unwrap(),
            RetryQueueResult::Existing(route) if route.message_id == "retry"
        ));
        assert_eq!(router.retry_for("original").unwrap().unwrap().message_id, "retry");
    }

    #[test]
    fn storage_failure_does_not_leave_coordinator_state() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        store.lock().unwrap().insert_message(&message("duplicate")).unwrap();
        let router = RouterCoordinator::new(store.clone());

        assert!(router.queue(&message("duplicate"), Some("direct"), 1, 1, None).is_err());
        assert!(router.message("duplicate").is_none());
        assert!(store.lock().unwrap().outbound_route("duplicate").unwrap().is_none());
    }

    struct TestClock {
        monotonic: Mutex<Instant>,
        unix_ms: AtomicI64,
    }

    impl RouterClock for TestClock {
        fn monotonic_now(&self) -> Instant {
            *self.monotonic.lock().unwrap()
        }

        fn unix_time_ms(&self) -> i64 {
            self.unix_ms.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn injected_clock_controls_deadline_expiry() {
        let start = Instant::now();
        let clock =
            Arc::new(TestClock { monotonic: Mutex::new(start), unix_ms: AtomicI64::new(1_000) });
        let router = RouterCoordinator::with_clock(
            Arc::new(Mutex::new(MessagesStore::in_memory().unwrap())),
            clock.clone(),
        );
        let plan = router.queue(&message("message"), Some("direct"), 1, 1, None).unwrap();
        *clock.monotonic.lock().unwrap() = plan.deadline;

        assert_eq!(
            router.remaining(plan.deadline).unwrap_err().kind(),
            std::io::ErrorKind::TimedOut
        );
        assert_eq!(plan.deadline_unix_ms, 1_000 + DIRECT_DEADLINE.as_millis() as i64);
    }

    #[test]
    fn restart_rehydrates_and_reconciles_interrupted_states() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let start = Instant::now();
        let clock =
            Arc::new(TestClock { monotonic: Mutex::new(start), unix_ms: AtomicI64::new(1_000) });
        {
            let router = RouterCoordinator::with_clock(store.clone(), clock.clone());
            router.queue(&message("queued"), Some("direct"), 1, 1, None).unwrap();
            router.queue(&message("sending"), Some("direct"), 1, 1, None).unwrap();
            router.begin_attempt("sending").unwrap();
            router.queue(&message("sent"), Some("direct"), 1, 1, None).unwrap();
            router.begin_attempt("sent").unwrap();
            router.finish("sent", OutboundState::Sent, "sent: direct").unwrap();
        }

        clock.unix_ms.store(2_000, Ordering::Relaxed);
        *clock.monotonic.lock().unwrap() = start + Duration::from_secs(1);
        let recovered = RouterCoordinator::with_clock(store.clone(), clock.clone());

        assert_eq!(recovered.message("queued").unwrap().state, OutboundState::Queued);
        assert_eq!(recovered.message("sending").unwrap().state, OutboundState::Queued);
        assert_eq!(recovered.message("sent").unwrap().state, OutboundState::Sent);
        assert_eq!(
            store.lock().unwrap().outbound_attempts("sending").unwrap()[0].state,
            "interrupted"
        );

        clock.unix_ms.store(34_000, Ordering::Relaxed);
        *clock.monotonic.lock().unwrap() = start + Duration::from_secs(33);
        assert_eq!(recovered.reconcile_deadlines().unwrap().len(), 3);
        assert_eq!(recovered.message("queued").unwrap().state, OutboundState::Expired);
        assert_eq!(recovered.message("sending").unwrap().state, OutboundState::Expired);
        assert_eq!(recovered.message("sent").unwrap().state, OutboundState::Expired);
    }

    #[test]
    fn legacy_correlated_messages_keep_independent_attempt_projections_after_restart() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        {
            let router = RouterCoordinator::new(store.clone());
            router.queue(&message("original"), Some("opportunistic"), 1, 1, None).unwrap();
            assert_eq!(router.begin_attempt("original").unwrap().number, 1);
            router.finish("original", OutboundState::Failed, "failed: test").unwrap();
        }

        let router = RouterCoordinator::new(store.clone());
        router.queue(&message("retry"), Some("opportunistic"), 1, 1, Some("original")).unwrap();
        assert_eq!(router.begin_attempt("retry").unwrap().number, 1);
        let retry = router.message("retry").unwrap();
        assert_eq!(retry.requested_method, DeliveryMethod::Opportunistic);
        assert_eq!(retry.correlation_id, "original");
        assert_eq!(retry.total_attempts, 1);
        assert_eq!(retry.attempts.iter().map(|attempt| attempt.number).collect::<Vec<_>>(), [1]);
        assert_eq!(
            store.lock().unwrap().outbound_attempts_for_correlation("original").unwrap().len(),
            2
        );
    }

    #[test]
    fn committed_route_missing_from_memory_is_rehydrated_on_restart() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let route = OutboundRouteRecord {
            message_id: "committed".into(),
            requested_method: "direct".into(),
            actual_method: "direct".into(),
            representation: "packet".into(),
            fallback_reason: None,
            correlation_id: "committed".into(),
            retry_of: None,
            deadline_unix_ms: SystemRouterClock.unix_time_ms() + 30_000,
            state: "queued".into(),
            attempt_count: 0,
        };
        store.lock().unwrap().insert_outbound_message(&message("committed"), &route).unwrap();

        let router = RouterCoordinator::new(store);

        assert_eq!(router.message("committed").unwrap().state, OutboundState::Queued);
    }

    #[test]
    fn lifecycle_metadata_and_per_message_attempts_survive_database_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("messages.sqlite");
        {
            let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
            let router = RouterCoordinator::new(store);
            let plan = router
                .queue(
                    &message("first"),
                    Some("opportunistic"),
                    LINK_PACKET_MDU + 1,
                    LXMF_MAX_PAYLOAD + 1,
                    Some("send-correlation"),
                )
                .unwrap();
            assert_eq!(plan.actual_method, DeliveryMethod::Direct);
            router.begin_attempt("first").unwrap();
            router.finish("first", OutboundState::Failed, "failed: offline").unwrap();
        }

        let reopened_store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
        let reopened = RouterCoordinator::new(reopened_store.clone());
        let recovered_message = reopened.message("first").unwrap();
        assert_eq!(recovered_message.requested_method, DeliveryMethod::Opportunistic);
        assert_eq!(recovered_message.actual_method, DeliveryMethod::Direct);
        assert!(
            recovered_message
                .fallback_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("packet limit"))
        );
        assert_eq!(recovered_message.correlation_id, "send-correlation");
        assert_eq!(recovered_message.total_attempts, 1);

        reopened
            .queue(
                &message("retry"),
                Some("opportunistic"),
                LINK_PACKET_MDU + 1,
                LXMF_MAX_PAYLOAD + 1,
                Some("send-correlation"),
            )
            .unwrap();
        assert_eq!(reopened.begin_attempt("retry").unwrap().number, 1);
        drop(reopened);
        drop(reopened_store);

        let final_store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
        let final_router = RouterCoordinator::new(final_store);
        let retry = final_router.message("retry").unwrap();
        assert_eq!(retry.correlation_id, "send-correlation");
        assert_eq!(retry.total_attempts, 1);
        assert_eq!(retry.attempts.iter().map(|attempt| attempt.number).collect::<Vec<_>>(), [1]);
    }
}
