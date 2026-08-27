use crate::services::MessagingService;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::{AbortHandle, JoinHandle};
use tokio_util::sync::CancellationToken;

pub struct StandardPropagationSyncWorker {
    cancellation: CancellationToken,
    task: JoinHandle<()>,
}

impl StandardPropagationSyncWorker {
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

pub fn spawn_standard_propagation_sync_worker(
    messaging: Arc<MessagingService>,
) -> StandardPropagationSyncWorker {
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let task = tokio::spawn(async move {
        let mut delay = Duration::from_secs(30);
        loop {
            if worker_cancellation.is_cancelled() {
                break;
            }
            let outbound = messaging
                .resume_standard_propagation_outbound_once(worker_cancellation.clone())
                .await;
            if worker_cancellation.is_cancelled() {
                break;
            }
            let inbound = messaging
                .sync_standard_propagation_once(
                    std::time::Instant::now() + Duration::from_secs(32),
                    worker_cancellation.clone(),
                )
                .await;
            delay = if outbound.is_ok() && inbound.is_ok() {
                Duration::from_secs(30)
            } else {
                (delay * 2).min(Duration::from_secs(5 * 60))
            };
            tokio::select! {
                () = worker_cancellation.cancelled() => break,
                () = tokio::time::sleep(delay) => {}
            }
        }
    });
    StandardPropagationSyncWorker { cancellation, task }
}
