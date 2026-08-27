use crate::services::MessagingService;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

pub fn spawn_router_deadline_worker(messaging: Arc<MessagingService>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = messaging.reconcile_router_deadlines().await {
                crate::daemon_diagnostic!("[router] deadline reconciliation failed: {error}");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::messages::{MessageRecord, MessagesStore, OutboundRouteRecord};
    use crate::transport::mock_transport::{MockCall, MockTransport};
    use std::sync::Mutex;

    #[tokio::test]
    async fn scheduler_expires_persisted_route_added_after_startup() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let messaging = Arc::new(MessagingService::with_store(store.clone()));
        let message = MessageRecord {
            id: "scheduled-expiry".into(),
            source: "source".into(),
            destination: "destination".into(),
            title: String::new(),
            content: String::new(),
            timestamp: 1,
            direction: "out".into(),
            fields: None,
            receipt_status: Some("queued".into()),
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
            deadline_unix_ms: 0,
            state: "queued".into(),
            attempt_count: 0,
        };
        store.lock().unwrap().insert_outbound_message(&message, &route).unwrap();
        let worker = spawn_router_deadline_worker(messaging);

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if store.lock().unwrap().outbound_route(&message.id).unwrap().unwrap().state
                    == "expired"
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("router deadline worker did not reconcile persisted route");
        worker.abort();
        let _ = worker.await;
    }

    #[tokio::test]
    async fn scheduler_cancels_resource_before_persisting_expiry() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let messaging = Arc::new(MessagingService::with_store(store.clone()));
        let transport = Arc::new(MockTransport::new_default());
        messaging.set_signer(
            transport.clone(),
            Arc::new(rns_core::identity::PrivateIdentity::new_from_name("expiry-test")),
        );
        let message = MessageRecord {
            id: "resource-expiry".into(),
            source: "source".into(),
            destination: "destination".into(),
            title: String::new(),
            content: String::new(),
            timestamp: 1,
            direction: "out".into(),
            fields: None,
            receipt_status: Some("sent: direct".into()),
            read: true,
        };
        let resource_hash = rns_core::hash::Hash::new([0x77; 32]);
        store
            .lock()
            .unwrap()
            .insert_outbound_message(
                &message,
                &OutboundRouteRecord {
                    message_id: message.id.clone(),
                    requested_method: "direct".into(),
                    actual_method: "direct".into(),
                    representation: "resource".into(),
                    fallback_reason: None,
                    correlation_id: message.id.clone(),
                    retry_of: None,
                    deadline_unix_ms: 0,
                    state: "sent".into(),
                    attempt_count: 1,
                },
            )
            .unwrap();
        store
            .lock()
            .unwrap()
            .track_outbound_evidence(
                &hex::encode(resource_hash.to_bytes()),
                &message.id,
                "resource",
            )
            .unwrap();
        let worker = spawn_router_deadline_worker(messaging);

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if store.lock().unwrap().outbound_route(&message.id).unwrap().unwrap().state
                    == "expired"
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("resource route did not expire");

        assert!(transport.calls().iter().any(|call| {
            matches!(call, MockCall::CancelResource { hash } if *hash == resource_hash)
        }));
        worker.abort();
        let _ = worker.await;
    }
}
