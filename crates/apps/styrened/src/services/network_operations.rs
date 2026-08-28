//! Daemon-owned coordination for bounded Reticulum operator operations.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use rand_core::{OsRng, RngCore};
use rns_core::hash::AddressHash;
use styrene_ipc::types::{
    NetworkOperationInfo, NetworkOperationKind, NetworkOperationOutcome, NetworkOperationProgress,
    ObservationMetadata, ObservationSource, StartNetworkOperationInfo,
};
use tokio_util::sync::CancellationToken;

use crate::services::EventService;
use crate::transport::mesh_transport::{
    LinkOpenResult, MeshTransport, TransportError, TransportLifecycleEvent,
};

const CAPACITY: usize = 256;
const MAX_TIMEOUT_MS: u64 = 120_000;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

struct OperationEntry {
    info: NetworkOperationInfo,
    cancellation: CancellationToken,
    deadline: tokio::time::Instant,
    terminal: tokio::sync::watch::Sender<bool>,
}

enum OperationCompletion {
    Succeeded { rtt_ms: Option<f64>, created_link: Option<AddressHash> },
    Dispatched(&'static str),
}

impl OperationCompletion {
    fn created_link(&self) -> Option<AddressHash> {
        match self {
            Self::Succeeded { created_link, .. } => *created_link,
            Self::Dispatched(_) => None,
        }
    }
}

pub struct NetworkOperationService {
    transport: Arc<dyn MeshTransport>,
    events: Arc<EventService>,
    entries: Mutex<BTreeMap<String, OperationEntry>>,
    order: Mutex<VecDeque<String>>,
    probe_locks: Mutex<BTreeMap<AddressHash, Weak<tokio::sync::Mutex<()>>>>,
    capacity: usize,
}

impl NetworkOperationService {
    pub fn new(transport: Arc<dyn MeshTransport>, events: Arc<EventService>) -> Arc<Self> {
        Self::with_capacity(transport, events, CAPACITY)
    }

