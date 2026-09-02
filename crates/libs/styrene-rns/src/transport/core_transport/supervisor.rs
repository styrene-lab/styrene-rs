//! Supervision of the transport's long-lived worker tasks.
//!
//! Every worker the transport runtime spawns is retained here under a stable
//! name. The supervisor runs until the runtime cancellation token fires or a
//! worker completes early. An early completion, whether a silent return or a
//! panic, is attributed to its worker, cancels the remaining set, and drains
//! it within a bound so no sibling outlives the failure unobserved. Ordinary
//! cancellation is drained the same way and is never reported as a failure.

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use tokio::task::{Id, JoinError, JoinSet};
use tokio_util::sync::CancellationToken;

/// How a supervised worker left before shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerExit {
    /// The worker future completed on its own.
    Returned,
    /// The worker panicked; the panic was caught at the task boundary.
    Panicked,
    /// The worker task was aborted from outside the supervisor.
    Aborted,
}

/// An attributable early completion of one named worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerFailure {
    pub worker: &'static str,
    pub exit: WorkerExit,
}

/// Why supervision ended and whether the worker set was fully drained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisionOutcome {
    /// The cancellation token fired while every worker was healthy.
    Shutdown { drained: bool },
    /// One worker completed before shutdown; siblings were cancelled.
    WorkerFailed { failure: WorkerFailure, drained: bool },
}

impl SupervisionOutcome {
    pub fn failure(&self) -> Option<WorkerFailure> {
        match self {
            Self::Shutdown { .. } => None,
            Self::WorkerFailed { failure, .. } => Some(*failure),
        }
    }

    pub fn drained(&self) -> bool {
        match self {
            Self::Shutdown { drained } | Self::WorkerFailed { drained, .. } => *drained,
        }
    }
}

pub struct WorkerSupervisor {
    name: String,
    cancel: CancellationToken,
    workers: JoinSet<()>,
    names: HashMap<Id, &'static str>,
    drain_bound: Duration,
}

impl WorkerSupervisor {
    pub fn new(name: impl Into<String>, cancel: CancellationToken, drain_bound: Duration) -> Self {
        Self {
            name: name.into(),
            cancel,
            workers: JoinSet::new(),
            names: HashMap::new(),
            drain_bound,
        }
    }

    /// Spawn and retain one named worker.
    pub fn spawn<F>(&mut self, worker: &'static str, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let id = self.workers.spawn(future).id();
        self.names.insert(id, worker);
    }

    /// Run until cancellation or the first early completion, then drain.
    pub async fn supervise(mut self) -> SupervisionOutcome {
        loop {
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => {
                    let drained = self.drain().await;
                    return SupervisionOutcome::Shutdown { drained };
                }
                joined = self.workers.join_next_with_id(), if !self.workers.is_empty() => {
                    let Some(joined) = joined else { continue };
                    let (worker, exit) = self.classify(joined);
                    if self.cancel.is_cancelled() {
                        continue;
                    }
                    log::error!(
                        "tp({}): worker {} exited before shutdown ({:?}); cancelling siblings",
                        self.name,
                        worker,
                        exit
                    );
                    self.cancel.cancel();
                    let drained = self.drain().await;
                    return SupervisionOutcome::WorkerFailed {
                        failure: WorkerFailure { worker, exit },
                        drained,
                    };
                }
            }
        }
    }

    fn classify(&mut self, joined: Result<(Id, ()), JoinError>) -> (&'static str, WorkerExit) {
        match joined {
            Ok((id, ())) => (self.name_of(id), WorkerExit::Returned),
            Err(err) => {
                let exit = if err.is_panic() { WorkerExit::Panicked } else { WorkerExit::Aborted };
                (self.name_of(err.id()), exit)
            }
        }
    }

    fn name_of(&mut self, id: Id) -> &'static str {
        self.names.remove(&id).unwrap_or("unnamed")
    }

    /// Wait for every remaining worker within the drain bound, aborting any
    /// worker that ignores cancellation once the bound expires.
    async fn drain(&mut self) -> bool {
        let deadline = tokio::time::sleep(self.drain_bound);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                joined = self.workers.join_next_with_id() => {
                    match joined {
                        None => return true,
                        Some(Ok(_)) => {}
                        Some(Err(err)) if err.is_panic() => {
                            let worker = self.name_of(err.id());
                            log::warn!("tp({}): worker {} panicked during drain", self.name, worker);
                        }
                        Some(Err(_)) => {}
                    }
                }
                _ = &mut deadline => {
                    let stuck: Vec<&'static str> = self
                        .workers
                        .abort_all_and_names(&mut self.names);
                    log::error!(
                        "tp({}): workers {:?} ignored cancellation for {:?}; aborted",
                        self.name,
                        stuck,
                        self.drain_bound
                    );
                    return false;
                }
            }
        }
    }
}

