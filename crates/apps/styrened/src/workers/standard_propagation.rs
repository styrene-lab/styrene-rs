use crate::services::MessagingService;
use crate::transport::mesh_transport::{TransportError, TransportLifecycleEvent};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::{AbortHandle, JoinHandle};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardPropagationSyncTriggerKind {
    InitialConnection,
    Reconnect,
    ForegroundOpportunity,
    Periodic,
    Manual,
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
}

pub struct StandardPropagationSyncWorker {
    cancellation: CancellationToken,
    trigger: StandardPropagationSyncTrigger,
    policy: StandardPropagationSyncPolicy,
    task: JoinHandle<()>,
}

impl StandardPropagationSyncWorker {
    pub fn trigger(&self) -> StandardPropagationSyncTrigger {
        self.trigger.clone()
    }

    pub fn policy(&self) -> StandardPropagationSyncPolicy {
        self.policy
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
) -> StandardPropagationSyncWorker {
    spawn_standard_propagation_sync_worker_with_policy(
        messaging,
        lifecycle,
        initially_connected,
        StandardPropagationSyncPolicy::default(),
    )
}

fn spawn_standard_propagation_sync_worker_with_policy(
    messaging: Arc<MessagingService>,
    mut lifecycle: broadcast::Receiver<TransportLifecycleEvent>,
    initially_connected: bool,
    policy: StandardPropagationSyncPolicy,
) -> StandardPropagationSyncWorker {
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let (command_tx, mut command_rx) = mpsc::channel::<SyncCommand>(8);
    let trigger = StandardPropagationSyncTrigger { sender: command_tx };
    let task = tokio::spawn(async move {
        let mut state = SyncScheduleState::new(policy);
        let initial_delay = if initially_connected { Duration::ZERO } else { policy.cooldown };
        let mut next_periodic = tokio::time::Instant::now() + initial_delay;
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
                () = tokio::time::sleep_until(next_periodic) => SyncCommand {
                    trigger: if state.last_started.is_none() {
                        StandardPropagationSyncTriggerKind::InitialConnection
                    } else {
                        StandardPropagationSyncTriggerKind::Periodic
                    },
                    deadline: None,
                    response: None,
                },
            };

            let decision = state.request(command.trigger, Instant::now());
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
                next_periodic = tokio::time::Instant::from_std(state.last_started.map_or_else(
                    || Instant::now() + policy.cooldown,
                    |started| started + policy.cooldown,
                ));
                continue;
            };
            let deadline = command.deadline.unwrap_or(policy_deadline).min(policy.deadline);
            let result = run_sync(&messaging, deadline, worker_cancellation.clone()).await;
            state.finish();
            next_periodic = tokio::time::Instant::now() + policy.cooldown;
            if let Some(response) = command.response {
                let _ = response.send(result);
            }
        }
    });
    StandardPropagationSyncWorker { cancellation, trigger, policy, task }
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
    use std::sync::Mutex;

    fn policy() -> StandardPropagationSyncPolicy {
        StandardPropagationSyncPolicy {
            automatic: true,
            cooldown: Duration::from_secs(30),
            deadline: Duration::from_secs(12),
        }
    }

    #[test]
    fn initial_reconnect_and_foreground_triggers_share_cooldown_and_deadline() {
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

    #[tokio::test]
    async fn worker_shutdown_cancels_an_in_flight_initial_sync() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let peer = PrivateIdentity::new_from_name("scheduler cancellation peer");
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
            Arc::new(PrivateIdentity::new_from_name("scheduler cancellation local")),
        );
        let mut worker = spawn_standard_propagation_sync_worker_with_policy(
            messaging,
            transport.subscribe_lifecycle(),
            false,
            StandardPropagationSyncPolicy { deadline: Duration::from_secs(60), ..policy() },
        );
        transport.inject_lifecycle(TransportLifecycleEvent::Connected);
        transport.wait_for_calls(1, |call| matches!(call, MockCall::ResolveIdentity { .. })).await;

        tokio::time::timeout(Duration::from_secs(1), worker.shutdown())
            .await
            .expect("scheduler shutdown did not cancel the in-flight sync");
        assert!(worker.is_finished());
        assert!(transport.calls().iter().any(|call| {
            matches!(call, MockCall::RequestPath { dest } if *dest == AddressHash::new(destination.as_slice().try_into().unwrap()))
        }));
    }
}
