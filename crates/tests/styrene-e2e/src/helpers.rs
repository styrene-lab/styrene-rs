//! Timeout wrappers and polling assertion helpers for e2e tests.

use std::time::Duration;

use rns_core::hash::AddressHash;
use styrened::app_context::AppContext;
use styrened::storage::messages::MessageRecord;

/// Maximum time any single e2e test should run.
pub const E2E_TIMEOUT: Duration = Duration::from_secs(30);

/// Settle delay for TCP connection establishment.
pub const SETTLE: Duration = Duration::from_millis(500);

/// Poll interval for assertion helpers.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Run a future with the standard e2e timeout, panicking on expiry.
pub async fn with_timeout<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::time::timeout(E2E_TIMEOUT, f)
        .await
        .expect("e2e test timed out after 30s")
}

/// Poll until the transport can resolve a peer's identity from announces.
pub async fn await_identity_resolved(
    ctx: &AppContext,
    dest_hash: &AddressHash,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if ctx.transport().resolve_identity(dest_hash).await.is_some() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for identity resolution of {}",
                hex::encode(dest_hash.as_slice())
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Poll until a path is available for a destination.
pub async fn await_path(
    ctx: &AppContext,
    dest_hash: &AddressHash,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if ctx.transport().query_path(dest_hash).await.is_some() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for path to {}",
                hex::encode(dest_hash.as_slice())
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Poll until an inbound message appears in the node's store.
pub async fn await_inbound_message(
    ctx: &AppContext,
    timeout: Duration,
) -> MessageRecord {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        {
            let store = ctx.store().lock().expect("lock store");
            if let Ok(messages) = store.list_messages(100, None) {
                if let Some(msg) = messages.into_iter().find(|m| m.direction == "in") {
                    return msg;
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for inbound message");
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Poll until at least `count` inbound messages appear in the node's store.
pub async fn await_inbound_count(
    ctx: &AppContext,
    count: usize,
    timeout: Duration,
) -> Vec<MessageRecord> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        {
            let store = ctx.store().lock().expect("lock store");
            if let Ok(messages) = store.list_messages(500, None) {
                let inbound: Vec<_> = messages
                    .into_iter()
                    .filter(|m| m.direction == "in")
                    .collect();
                if inbound.len() >= count {
                    return inbound;
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let store = ctx.store().lock().expect("lock store");
            let actual = store
                .list_messages(500, None)
                .map(|msgs| msgs.iter().filter(|m| m.direction == "in").count())
                .unwrap_or(0);
            panic!(
                "timed out waiting for {} inbound messages (got {})",
                count, actual
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Poll until total message count (in + out) reaches `count` on a node.
pub async fn await_message_count(
    ctx: &AppContext,
    count: usize,
    timeout: Duration,
) -> Vec<MessageRecord> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        {
            let store = ctx.store().lock().expect("lock store");
            if let Ok(messages) = store.list_messages(500, None) {
                if messages.len() >= count {
                    return messages;
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let store = ctx.store().lock().expect("lock store");
            let actual = store
                .list_messages(500, None)
                .map(|msgs| msgs.len())
                .unwrap_or(0);
            panic!(
                "timed out waiting for {} total messages (got {})",
                count, actual
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Set up two connected, mutually-announced nodes. Returns (server_node, client_node).
pub async fn two_connected_nodes(
    server_name: &str,
    client_name: &str,
) -> (crate::node::TestNode, crate::node::TestNode) {
    let server = crate::node::TestNodeBuilder::new(server_name)
        .tcp_server("127.0.0.1:0")
        .build()
        .await;

    let client = crate::node::TestNodeBuilder::new(client_name)
        .tcp_client(server.listen_addr.expect("listen addr"))
        .build()
        .await;

    tokio::time::sleep(SETTLE).await;

    server.announce().await;
    client.announce().await;

    await_identity_resolved(
        &server.app_context,
        &client.delivery_addr,
        Duration::from_secs(10),
    )
    .await;
    await_identity_resolved(
        &client.app_context,
        &server.delivery_addr,
        Duration::from_secs(10),
    )
    .await;

    (server, client)
}