trait AbortAllNamed {
    fn abort_all_and_names(&mut self, names: &mut HashMap<Id, &'static str>) -> Vec<&'static str>;
}

impl AbortAllNamed for JoinSet<()> {
    fn abort_all_and_names(&mut self, names: &mut HashMap<Id, &'static str>) -> Vec<&'static str> {
        self.abort_all();
        let mut stuck: Vec<&'static str> = names.drain().map(|(_, name)| name).collect();
        stuck.sort_unstable();
        stuck
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::time::timeout;

    const BOUND: Duration = Duration::from_millis(200);

    async fn cancel_aware(cancel: CancellationToken, saw_cancel: Arc<AtomicBool>) {
        cancel.cancelled().await;
        saw_cancel.store(true, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn silent_early_return_is_attributed_and_cancels_siblings() {
        let cancel = CancellationToken::new();
        let mut supervisor = WorkerSupervisor::new("t", cancel.clone(), BOUND);
        let sibling_saw_cancel = Arc::new(AtomicBool::new(false));
        supervisor.spawn("links", cancel_aware(cancel.clone(), sibling_saw_cancel.clone()));
        supervisor.spawn("packet", async {});

        let outcome = timeout(Duration::from_secs(1), supervisor.supervise())
            .await
            .expect("supervision ends after the early return");

        assert_eq!(
            outcome,
            SupervisionOutcome::WorkerFailed {
                failure: WorkerFailure { worker: "packet", exit: WorkerExit::Returned },
                drained: true,
            }
        );
        assert!(cancel.is_cancelled());
        assert!(sibling_saw_cancel.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn panic_is_attributed_to_the_named_worker() {
        let cancel = CancellationToken::new();
        let mut supervisor = WorkerSupervisor::new("t", cancel.clone(), BOUND);
        let sibling_saw_cancel = Arc::new(AtomicBool::new(false));
        supervisor.spawn("links", cancel_aware(cancel.clone(), sibling_saw_cancel.clone()));
        supervisor.spawn("cache", async { panic!("cache worker exploded") });

        let outcome = timeout(Duration::from_secs(1), supervisor.supervise())
            .await
            .expect("supervision ends after the panic");

        assert_eq!(
            outcome.failure(),
            Some(WorkerFailure { worker: "cache", exit: WorkerExit::Panicked })
        );
        assert!(outcome.drained());
        assert!(sibling_saw_cancel.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn normal_shutdown_drains_every_worker_without_a_failure() {
        let cancel = CancellationToken::new();
        let mut supervisor = WorkerSupervisor::new("t", cancel.clone(), BOUND);
        let flags: Vec<Arc<AtomicBool>> =
            (0..3).map(|_| Arc::new(AtomicBool::new(false))).collect();
        for (index, flag) in flags.iter().enumerate() {
            let name: &'static str = ["packet", "links", "scheduler"][index];
            supervisor.spawn(name, cancel_aware(cancel.clone(), flag.clone()));
        }
        let supervision = tokio::spawn(supervisor.supervise());
        tokio::task::yield_now().await;
        cancel.cancel();

        let outcome = timeout(Duration::from_secs(1), supervision)
            .await
            .expect("shutdown completes within the bound")
            .expect("supervisor task joins");

        assert_eq!(outcome, SupervisionOutcome::Shutdown { drained: true });
        assert!(flags.iter().all(|flag| flag.load(Ordering::SeqCst)));
    }

    #[tokio::test]
    async fn worker_completion_during_shutdown_is_not_a_failure() {
        let cancel = CancellationToken::new();
        let mut supervisor = WorkerSupervisor::new("t", cancel.clone(), BOUND);
        let release = CancellationToken::new();
        let gate = release.clone();
        supervisor.spawn("packet", async move { gate.cancelled().await });
        cancel.cancel();
        release.cancel();

        let outcome = timeout(Duration::from_secs(1), supervisor.supervise())
            .await
            .expect("shutdown completes");

        assert_eq!(outcome, SupervisionOutcome::Shutdown { drained: true });
    }

    #[tokio::test]
    async fn drain_bound_aborts_workers_that_ignore_cancellation() {
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let cancel = CancellationToken::new();
        let mut supervisor = WorkerSupervisor::new("t", cancel.clone(), Duration::from_millis(50));
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = DropFlag(dropped.clone());
        supervisor.spawn("stuck", async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        cancel.cancel();

        let outcome = timeout(Duration::from_secs(1), supervisor.supervise())
            .await
            .expect("drain bound expires");

        assert_eq!(outcome, SupervisionOutcome::Shutdown { drained: false });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(dropped.load(Ordering::SeqCst), "aborted worker must not survive supervision");
    }

    #[tokio::test]
    async fn empty_worker_set_waits_for_shutdown() {
        let cancel = CancellationToken::new();
        let supervisor = WorkerSupervisor::new("t", cancel.clone(), BOUND);
        let supervision = tokio::spawn(supervisor.supervise());
        tokio::task::yield_now().await;
        assert!(!supervision.is_finished());
        cancel.cancel();
        let outcome = timeout(Duration::from_secs(1), supervision)
            .await
            .expect("shutdown completes")
            .expect("supervisor task joins");
        assert_eq!(outcome, SupervisionOutcome::Shutdown { drained: true });
    }
}
