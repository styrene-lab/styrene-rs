use crate::services::{EventService, MessagingService};
use crate::transport::mesh_transport::{TransportError, TransportLifecycleEvent};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::{AbortHandle, JoinHandle};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardPropagationSyncTriggerKind {
    InitialConnection,
    Reconnect,
    ForegroundOpportunity,
    BackgroundOpportunity,
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardPropagationSyncTerminalOutcome {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardPropagationSyncActivity {
    pub trigger: StandardPropagationSyncTriggerKind,
    pub started_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardPropagationSyncCompletion {
    pub trigger: StandardPropagationSyncTriggerKind,
    pub started_at: i64,
    pub finished_at: i64,
    pub outcome: StandardPropagationSyncTerminalOutcome,
    pub new_messages: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StandardPropagationSyncTelemetry {
    pub active: Option<StandardPropagationSyncActivity>,
    pub last_completed: Option<StandardPropagationSyncCompletion>,
    pub cooldown_remaining: Duration,
}

#[derive(Default)]
struct SyncTelemetryState {
    active: Option<StandardPropagationSyncActivity>,
    last_completed: Option<StandardPropagationSyncCompletion>,
    last_started: Option<Instant>,
}

impl SyncTelemetryState {
    fn start(
        &mut self,
        trigger: StandardPropagationSyncTriggerKind,
        started: Instant,
        started_at: i64,
    ) {
        self.last_started = Some(started);
        self.active = Some(StandardPropagationSyncActivity { trigger, started_at });
    }

    fn finish(&mut self, result: &Result<usize, TransportError>, finished_at: i64) {
        let Some(active) = self.active.take() else {
            return;
        };
        let (outcome, new_messages) = match result {
            Ok(count) => (StandardPropagationSyncTerminalOutcome::Succeeded, *count),
            Err(TransportError::TimedOut) => (StandardPropagationSyncTerminalOutcome::TimedOut, 0),
            Err(TransportError::Cancelled) => {
                (StandardPropagationSyncTerminalOutcome::Cancelled, 0)
            }
            Err(_) => (StandardPropagationSyncTerminalOutcome::Failed, 0),
        };
        self.last_completed = Some(StandardPropagationSyncCompletion {
            trigger: active.trigger,
            started_at: active.started_at,
            finished_at,
            outcome,
            new_messages,
        });
    }

    fn snapshot(
        &self,
        now: Instant,
        policy: StandardPropagationSyncPolicy,
    ) -> StandardPropagationSyncTelemetry {
        let cooldown_remaining = self.last_started.map_or(Duration::ZERO, |started| {
            policy.cooldown.saturating_sub(now.saturating_duration_since(started))
        });
        StandardPropagationSyncTelemetry {
            active: self.active,
            last_completed: self.last_completed,
            cooldown_remaining,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardPropagationSyncPolicy {
    pub automatic: bool,
    pub cooldown: Duration,
    pub deadline: Duration,
}

impl Default for StandardPropagationSyncPolicy {
    fn default() -> Self {
        Self {
            automatic: true,
            cooldown: Duration::from_secs(30),
            deadline: Duration::from_secs(32),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScheduleDecision {
    Start { deadline: Duration },
    Disabled,
    InFlight,
    CoolingDown,
}

struct SyncScheduleState {
    policy: StandardPropagationSyncPolicy,
    in_flight: bool,
    last_started: Option<Instant>,
}

impl SyncScheduleState {
    fn new(policy: StandardPropagationSyncPolicy) -> Self {
        Self { policy, in_flight: false, last_started: None }
    }

    fn request(
        &mut self,
        trigger: StandardPropagationSyncTriggerKind,
        now: Instant,
    ) -> ScheduleDecision {
        if self.in_flight {
            return ScheduleDecision::InFlight;
        }
        let automatic = trigger != StandardPropagationSyncTriggerKind::Manual;
        if automatic && !self.policy.automatic {
            return ScheduleDecision::Disabled;
        }
        if automatic
            && self.last_started.is_some_and(|started| {
                now.saturating_duration_since(started) < self.policy.cooldown
            })
        {
            return ScheduleDecision::CoolingDown;
        }
        self.in_flight = true;
        self.last_started = Some(now);
        ScheduleDecision::Start { deadline: self.policy.deadline }
    }

    fn finish(&mut self) {
        self.in_flight = false;
    }
}

struct SyncCommand {
    trigger: StandardPropagationSyncTriggerKind,
    deadline: Option<Duration>,
    response: Option<oneshot::Sender<Result<usize, TransportError>>>,
}

#[derive(Clone)]
pub struct StandardPropagationSyncTrigger {
    sender: mpsc::Sender<SyncCommand>,
}

impl StandardPropagationSyncTrigger {
    pub async fn manual(&self, deadline: Duration) -> Result<usize, TransportError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.sender
            .send(SyncCommand {
                trigger: StandardPropagationSyncTriggerKind::Manual,
                deadline: Some(deadline),
                response: Some(response_tx),
            })
            .await
            .map_err(|_| TransportError::Unavailable)?;
        response_rx.await.map_err(|_| TransportError::Cancelled)?
    }

    pub fn foreground_opportunity(&self) -> bool {
        self.sender
            .try_send(SyncCommand {
                trigger: StandardPropagationSyncTriggerKind::ForegroundOpportunity,
                deadline: None,
                response: None,
            })
            .is_ok()
    }

    pub fn background_opportunity(&self) -> bool {
        self.sender
            .try_send(SyncCommand {
                trigger: StandardPropagationSyncTriggerKind::BackgroundOpportunity,
                deadline: None,
                response: None,
            })
            .is_ok()
    }
}

pub struct StandardPropagationSyncWorker {
    cancellation: CancellationToken,
    trigger: StandardPropagationSyncTrigger,
    policy: StandardPropagationSyncPolicy,
    telemetry: Arc<Mutex<SyncTelemetryState>>,
    task: JoinHandle<()>,
}

#[derive(Clone)]
pub struct StandardPropagationSyncObservation {
    policy: StandardPropagationSyncPolicy,
    telemetry: Arc<Mutex<SyncTelemetryState>>,
}

impl StandardPropagationSyncObservation {
    pub fn policy(&self) -> StandardPropagationSyncPolicy {
        self.policy
    }

    pub fn telemetry(&self) -> StandardPropagationSyncTelemetry {
        self.telemetry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .snapshot(Instant::now(), self.policy)
    }
}

impl StandardPropagationSyncWorker {
    pub fn trigger(&self) -> StandardPropagationSyncTrigger {
        self.trigger.clone()
    }

    pub fn policy(&self) -> StandardPropagationSyncPolicy {
        self.policy
    }

    pub fn telemetry(&self) -> StandardPropagationSyncTelemetry {
        self.telemetry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .snapshot(Instant::now(), self.policy)
    }

    pub fn observation(&self) -> StandardPropagationSyncObservation {
        StandardPropagationSyncObservation {
            policy: self.policy,
            telemetry: Arc::clone(&self.telemetry),
        }
    }

    pub fn abort(&self) {
        self.task.abort();
    }

    pub async fn shutdown(&mut self) {
        self.cancellation.cancel();
        let _ = (&mut self.task).await;
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    pub fn abort_handle(&self) -> AbortHandle {
        self.task.abort_handle()
    }
}

async fn run_sync(
    messaging: &MessagingService,
    deadline: Duration,
    cancellation: CancellationToken,
) -> Result<usize, TransportError> {
    let operation_cancellation = cancellation.child_token();
    let worker_cancellation = operation_cancellation.clone();
    let mut operation = Box::pin(async move {
        let outbound =
            messaging.resume_standard_propagation_outbound_once(worker_cancellation.clone()).await;
        let inbound = messaging
            .sync_standard_propagation_once(Instant::now() + deadline, worker_cancellation)
            .await;
        match (outbound, inbound) {
            (Ok(_), Ok(count)) => Ok(count),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    });
    tokio::select! {
        result = &mut operation => result,
        () = cancellation.cancelled() => {
            operation_cancellation.cancel();
            let _ = operation.await;
            Err(TransportError::Cancelled)
        }
        () = tokio::time::sleep(deadline) => {
            operation_cancellation.cancel();
            let _ = operation.await;
            Err(TransportError::TimedOut)
        }
    }
}

pub fn spawn_standard_propagation_sync_worker(
    messaging: Arc<MessagingService>,
    lifecycle: broadcast::Receiver<TransportLifecycleEvent>,
    initially_connected: bool,
    events: Arc<EventService>,
) -> StandardPropagationSyncWorker {
    spawn_standard_propagation_sync_worker_with_policy_and_events(
        messaging,
        lifecycle,
        initially_connected,
        StandardPropagationSyncPolicy::default(),
        Some(events),
    )
}

#[cfg(test)]
fn spawn_standard_propagation_sync_worker_with_policy(
    messaging: Arc<MessagingService>,
    lifecycle: broadcast::Receiver<TransportLifecycleEvent>,
    initially_connected: bool,
    policy: StandardPropagationSyncPolicy,
) -> StandardPropagationSyncWorker {
    spawn_standard_propagation_sync_worker_with_policy_and_events(
        messaging,
        lifecycle,
        initially_connected,
        policy,
        None,
    )
}

fn spawn_standard_propagation_sync_worker_with_policy_and_events(
    messaging: Arc<MessagingService>,
    mut lifecycle: broadcast::Receiver<TransportLifecycleEvent>,
    initially_connected: bool,
    policy: StandardPropagationSyncPolicy,
    events: Option<Arc<EventService>>,
) -> StandardPropagationSyncWorker {
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let retained_completion = match messaging.standard_propagation_sync_telemetry() {
        Ok(Some(value)) => match serde_json::from_value(value) {
            Ok(completion) => Some(completion),
            Err(error) => {
                eprintln!("[standard-propagation] invalid retained sync telemetry: {error}");
                None
            }
        },
        Ok(None) => None,
        Err(error) => {
            eprintln!("[standard-propagation] load retained sync telemetry failed: {error}");
            None
        }
    };
    let telemetry = Arc::new(Mutex::new(SyncTelemetryState {
        last_completed: retained_completion,
        ..SyncTelemetryState::default()
    }));
    let worker_telemetry = telemetry.clone();
    let (command_tx, mut command_rx) = mpsc::channel::<SyncCommand>(8);
    if initially_connected {
        let _ = command_tx.try_send(SyncCommand {
            trigger: StandardPropagationSyncTriggerKind::InitialConnection,
            deadline: None,
            response: None,
        });
    }
    let trigger = StandardPropagationSyncTrigger { sender: command_tx };
    let task = tokio::spawn(async move {
        let mut state = SyncScheduleState::new(policy);
        loop {
            let command = tokio::select! {
                biased;
                () = worker_cancellation.cancelled() => break,
                command = command_rx.recv() => match command {
                    Some(command) => command,
                    None => break,
                },
                event = lifecycle.recv() => {
                    let trigger = match event {
                        Ok(TransportLifecycleEvent::Connected) => {
                            StandardPropagationSyncTriggerKind::InitialConnection
                        }
                        Ok(TransportLifecycleEvent::Reconnected) => {
                            StandardPropagationSyncTriggerKind::Reconnect
                        }
                        Ok(_) => continue,
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            StandardPropagationSyncTriggerKind::Reconnect
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    };
                    SyncCommand { trigger, deadline: None, response: None }
                }
            };

            let started = Instant::now();
            let decision = state.request(command.trigger, started);
            let ScheduleDecision::Start { deadline: policy_deadline } = decision else {
                if let Some(response) = command.response {
                    let error = match decision {
                        ScheduleDecision::InFlight => TransportError::SendFailed(
                            "propagation synchronization already in flight".into(),
                        ),
                        ScheduleDecision::Disabled | ScheduleDecision::CoolingDown => {
                            TransportError::Unavailable
                        }
                        ScheduleDecision::Start { .. } => unreachable!(),
                    };
                    let _ = response.send(Err(error));
                }
                continue;
            };
            let deadline = command.deadline.unwrap_or(policy_deadline).min(policy.deadline);
            let started_at = rns_core::transport::time::now_epoch_secs_i64();
            worker_telemetry.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).start(
                command.trigger,
                started,
                started_at,
            );
            if let Some(events) = &events {
                events.emit_standard_propagation_changed(started_at);
            }
            let result = run_sync(&messaging, deadline, worker_cancellation.clone()).await;
            state.finish();
            let finished_at = rns_core::transport::time::now_epoch_secs_i64();
            let completed = {
                let mut telemetry =
                    worker_telemetry.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                telemetry.finish(&result, finished_at);
                telemetry.last_completed
            };
            if let Some(completed) = completed {
                match serde_json::to_value(completed) {
                    Ok(value) => {
                        if let Err(error) =
                            messaging.retain_standard_propagation_sync_telemetry(&value)
                        {
                            eprintln!(
                                "[standard-propagation] retain sync telemetry failed: {error}"
                            );
                        }
                    }
                    Err(error) => {
                        eprintln!("[standard-propagation] encode sync telemetry failed: {error}");
                    }
                }
            }
            if let Some(events) = &events {
                events.emit_standard_propagation_changed(finished_at);
            }
            if let Some(response) = command.response {
                let _ = response.send(result);
            }
        }
    });
    StandardPropagationSyncWorker { cancellation, trigger, policy, telemetry, task }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::messages::MessagesStore;
    use crate::storage::standard_propagation::StandardPropagationPeer;
    use crate::transport::mesh_transport::MeshTransport;
    use crate::transport::mock_transport::{MockCall, MockTransport};
    use rns_core::destination::{DestinationName, SingleOutputDestination};
    use rns_core::hash::AddressHash;
    use rns_core::identity::PrivateIdentity;

    fn policy() -> StandardPropagationSyncPolicy {
        StandardPropagationSyncPolicy {
            automatic: true,
            cooldown: Duration::from_secs(30),
            deadline: Duration::from_secs(12),
        }
    }

    fn scheduler_fixture() -> (Arc<MessagingService>, Arc<MockTransport>, PrivateIdentity) {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let peer = PrivateIdentity::new_from_name("scheduler peer");
        let destination = SingleOutputDestination::new(
            *peer.as_identity(),
            DestinationName::new("lxmf", "propagation"),
        )
        .desc
        .address_hash;
        let mut peer_hash = [0; 16];
        peer_hash.copy_from_slice(peer.address_hash().as_slice());
        store
            .lock()
            .unwrap()
            .standard_propagation_upsert_peer(&StandardPropagationPeer {
                identity_hash: peer_hash,
                propagation_destination: Some(destination.as_slice().try_into().unwrap()),
                configured: false,
                enabled: true,
                transfer_limit_kb: Some(256),
                sync_limit_kb: Some(4_000),
                stamp_cost: Some(0),
                stamp_flexibility: Some(0),
                peering_cost: Some(0),
                observed_at: 1,
            })
            .unwrap();
        store
            .lock()
            .unwrap()
            .standard_propagation_set_selection(Some(peer_hash), "manual", 1)
            .unwrap();
        let messaging = Arc::new(MessagingService::with_store(store));
        let transport = Arc::new(MockTransport::new_default());
        messaging.set_signer(
            transport.clone(),
            Arc::new(PrivateIdentity::new_from_name("scheduler local")),
        );
        (messaging, transport, peer)
    }

    async fn completed_trigger(
        worker: &StandardPropagationSyncWorker,
    ) -> StandardPropagationSyncTriggerKind {
        for _ in 0..100 {
            if let Some(completed) = worker.telemetry().last_completed {
                return completed.trigger;
            }
            tokio::task::yield_now().await;
        }
        panic!("propagation synchronization did not publish terminal telemetry");
    }

    #[test]
    fn automatic_lifecycle_triggers_share_cooldown_and_deadline() {
        let start = Instant::now();
        let mut state = SyncScheduleState::new(policy());

        assert_eq!(
            state.request(StandardPropagationSyncTriggerKind::InitialConnection, start),
            ScheduleDecision::Start { deadline: Duration::from_secs(12) }
        );
        state.finish();
        assert_eq!(
            state.request(
                StandardPropagationSyncTriggerKind::Reconnect,
                start + Duration::from_secs(1)
            ),
            ScheduleDecision::CoolingDown
        );
        assert_eq!(
            state.request(
                StandardPropagationSyncTriggerKind::ForegroundOpportunity,
                start + Duration::from_secs(30),
            ),
            ScheduleDecision::Start { deadline: Duration::from_secs(12) }
        );
        state.finish();
        assert_eq!(
            state.request(
                StandardPropagationSyncTriggerKind::BackgroundOpportunity,
                start + Duration::from_secs(60),
            ),
            ScheduleDecision::Start { deadline: Duration::from_secs(12) }
        );
    }

    #[test]
    fn overlapping_triggers_are_single_flight_and_manual_bypasses_cooldown() {
        let start = Instant::now();
        let mut state = SyncScheduleState::new(policy());

        assert!(matches!(
            state.request(StandardPropagationSyncTriggerKind::Manual, start),
            ScheduleDecision::Start { .. }
        ));
        assert_eq!(
            state.request(StandardPropagationSyncTriggerKind::Reconnect, start),
            ScheduleDecision::InFlight
        );
        state.finish();
        assert!(matches!(
            state.request(StandardPropagationSyncTriggerKind::Manual, start),
            ScheduleDecision::Start { .. }
        ));
    }

    #[test]
    fn disabled_automatic_policy_still_allows_manual_sync() {
        let start = Instant::now();
        let mut disabled = policy();
        disabled.automatic = false;
        let mut state = SyncScheduleState::new(disabled);

        assert_eq!(
            state.request(StandardPropagationSyncTriggerKind::InitialConnection, start),
            ScheduleDecision::Disabled
        );
        assert!(matches!(
            state.request(StandardPropagationSyncTriggerKind::Manual, start),
            ScheduleDecision::Start { .. }
        ));
    }

    #[test]
    fn telemetry_records_only_started_syncs_and_preserves_terminal_result() {
        let started = Instant::now();
        let mut schedule = SyncScheduleState::new(policy());
        let mut telemetry = SyncTelemetryState::default();

        assert_eq!(
            schedule.request(StandardPropagationSyncTriggerKind::InitialConnection, started),
            ScheduleDecision::Start { deadline: Duration::from_secs(12) }
        );
        telemetry.start(StandardPropagationSyncTriggerKind::InitialConnection, started, 100);
        telemetry.finish(&Ok(3), 104);
        schedule.finish();

        assert_eq!(
            schedule.request(
                StandardPropagationSyncTriggerKind::Reconnect,
                started + Duration::from_secs(5)
            ),
            ScheduleDecision::CoolingDown
        );
        assert_eq!(
            telemetry.snapshot(started + Duration::from_secs(5), policy()),
            StandardPropagationSyncTelemetry {
                active: None,
                last_completed: Some(StandardPropagationSyncCompletion {
                    trigger: StandardPropagationSyncTriggerKind::InitialConnection,
                    started_at: 100,
                    finished_at: 104,
                    outcome: StandardPropagationSyncTerminalOutcome::Succeeded,
                    new_messages: 3,
                }),
                cooldown_remaining: Duration::from_secs(25),
            }
        );
    }

    #[test]
    fn telemetry_classifies_terminal_transport_outcomes() {
        let started = Instant::now();
        for (error, expected) in [
            (TransportError::TimedOut, StandardPropagationSyncTerminalOutcome::TimedOut),
            (TransportError::Cancelled, StandardPropagationSyncTerminalOutcome::Cancelled),
            (TransportError::Unavailable, StandardPropagationSyncTerminalOutcome::Failed),
        ] {
            let mut telemetry = SyncTelemetryState::default();
            telemetry.start(StandardPropagationSyncTriggerKind::Manual, started, 200);
            telemetry.finish(&Err(error), 201);
            assert_eq!(telemetry.last_completed.unwrap().outcome, expected);
        }
    }

    #[tokio::test]
    async fn shutdown_cancellation_interrupts_deadline_wait() {
        let cancellation = CancellationToken::new();
        let worker = cancellation.clone();
        let wait = tokio::spawn(async move {
            tokio::select! {
                () = worker.cancelled() => "cancelled",
                () = tokio::time::sleep(Duration::from_secs(60)) => "deadline",
            }
        });

        cancellation.cancel();
        assert_eq!(wait.await.unwrap(), "cancelled");
    }

    #[tokio::test(start_paused = true)]
    async fn wall_clock_passage_does_not_start_mobile_sync() {
        let (messaging, transport, _) = scheduler_fixture();
        let mut worker = spawn_standard_propagation_sync_worker_with_policy(
            messaging,
            transport.subscribe_lifecycle(),
            false,
            policy(),
        );

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(300)).await;
        tokio::task::yield_now().await;

        assert!(transport.calls().is_empty(), "wall-clock passage started propagation sync");
        worker.shutdown().await;
    }

    #[tokio::test]
    async fn initially_connected_worker_starts_one_sync() {
        let (messaging, transport, _) = scheduler_fixture();
        let mut worker = spawn_standard_propagation_sync_worker_with_policy(
            messaging,
            transport.subscribe_lifecycle(),
            true,
            policy(),
        );

        transport.wait_for_calls(1, |call| matches!(call, MockCall::ResolveIdentity { .. })).await;

        assert_eq!(
            transport
                .calls()
                .iter()
                .filter(|call| matches!(call, MockCall::ResolveIdentity { .. }))
                .count(),
            1
        );
        assert_eq!(
            completed_trigger(&worker).await,
            StandardPropagationSyncTriggerKind::InitialConnection
        );
        worker.shutdown().await;
    }

    #[tokio::test]
    async fn sync_start_and_completion_notify_mobile_observers() {
        let (messaging, transport, _) = scheduler_fixture();
        let events = Arc::new(EventService::new());
        let mut receiver = events.subscribe_daemon_events();
        let mut worker = spawn_standard_propagation_sync_worker_with_policy_and_events(
            messaging,
            transport.subscribe_lifecycle(),
            true,
            policy(),
            Some(events),
        );

        assert_eq!(
            completed_trigger(&worker).await,
            StandardPropagationSyncTriggerKind::InitialConnection
        );
        for expected_phase in ["start", "completion"] {
            assert!(
                matches!(
                    receiver.try_recv(),
                    Ok(styrene_ipc::types::DaemonEvent::StandardPropagationChanged { .. })
                ),
                "missing propagation {expected_phase} notification"
            );
        }
        worker.shutdown().await;
    }

    #[tokio::test]
    async fn reconnect_starts_one_sync() {
        let (messaging, transport, _) = scheduler_fixture();
        let mut worker = spawn_standard_propagation_sync_worker_with_policy(
            messaging,
            transport.subscribe_lifecycle(),
            false,
            policy(),
        );

        transport.inject_lifecycle(TransportLifecycleEvent::Reconnected);
        transport.wait_for_calls(1, |call| matches!(call, MockCall::ResolveIdentity { .. })).await;

        assert_eq!(
            transport
                .calls()
                .iter()
                .filter(|call| matches!(call, MockCall::ResolveIdentity { .. }))
                .count(),
            1
        );
        assert_eq!(completed_trigger(&worker).await, StandardPropagationSyncTriggerKind::Reconnect);
        worker.shutdown().await;
    }

    #[tokio::test]
    async fn foreground_opportunity_starts_one_sync() {
        let (messaging, transport, _) = scheduler_fixture();
        let mut worker = spawn_standard_propagation_sync_worker_with_policy(
            messaging,
            transport.subscribe_lifecycle(),
            false,
            policy(),
        );

        assert!(worker.trigger().foreground_opportunity());
        transport.wait_for_calls(1, |call| matches!(call, MockCall::ResolveIdentity { .. })).await;

        assert_eq!(
            transport
                .calls()
                .iter()
                .filter(|call| matches!(call, MockCall::ResolveIdentity { .. }))
                .count(),
            1
        );
        assert_eq!(
            completed_trigger(&worker).await,
            StandardPropagationSyncTriggerKind::ForegroundOpportunity
        );
        worker.shutdown().await;
    }

    #[tokio::test]
    async fn granted_background_opportunity_starts_one_sync() {
        let (messaging, transport, _) = scheduler_fixture();
        let mut worker = spawn_standard_propagation_sync_worker_with_policy(
            messaging,
            transport.subscribe_lifecycle(),
            false,
            policy(),
        );

        assert!(worker.trigger().background_opportunity());
        transport.wait_for_calls(1, |call| matches!(call, MockCall::ResolveIdentity { .. })).await;

        assert_eq!(
            transport
                .calls()
                .iter()
                .filter(|call| matches!(call, MockCall::ResolveIdentity { .. }))
                .count(),
            1
        );
        assert_eq!(
            completed_trigger(&worker).await,
            StandardPropagationSyncTriggerKind::BackgroundOpportunity
        );
        worker.shutdown().await;
    }

    #[tokio::test]
    async fn manual_sync_publishes_manual_trigger_telemetry() {
        let (messaging, transport, _) = scheduler_fixture();
        let mut worker = spawn_standard_propagation_sync_worker_with_policy(
            messaging,
            transport.subscribe_lifecycle(),
            false,
            policy(),
        );

        let _ = worker.trigger().manual(policy().deadline).await;

        assert_eq!(completed_trigger(&worker).await, StandardPropagationSyncTriggerKind::Manual);
        worker.shutdown().await;
    }

    #[tokio::test]
    async fn automatic_trigger_during_cooldown_does_not_start_another_sync() {
        let (messaging, transport, _) = scheduler_fixture();
        let mut worker = spawn_standard_propagation_sync_worker_with_policy(
            messaging,
            transport.subscribe_lifecycle(),
            true,
            StandardPropagationSyncPolicy { cooldown: Duration::MAX, ..policy() },
        );
        let trigger = worker.trigger();
        transport.wait_for_calls(1, |call| matches!(call, MockCall::ResolveIdentity { .. })).await;

        assert!(trigger.foreground_opportunity());
        let _ = trigger.manual(policy().deadline).await;

        assert_eq!(
            transport
                .calls()
                .iter()
                .filter(|call| matches!(call, MockCall::ResolveIdentity { .. }))
                .count(),
            2,
            "the automatic cooldown trigger ran before the manual barrier"
        );
        worker.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn overlapping_opportunity_remains_single_flight() {
        let (messaging, transport, peer) = scheduler_fixture();
        transport.queue_resolve(Some(*peer.as_identity()));
        transport.queue_open_link(Ok(AddressHash::new([7; 16])));
        let mut worker = spawn_standard_propagation_sync_worker_with_policy(
            messaging,
            transport.subscribe_lifecycle(),
            true,
            policy(),
        );
        let trigger = worker.trigger();
        transport.wait_for_calls(1, |call| matches!(call, MockCall::StartRequest { .. })).await;

        assert!(trigger.foreground_opportunity());
        tokio::task::yield_now().await;

        assert_eq!(
            transport
                .calls()
                .iter()
                .filter(|call| matches!(call, MockCall::StartRequest { .. }))
                .count(),
            1
        );
        worker.shutdown().await;
    }

    #[tokio::test]
    async fn process_restart_retains_last_completion_but_resets_cooldown() {
        let (messaging, transport, _) = scheduler_fixture();
        let mut first = spawn_standard_propagation_sync_worker_with_policy(
            messaging.clone(),
            transport.subscribe_lifecycle(),
            true,
            StandardPropagationSyncPolicy { cooldown: Duration::MAX, ..policy() },
        );
        transport.wait_for_calls(1, |call| matches!(call, MockCall::ResolveIdentity { .. })).await;
        assert_eq!(
            completed_trigger(&first).await,
            StandardPropagationSyncTriggerKind::InitialConnection
        );
        let retained = first.telemetry().last_completed.expect("completed first sync");
        first.shutdown().await;
        transport.clear_calls();

        let mut restarted = spawn_standard_propagation_sync_worker_with_policy(
            messaging,
            transport.subscribe_lifecycle(),
            false,
            StandardPropagationSyncPolicy { cooldown: Duration::MAX, ..policy() },
        );
        assert_eq!(restarted.telemetry().last_completed, Some(retained));
        assert_eq!(restarted.telemetry().cooldown_remaining, Duration::ZERO);

        assert!(restarted.trigger().foreground_opportunity());
        transport.wait_for_calls(1, |call| matches!(call, MockCall::ResolveIdentity { .. })).await;

        assert_eq!(
            transport
                .calls()
                .iter()
                .filter(|call| matches!(call, MockCall::ResolveIdentity { .. }))
                .count(),
            1
        );
        restarted.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn worker_shutdown_cancels_an_in_flight_initial_sync() {
        let (messaging, transport, peer) = scheduler_fixture();
        transport.queue_resolve(Some(*peer.as_identity()));
        transport.queue_open_link(Ok(AddressHash::new([9; 16])));
        let mut worker = spawn_standard_propagation_sync_worker_with_policy(
            messaging,
            transport.subscribe_lifecycle(),
            true,
            StandardPropagationSyncPolicy { deadline: Duration::from_secs(60), ..policy() },
        );
        transport.wait_for_calls(1, |call| matches!(call, MockCall::StartRequest { .. })).await;

        worker.shutdown().await;

        assert!(worker.is_finished());
        assert!(
            transport.calls().iter().any(|call| matches!(call, MockCall::CancelRequest { .. }))
        );
    }
}