    fn with_capacity(
        transport: Arc<dyn MeshTransport>,
        events: Arc<EventService>,
        capacity: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            transport,
            events,
            entries: Mutex::new(BTreeMap::new()),
            order: Mutex::new(VecDeque::new()),
            probe_locks: Mutex::new(BTreeMap::new()),
            capacity,
        })
    }

    pub fn start(
        self: &Arc<Self>,
        request: StartNetworkOperationInfo,
    ) -> Result<NetworkOperationInfo, String> {
        validate(&request)?;
        let info = self.insert(request, None)?;
        let service = self.clone();
        let operation_id = info.operation_id.clone();
        tokio::spawn(async move { service.run(operation_id).await });
        Ok(info)
    }

    pub fn denied(
        &self,
        request: StartNetworkOperationInfo,
        reason: String,
    ) -> Result<NetworkOperationInfo, String> {
        validate(&request)?;
        self.insert(request, Some((NetworkOperationOutcome::Denied, reason)))
    }

    pub fn get(&self, operation_id: &str) -> Option<NetworkOperationInfo> {
        self.entries.lock().unwrap().get(operation_id).map(|entry| entry.info.clone())
    }

    pub fn list(&self) -> Vec<NetworkOperationInfo> {
        let entries = self.entries.lock().unwrap();
        self.order
            .lock()
            .unwrap()
            .iter()
            .filter_map(|id| entries.get(id).map(|entry| entry.info.clone()))
            .collect()
    }

    pub async fn cancel(&self, operation_id: &str) -> Result<NetworkOperationInfo, String> {
        let (token, mut terminal) = {
            let mut entries = self.entries.lock().unwrap();
            let entry = entries.get_mut(operation_id).ok_or("network operation not found")?;
            if entry.info.is_terminal() {
                return Ok(entry.info.clone());
            }
            if !entry.info.cancellable {
                return Err("network operation is not cancellable".into());
            }
            (entry.cancellation.clone(), entry.terminal.subscribe())
        };
        token.cancel();
        if !*terminal.borrow() {
            terminal.changed().await.map_err(|_| "operation completion channel closed")?;
        }
        self.get(operation_id).ok_or_else(|| "network operation disappeared".into())
    }

    fn insert(
        &self,
        request: StartNetworkOperationInfo,
        terminal: Option<(NetworkOperationOutcome, String)>,
    ) -> Result<NetworkOperationInfo, String> {
        let mut id = [0u8; 16];
        OsRng.fill_bytes(&mut id);
        let operation_id = hex::encode(id);
        let now = unix_ms();
        let mut info = NetworkOperationInfo::default();
        info.operation_id = operation_id.clone();
        info.kind = request.kind;
        info.destination_hash = request.destination_hash;
        info.link_id = request.link_id;
        info.started_unix_ms = now;
        info.deadline_unix_ms = now.saturating_add(request.timeout_ms as i64);
        info.cancellable = matches!(
            info.kind,
            NetworkOperationKind::PathRequest
                | NetworkOperationKind::Probe
                | NetworkOperationKind::LinkOpen
        );
        info.progress = NetworkOperationProgress::Accepted;
        info.observation = operation_observation(&operation_id);
        if let Some((outcome, detail)) = terminal {
            info.outcome = Some(outcome);
            info.detail = Some(detail);
        }

        let mut entries = self.entries.lock().unwrap();
        let mut order = self.order.lock().unwrap();
        while entries.len() >= self.capacity {
            let Some(index) = order
                .iter()
                .position(|id| entries.get(id).is_some_and(|entry| entry.info.is_terminal()))
            else {
                return Err("network operation capacity exhausted".into());
            };
            if let Some(evicted) = order.remove(index) {
                entries.remove(&evicted);
            }
        }
        let (terminal, _) = tokio::sync::watch::channel(false);
        entries.insert(
            operation_id.clone(),
            OperationEntry {
                info: info.clone(),
                cancellation: CancellationToken::new(),
                deadline: tokio::time::Instant::now() + Duration::from_millis(request.timeout_ms),
                terminal,
            },
        );
        order.push_back(operation_id);
        drop(order);
        drop(entries);
        self.events.emit_network_operation(info.clone());
        Ok(info)
    }

    async fn run(self: Arc<Self>, operation_id: String) {
        let Some(info) = self.get(&operation_id) else { return };
        let (cancellation, deadline) = {
            let entries = self.entries.lock().unwrap();
            let Some(entry) = entries.get(&operation_id) else { return };
            (entry.cancellation.clone(), entry.deadline)
        };
        let result = self.execute(&operation_id, info.clone(), cancellation, deadline).await;
        if tokio::time::Instant::now() >= deadline && result.is_ok() {
            let cleanup = if let Some(link_id) =
                result.as_ref().ok().and_then(OperationCompletion::created_link)
            {
                self.transport.cancel_link_open(&link_id).await
            } else {
                Ok(())
            };
            if let Err(error) = cleanup {
                self.terminal(
                    &operation_id,
                    NetworkOperationOutcome::Failed,
                    Some(error.to_string()),
                    None,
                );
            } else {
                self.terminal(&operation_id, NetworkOperationOutcome::TimedOut, None, None);
            }
            return;
        }
        match result {
            Ok(OperationCompletion::Succeeded { rtt_ms, .. }) => {
                self.terminal(&operation_id, NetworkOperationOutcome::Succeeded, None, rtt_ms)
            }
            Ok(OperationCompletion::Dispatched(detail)) => self.terminal(
                &operation_id,
                NetworkOperationOutcome::Dispatched,
                Some(detail.into()),
                None,
            ),
            Err(error) => {
                let outcome = match error {
                    TransportError::Cancelled => NetworkOperationOutcome::Cancelled,
                    TransportError::TimedOut => NetworkOperationOutcome::TimedOut,
                    TransportError::Unavailable => NetworkOperationOutcome::Unavailable,
                    _ => NetworkOperationOutcome::Failed,
                };
                self.terminal(&operation_id, outcome, Some(error.to_string()), None);
            }
        }
    }

    async fn execute(
        &self,
        operation_id: &str,
        info: NetworkOperationInfo,
        cancellation: CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<OperationCompletion, TransportError> {
        if !self.transport.is_connected() {
            return Err(TransportError::Unavailable);
        }
        match info.kind {
            NetworkOperationKind::Announce => {
                self.progress(operation_id, NetworkOperationProgress::Dispatched, None);
                ensure_before_deadline(deadline)?;
                self.transport.dispatch_announce(None).await?;
                Ok(OperationCompletion::Dispatched(
                    "announce accepted by local transport; remote reception is unconfirmed",
                ))
            }
            NetworkOperationKind::PathRequest => {
                let destination = parse_address(info.destination_hash.as_deref())?;
                ensure_not_cancelled(&cancellation)?;
                self.transport.request_path(&destination).await;
                self.progress(operation_id, NetworkOperationProgress::AwaitingPath, None);
                loop {
                    ensure_before_deadline(deadline)?;
                    if self.transport.query_path(&destination).await.is_some() {
                        ensure_before_deadline(deadline)?;
                        return Ok(OperationCompletion::Succeeded {
                            rtt_ms: None,
                            created_link: None,
                        });
                    }
                    wait_for_poll(&cancellation, deadline).await?;
                }
            }
            NetworkOperationKind::LinkOpen => {
                let destination = parse_address(info.destination_hash.as_deref())?;
                let mut lifecycle = self.transport.subscribe_lifecycle();
                ensure_not_cancelled(&cancellation)?;
                let open = self
                    .transport
                    .open_link(
                        &destination,
                        cancellation.clone(),
                        deadline.saturating_duration_since(tokio::time::Instant::now()),
                    )
                    .await?;
                let link_id = open.link_id();
                self.progress(
                    operation_id,
                    NetworkOperationProgress::AwaitingLink,
                    Some(hex::encode(link_id.as_slice())),
                );
                if matches!(open, LinkOpenResult::Reused(_)) {
                    let snapshot = self.transport.link_lifecycle_snapshot().await;
                    if let Some(active) = snapshot.active.iter().find(|link| {
                        link.id == link_id
                            && link.status
                                == rns_core::transport::destination_ext::link::LinkStatus::Active
                    }) {
                        ensure_before_deadline(deadline)?;
                        return Ok(OperationCompletion::Succeeded {
                            rtt_ms: active.rtt.map(|rtt| rtt.as_secs_f64() * 1_000.0),
                            created_link: None,
                        });
                    }
                }
                loop {
                    match wait_for_lifecycle(&mut lifecycle, &cancellation, deadline).await {
                        Ok(TransportLifecycleEvent::LinkActivated {
                            link_id: observed,
                            rtt_ms,
                            ..
                        }) if observed == hex::encode(link_id.as_slice()) => {
                            ensure_before_deadline(deadline)?;
                            return Ok(OperationCompletion::Succeeded {
                                rtt_ms: Some(rtt_ms),
                                created_link: open.is_created().then_some(link_id),
                            });
                        }
                        Ok(TransportLifecycleEvent::LinkClosed { link_id: observed, .. })
                            if observed == hex::encode(link_id.as_slice()) =>
                        {
                            return Err(TransportError::LinkFailed(
                                "link closed before establishment".into(),
                            ));
                        }
                        Ok(_) => {}
                        Err(TransportError::Cancelled) => {
                            return if open.is_created() {
                                self.cleanup_link_open(link_id, TransportError::Cancelled).await
                            } else {
                                Err(TransportError::Cancelled)
                            };
                        }
                        Err(TransportError::TimedOut) => {
                            return if open.is_created() {
                                self.cleanup_link_open(link_id, TransportError::TimedOut).await
                            } else {
                                Err(TransportError::TimedOut)
                            };
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            NetworkOperationKind::Probe => {
                let link_id = parse_address(info.link_id.as_deref())?;
                let _probe_guard = self.probe_guard(link_id, &cancellation, deadline).await?;
                let mut lifecycle = self.transport.subscribe_lifecycle();
                ensure_not_cancelled(&cancellation)?;
                self.transport.probe_link(&link_id).await?;
                self.progress(operation_id, NetworkOperationProgress::AwaitingProbe, None);
                loop {
                    match wait_for_lifecycle(&mut lifecycle, &cancellation, deadline).await? {
                        TransportLifecycleEvent::LinkRttUpdated {
                            link_id: observed,
                            rtt_ms,
                            ..
                        } if observed == hex::encode(link_id.as_slice()) => {
                            ensure_before_deadline(deadline)?;
                            return Ok(OperationCompletion::Succeeded {
                                rtt_ms: Some(rtt_ms),
                                created_link: None,
                            });
                        }
                        _ => {}
                    }
                }
            }
            NetworkOperationKind::LinkClose => {
                let link_id = parse_address(info.link_id.as_deref())?;
                self.progress(operation_id, NetworkOperationProgress::Dispatched, None);
                ensure_before_deadline(deadline)?;
                self.transport.close_link(&link_id).await?;
                Ok(OperationCompletion::Dispatched(
                    "canonical LINKCLOSE accepted locally; remote processing is unacknowledged",
                ))
            }
            NetworkOperationKind::Unknown => Err(TransportError::Unavailable),
            _ => Err(TransportError::Unavailable),
        }
    }

    async fn cleanup_link_open(
        &self,
        link_id: AddressHash,
        terminal: TransportError,
    ) -> Result<OperationCompletion, TransportError> {
        self.transport
            .cancel_link_open(&link_id)
            .await
            .map_err(|error| TransportError::CleanupFailed(format!("{terminal}; {error}")))?;
        Err(terminal)
    }

    async fn probe_guard(
        &self,
        link_id: AddressHash,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, TransportError> {
        let lock = {
            let mut locks = self.probe_locks.lock().unwrap();
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(&link_id).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(link_id, Arc::downgrade(&lock));
                lock
            }
        };
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => Err(TransportError::TimedOut),
            _ = cancellation.cancelled() => Err(TransportError::Cancelled),
            guard = lock.lock_owned() => Ok(guard),
        }
    }

    fn progress(
        &self,
        operation_id: &str,
        progress: NetworkOperationProgress,
        link_id: Option<String>,
    ) {
        let event = {
            let mut entries = self.entries.lock().unwrap();
            let Some(entry) = entries.get_mut(operation_id) else { return };
            if entry.info.is_terminal() {
                return;
            }
            entry.info.progress = progress;
            if link_id.is_some() {
                entry.info.link_id = link_id;
            }
            entry.info.observation = operation_observation(operation_id);
            entry.info.clone()
        };
        self.events.emit_network_operation(event);
    }

    fn terminal(
        &self,
        operation_id: &str,
        outcome: NetworkOperationOutcome,
        detail: Option<String>,
        rtt_ms: Option<f64>,
    ) {
        let (event, terminal) = {
            let mut entries = self.entries.lock().unwrap();
            let Some(entry) = entries.get_mut(operation_id) else { return };
            if entry.info.is_terminal() {
                return;
            }
            entry.info.outcome = Some(outcome);
            entry.info.detail = detail;
            entry.info.rtt_ms = rtt_ms;
            entry.info.observation = operation_observation(operation_id);
            (entry.info.clone(), entry.terminal.clone())
        };
        self.events.emit_network_operation(event);
        terminal.send_replace(true);
    }
}

fn ensure_before_deadline(deadline: tokio::time::Instant) -> Result<(), TransportError> {
    if tokio::time::Instant::now() >= deadline { Err(TransportError::TimedOut) } else { Ok(()) }
}

fn ensure_not_cancelled(cancellation: &CancellationToken) -> Result<(), TransportError> {
    if cancellation.is_cancelled() { Err(TransportError::Cancelled) } else { Ok(()) }
}

async fn wait_for_poll(
    cancellation: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<(), TransportError> {
    tokio::select! {
        biased;
        _ = tokio::time::sleep_until(deadline) => Err(TransportError::TimedOut),
        _ = cancellation.cancelled() => Err(TransportError::Cancelled),
        _ = tokio::time::sleep(POLL_INTERVAL) => Ok(()),
    }
}

async fn wait_for_lifecycle(
    lifecycle: &mut tokio::sync::broadcast::Receiver<TransportLifecycleEvent>,
    cancellation: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<TransportLifecycleEvent, TransportError> {
    loop {
        let received = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => return Err(TransportError::TimedOut),
            _ = cancellation.cancelled() => return Err(TransportError::Cancelled),
            received = lifecycle.recv() => received,
        };
        match received {
            Ok(event) => return Ok(event),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return Err(TransportError::Unavailable);
            }
        }
    }
}

fn validate(request: &StartNetworkOperationInfo) -> Result<(), String> {
    if request.timeout_ms == 0 || request.timeout_ms > MAX_TIMEOUT_MS {
        return Err(format!("timeout_ms must be between 1 and {MAX_TIMEOUT_MS}"));
    }
    match request.kind {
        NetworkOperationKind::Announce => {
            if request.destination_hash.is_some() || request.link_id.is_some() {
                return Err("announce does not accept a destination_hash or link_id".into());
            }
        }
        NetworkOperationKind::PathRequest | NetworkOperationKind::LinkOpen => {
            parse_hash(request.destination_hash.as_deref(), "destination_hash")?;
            if request.link_id.is_some() {
                return Err("operation does not accept link_id".into());
            }
        }
        NetworkOperationKind::Probe | NetworkOperationKind::LinkClose => {
            parse_hash(request.link_id.as_deref(), "link_id")?;
        }
        NetworkOperationKind::Unknown => return Err("unknown network operation kind".into()),
        _ => return Err("unsupported network operation kind".into()),
    }
    Ok(())
}

fn parse_hash(value: Option<&str>, name: &str) -> Result<[u8; 16], String> {
    let value = value.ok_or_else(|| format!("missing {name}"))?;
    hex::decode(value)
        .map_err(|_| format!("invalid {name}"))?
        .try_into()
        .map_err(|_| format!("{name} must be 16 bytes"))
}

fn parse_address(value: Option<&str>) -> Result<AddressHash, TransportError> {
    parse_hash(value, "operation target").map(AddressHash::new).map_err(TransportError::SendFailed)
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn operation_observation(operation_id: &str) -> ObservationMetadata {
    let now = unix_ms() / 1_000;
    let mut observation =
        ObservationMetadata::at(ObservationSource::OperationCoordinator, Some(now), now, 300);
    observation.correlation_id = Some(operation_id.to_string());
    observation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::mock_transport::MockTransport;

    fn request(kind: NetworkOperationKind) -> StartNetworkOperationInfo {
        let mut request = StartNetworkOperationInfo::default();
        request.kind = kind;
        request.timeout_ms = 100;
        request
    }

    async fn terminal(
        service: &NetworkOperationService,
        operation_id: &str,
    ) -> NetworkOperationInfo {
        let mut terminal = {
            let entries = service.entries.lock().unwrap();
            entries.get(operation_id).expect("operation entry").terminal.subscribe()
        };
        if !*terminal.borrow() {
            terminal.changed().await.expect("operation terminal milestone");
        }
        service.get(operation_id).expect("terminal operation")
    }

    fn is_request_path(call: &crate::transport::mock_transport::MockCall) -> bool {
        matches!(call, crate::transport::mock_transport::MockCall::RequestPath { .. })
    }

    fn is_open_link(call: &crate::transport::mock_transport::MockCall) -> bool {
        matches!(call, crate::transport::mock_transport::MockCall::OpenLink { .. })
    }

    fn is_probe_link(call: &crate::transport::mock_transport::MockCall) -> bool {
        matches!(call, crate::transport::mock_transport::MockCall::ProbeLink { .. })
    }

    #[tokio::test(start_paused = true)]
    async fn all_operation_kinds_complete_from_transport_evidence() {
        let transport = Arc::new(MockTransport::new_default());
        let events = Arc::new(EventService::new());
        let service = NetworkOperationService::new(transport.clone(), events);
        let destination = AddressHash::new([1; 16]);
        let link_id = AddressHash::new([2; 16]);

        let announce = service.start(request(NetworkOperationKind::Announce)).unwrap();
        assert_eq!(
            terminal(&service, &announce.operation_id).await.outcome,
            Some(NetworkOperationOutcome::Dispatched)
        );

        transport.set_path(destination, 1, AddressHash::new([3; 16]));
        let mut path = request(NetworkOperationKind::PathRequest);
        path.destination_hash = Some(hex::encode(destination.as_slice()));
        let path = service.start(path).unwrap();
        assert_eq!(
            terminal(&service, &path.operation_id).await.outcome,
            Some(NetworkOperationOutcome::Succeeded)
        );

        transport.queue_open_link(Ok(link_id));
        let mut open = request(NetworkOperationKind::LinkOpen);
        open.destination_hash = Some(hex::encode(destination.as_slice()));
        let open = service.start(open).unwrap();
        transport.wait_for_calls(1, is_open_link).await;
        transport.inject_lifecycle(TransportLifecycleEvent::LinkActivated {
            link_id: hex::encode(link_id.as_slice()),
            peer_hash: hex::encode(destination.as_slice()),
            interface: Some("iface".into()),
            rtt_ms: 4.0,
        });
        let opened = terminal(&service, &open.operation_id).await;
        assert_eq!(opened.outcome, Some(NetworkOperationOutcome::Succeeded));
        assert_eq!(opened.link_id, Some(hex::encode(link_id.as_slice())));

        transport.queue_probe(Ok(()));
        let mut probe = request(NetworkOperationKind::Probe);
        probe.link_id = Some(hex::encode(link_id.as_slice()));
        let probe = service.start(probe).unwrap();
        transport.wait_for_calls(1, is_probe_link).await;
        transport.inject_lifecycle(TransportLifecycleEvent::LinkRttUpdated {
            link_id: hex::encode(link_id.as_slice()),
            peer_hash: hex::encode(destination.as_slice()),
            interface: Some("iface".into()),
            rtt_ms: 7.0,
        });
        assert_eq!(terminal(&service, &probe.operation_id).await.rtt_ms, Some(7.0));

        transport.queue_close(Ok(()));
        let mut close = request(NetworkOperationKind::LinkClose);
        close.link_id = Some(hex::encode(link_id.as_slice()));
        let close = service.start(close).unwrap();
        assert_eq!(
            terminal(&service, &close.operation_id).await.outcome,
            Some(NetworkOperationOutcome::Dispatched)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_denial_and_unavailable_are_typed_terminal_outcomes() {
        let transport = Arc::new(MockTransport::new_default());
        let service =
            NetworkOperationService::new(transport.clone(), Arc::new(EventService::new()));
        let mut path = request(NetworkOperationKind::PathRequest);
        path.destination_hash = Some("11".repeat(16));
        let timed = service.start(path.clone()).unwrap();
        transport.wait_for_calls(1, is_request_path).await;
        tokio::time::advance(Duration::from_millis(101)).await;
        assert_eq!(
            terminal(&service, &timed.operation_id).await.outcome,
            Some(NetworkOperationOutcome::TimedOut)
        );

        let denied = service.denied(path, "permission denied".into()).unwrap();
        assert_eq!(denied.outcome, Some(NetworkOperationOutcome::Denied));
        assert_eq!(
            denied.observation.correlation_id.as_deref(),
            Some(denied.operation_id.as_str())
        );

        transport.set_connected(false);
        let unavailable = service.start(request(NetworkOperationKind::Announce)).unwrap();
        assert_eq!(
            terminal(&service, &unavailable.operation_id).await.outcome,
            Some(NetworkOperationOutcome::Unavailable)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_is_immediate_and_late_success_cannot_revive_it() {
        let transport = Arc::new(MockTransport::new_default());
        let service =
            NetworkOperationService::new(transport.clone(), Arc::new(EventService::new()));
        let destination = AddressHash::new([7; 16]);
        let mut path = request(NetworkOperationKind::PathRequest);
        path.destination_hash = Some(hex::encode(destination.as_slice()));
        let started = service.start(path).unwrap();
        transport.wait_for_calls(1, is_request_path).await;

        let cancelled = service.cancel(&started.operation_id).await.unwrap();
        assert_eq!(cancelled.outcome, Some(NetworkOperationOutcome::Cancelled));
        transport.set_path(destination, 1, AddressHash::new([6; 16]));
        tokio::time::advance(Duration::from_secs(1)).await;

        assert_eq!(
            service.get(&started.operation_id).unwrap().outcome,
            Some(NetworkOperationOutcome::Cancelled)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn bounded_capacity_never_evicts_in_flight_work() {
        let transport = Arc::new(MockTransport::new_default());
        let service =
            NetworkOperationService::with_capacity(transport, Arc::new(EventService::new()), 1);
        let mut path = request(NetworkOperationKind::PathRequest);
        path.destination_hash = Some("11".repeat(16));
        let first = service.start(path.clone()).unwrap();

        assert_eq!(service.start(path).unwrap_err(), "network operation capacity exhausted");
        assert!(service.get(&first.operation_id).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn probe_ignores_preexisting_rtt_and_deadline_wins_exact_event_race() {
        use rns_core::transport::destination_ext::link::{LinkStateSnapshot, LinkStatus};

        let transport = Arc::new(MockTransport::new_default());
        let service =
            NetworkOperationService::new(transport.clone(), Arc::new(EventService::new()));
        let link_id = AddressHash::new([4; 16]);
        transport.set_link_snapshots(vec![LinkStateSnapshot {
            id: link_id,
            address_hash: AddressHash::new([5; 16]),
            interface: Some(AddressHash::new([6; 16])),
            rtt: Some(Duration::from_millis(1)),
            status: LinkStatus::Active,
            remote_identity: None,
            close_reason: None,
            observed_at: std::time::SystemTime::now(),
            age: Duration::ZERO,
        }]);
        transport.queue_probe(Ok(()));
        let mut probe = request(NetworkOperationKind::Probe);
        probe.link_id = Some(hex::encode(link_id.as_slice()));
        let probe = service.start(probe).unwrap();
        transport.wait_for_calls(1, is_probe_link).await;

        tokio::time::advance(Duration::from_millis(100)).await;
        transport.inject_lifecycle(TransportLifecycleEvent::LinkRttUpdated {
            link_id: hex::encode(link_id.as_slice()),
            peer_hash: "peer".into(),
            interface: None,
            rtt_ms: 9.0,
        });
        terminal(&service, &probe.operation_id).await;

        assert_eq!(
            service.get(&probe.operation_id).unwrap().outcome,
            Some(NetworkOperationOutcome::TimedOut)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_probes_on_one_link_require_distinct_rtt_responses() {
        let transport = Arc::new(MockTransport::new_default());
        let service =
            NetworkOperationService::new(transport.clone(), Arc::new(EventService::new()));
        let link_id = AddressHash::new([4; 16]);
        transport.queue_probe(Ok(()));
        transport.queue_probe(Ok(()));
        let mut request = request(NetworkOperationKind::Probe);
        request.link_id = Some(hex::encode(link_id.as_slice()));
        let first = service.start(request.clone()).unwrap();
        let second = service.start(request).unwrap();

        transport.wait_for_calls(1, is_probe_link).await;
        assert_eq!(
            transport
                .calls()
                .iter()
                .filter(|call| matches!(
                    call,
                    crate::transport::mock_transport::MockCall::ProbeLink { .. }
                ))
                .count(),
            1
        );

        transport.inject_lifecycle(TransportLifecycleEvent::LinkRttUpdated {
            link_id: hex::encode(link_id.as_slice()),
            peer_hash: "peer".into(),
            interface: None,
            rtt_ms: 7.0,
        });
        transport.wait_for_calls(2, is_probe_link).await;
        let completed_after_one = [&first, &second]
            .into_iter()
            .filter(|operation| {
                service.get(&operation.operation_id).is_some_and(|info| info.is_terminal())
            })
            .count();
        assert_eq!(completed_after_one, 1);

        transport.inject_lifecycle(TransportLifecycleEvent::LinkRttUpdated {
            link_id: hex::encode(link_id.as_slice()),
            peer_hash: "peer".into(),
            interface: None,
            rtt_ms: 9.0,
        });
        let first = terminal(&service, &first.operation_id).await;
        let second = terminal(&service, &second.operation_id).await;
        let mut rtts = [first.rtt_ms.unwrap(), second.rtt_ms.unwrap()];
        rtts.sort_by(f64::total_cmp);
        assert_eq!(rtts, [7.0, 9.0]);
    }

    #[tokio::test(start_paused = true)]
    async fn active_link_reuse_succeeds_from_snapshot_without_cleanup() {
        use rns_core::transport::destination_ext::link::{LinkStateSnapshot, LinkStatus};

        let transport = Arc::new(MockTransport::new_default());
        let service =
            NetworkOperationService::new(transport.clone(), Arc::new(EventService::new()));
        let destination = AddressHash::new([7; 16]);
        let link_id = AddressHash::new([8; 16]);
        transport.set_link_snapshots(vec![LinkStateSnapshot {
            id: link_id,
            address_hash: destination,
            interface: Some(AddressHash::new([9; 16])),
            rtt: Some(Duration::from_millis(4)),
            status: LinkStatus::Active,
            remote_identity: None,
            close_reason: None,
            observed_at: std::time::SystemTime::now(),
            age: Duration::ZERO,
        }]);
        transport.queue_reused_link(link_id);
        let mut open = request(NetworkOperationKind::LinkOpen);
        open.destination_hash = Some(hex::encode(destination.as_slice()));

        let opened = service.start(open).unwrap();
        let terminal = terminal(&service, &opened.operation_id).await;

        assert_eq!(terminal.outcome, Some(NetworkOperationOutcome::Succeeded));
        assert_eq!(terminal.rtt_ms, Some(4.0));
        assert!(!transport.calls().iter().any(|call| matches!(
            call,
            crate::transport::mock_transport::MockCall::CancelLinkOpen { .. }
        )));
    }

    #[tokio::test(start_paused = true)]
    async fn pending_link_reuse_cancel_and_timeout_never_cleanup_unowned_state() {
        let destination = AddressHash::new([7; 16]);
        let link_id = AddressHash::new([8; 16]);

        let transport = Arc::new(MockTransport::new_default());
        let service =
            NetworkOperationService::new(transport.clone(), Arc::new(EventService::new()));
        transport.queue_reused_link(link_id);
        let mut open = request(NetworkOperationKind::LinkOpen);
        open.destination_hash = Some(hex::encode(destination.as_slice()));
        let opened = service.start(open).unwrap();
        transport.wait_for_calls(1, is_open_link).await;

        let cancelled = service.cancel(&opened.operation_id).await.unwrap();
        assert_eq!(cancelled.outcome, Some(NetworkOperationOutcome::Cancelled));
        assert!(!transport.calls().iter().any(|call| matches!(
            call,
            crate::transport::mock_transport::MockCall::CancelLinkOpen { .. }
        )));

        let transport = Arc::new(MockTransport::new_default());
        let service =
            NetworkOperationService::new(transport.clone(), Arc::new(EventService::new()));
        transport.queue_reused_link(link_id);
        let mut open = request(NetworkOperationKind::LinkOpen);
        open.destination_hash = Some(hex::encode(destination.as_slice()));
        let opened = service.start(open).unwrap();
        transport.wait_for_calls(1, is_open_link).await;
        tokio::time::advance(Duration::from_millis(100)).await;
        terminal(&service, &opened.operation_id).await;

        assert_eq!(
            service.get(&opened.operation_id).unwrap().outcome,
            Some(NetworkOperationOutcome::TimedOut)
        );
        assert!(!transport.calls().iter().any(|call| matches!(
            call,
            crate::transport::mock_transport::MockCall::CancelLinkOpen { .. }
        )));
    }

    #[tokio::test(start_paused = true)]
    async fn link_open_cancellation_waits_for_cleanup_and_surfaces_failure() {
        let transport = Arc::new(MockTransport::new_default());
        let service =
            NetworkOperationService::new(transport.clone(), Arc::new(EventService::new()));
        let destination = AddressHash::new([7; 16]);
        let link_id = AddressHash::new([8; 16]);
        transport.queue_open_link(Ok(link_id));
        transport
            .queue_cancel_open(Err(TransportError::CleanupFailed("retained pending link".into())));
        let mut open = request(NetworkOperationKind::LinkOpen);
        open.destination_hash = Some(hex::encode(destination.as_slice()));
        let open = service.start(open).unwrap();
        transport.wait_for_calls(1, is_open_link).await;

        let cancelled = service.cancel(&open.operation_id).await.unwrap();

        assert_eq!(cancelled.outcome, Some(NetworkOperationOutcome::Failed));
        assert!(
            cancelled
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("retained pending link"))
        );
        assert!(transport.calls().iter().any(|call| matches!(
            call,
            crate::transport::mock_transport::MockCall::CancelLinkOpen { link_id: observed }
                if *observed == link_id
        )));

        let transport = Arc::new(MockTransport::new_default());
        let service =
            NetworkOperationService::new(transport.clone(), Arc::new(EventService::new()));
        transport.queue_open_link(Ok(link_id));
        transport.queue_cancel_open(Err(TransportError::CleanupFailed("timeout cleanup".into())));
        let mut open = request(NetworkOperationKind::LinkOpen);
        open.destination_hash = Some(hex::encode(destination.as_slice()));
        let open = service.start(open).unwrap();
        transport.wait_for_calls(1, is_open_link).await;
        tokio::time::advance(Duration::from_millis(100)).await;
        terminal(&service, &open.operation_id).await;

        let terminal = service.get(&open.operation_id).unwrap();
        assert_eq!(terminal.outcome, Some(NetworkOperationOutcome::Failed));
        assert!(
            terminal.detail.as_deref().is_some_and(|detail| detail.contains("timeout cleanup"))
        );
    }
}
