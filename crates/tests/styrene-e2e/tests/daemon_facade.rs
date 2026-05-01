//! DaemonFacade IPC scenarios — exercising the full IPC surface
//! through real nodes with real transport.
//!
//! Tests contacts, conversations, search, mark_read, delete,
//! pin/mute, query_devices, query_status, auto-reply, block/unblock
//! as they'd be called by the TUI or CLI over IPC.

use std::time::Duration;
use styrene_e2e::helpers::{
    with_timeout, await_identity_resolved, await_inbound_count, SETTLE,
};
use styrene_e2e::node::TestNodeBuilder;
use styrened::daemon_facade::DaemonFacade;
use styrene_ipc::traits::*;
use styrene_ipc::types::*;

/// Build a DaemonFacade for a TestNode, using the node's own identity as caller.
fn facade_for(node: &styrene_e2e::node::TestNode) -> DaemonFacade {
    DaemonFacade::new(node.app_context.clone(), node.identity_hash.clone())
}

// ── Status & Identity ──────────────────────────────────────────────────

#[tokio::test]
async fn query_status_reflects_connected_state() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-status")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let facade = facade_for(&alice);
        let status = facade.query_status().await.expect("query_status");

        assert!(status.rns_initialized, "transport should be initialized");
        assert!(status.transport_enabled, "transport should be enabled");
        assert!(status.uptime >= 0);
        assert!(!status.daemon_version.is_empty());
    })
    .await;
}

#[tokio::test]
async fn query_identity_returns_correct_hashes() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-id")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let facade = facade_for(&alice);
        let info = facade.query_identity().await.expect("query_identity");

        assert_eq!(info.identity_hash, alice.identity_hash);
        assert!(!info.destination_hash.is_empty());
        assert_eq!(info.lxmf_destination_hash, info.destination_hash);
    })
    .await;
}

#[tokio::test]
async fn set_identity_updates_display_name() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-set-id")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let facade = facade_for(&alice);

        let changed = facade
            .set_identity(Some("Alice Node"), None, None)
            .await
            .expect("set_identity");
        assert!(changed);

        let info = facade.query_identity().await.expect("query after set");
        assert_eq!(info.display_name, "Alice Node");
    })
    .await;
}

// ── Contacts ───────────────────────────────────────────────────────────

#[tokio::test]
async fn contact_crud_lifecycle() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-contacts")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let facade = facade_for(&alice);
        let peer_hash = "aabbccddaabbccddaabbccddaabbccdd";

        // Create contact
        let contact = facade
            .set_contact(peer_hash, Some("Bob"), Some("My friend"))
            .await
            .expect("set_contact");
        assert_eq!(contact.peer_hash, peer_hash);
        assert_eq!(contact.alias, Some("Bob".into()));
        assert_eq!(contact.notes, Some("My friend".into()));

        // List contacts
        let contacts = facade.query_contacts().await.expect("query_contacts");
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].alias, Some("Bob".into()));

        // Update contact
        let updated = facade
            .set_contact(peer_hash, Some("Robert"), None)
            .await
            .expect("update contact");
        assert_eq!(updated.alias, Some("Robert".into()));

        // Remove contact
        let removed = facade.remove_contact(peer_hash).await.expect("remove");
        assert!(removed);

        let contacts = facade.query_contacts().await.expect("query after remove");
        assert!(contacts.is_empty());
    })
    .await;
}

// ── Conversations & Messages ───────────────────────────────────────────

#[tokio::test]
async fn conversation_operations_after_message_exchange() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-conv")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob-conv")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        bob.announce().await;

        await_identity_resolved(
            &alice.app_context,
            &bob.delivery_addr,
            Duration::from_secs(10),
        )
        .await;
        await_identity_resolved(
            &bob.app_context,
            &alice.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        // Alice sends to Bob
        alice.send_chat(&bob.delivery_hash, "conv-test-1").await.expect("send");
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        let bob_facade = facade_for(&bob);

        // Query conversations — bob should have 1
        let convos = bob_facade
            .query_conversations(false)
            .await
            .expect("query_conversations");
        assert_eq!(convos.len(), 1, "bob should have 1 conversation");
        assert_eq!(convos[0].message_count, 1);

        // Query messages for the conversation
        let messages = bob_facade
            .query_messages(&convos[0].peer_hash, 100, None)
            .await
            .expect("query_messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "conv-test-1");
        assert!(!messages[0].is_outgoing);

        // Mark read
        let read_count = bob_facade
            .mark_read(&convos[0].peer_hash)
            .await
            .expect("mark_read");
        assert!(read_count >= 1);

        // Search messages
        let results = bob_facade
            .search_messages("conv-test", None, 10)
            .await
            .expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "conv-test-1");

        // Search with wrong query
        let no_results = bob_facade
            .search_messages("nonexistent", None, 10)
            .await
            .expect("search empty");
        assert!(no_results.is_empty());

        // Delete conversation
        let deleted = bob_facade
            .delete_conversation(&convos[0].peer_hash)
            .await
            .expect("delete_conversation");
        assert!(deleted >= 1);

        let convos = bob_facade
            .query_conversations(false)
            .await
            .expect("query after delete");
        assert!(convos.is_empty());
    })
    .await;
}

// ── Pin & Mute ─────────────────────────────────────────────────────────

#[tokio::test]
async fn pin_and_mute_conversations() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-pin")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let facade = facade_for(&alice);
        let peer = "1111111111111111111111111111111";

        // Pin
        let pinned = facade.pin_conversation(peer).await.expect("pin");
        assert!(pinned);

        // Mute
        let muted = facade.mute_conversation(peer).await.expect("mute");
        assert!(muted);

        // Unpin
        let unpinned = facade.unpin_conversation(peer).await.expect("unpin");
        assert!(unpinned);

        // Unmute
        let unmuted = facade.unmute_conversation(peer).await.expect("unmute");
        assert!(unmuted);
    })
    .await;
}

// ── Auto-reply ─────────────────────────────────────────────────────────

#[tokio::test]
async fn auto_reply_configuration() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-ar")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let facade = facade_for(&alice);

        // Default should be disabled
        let config = facade.query_auto_reply().await.expect("query");
        assert_eq!(config.mode, "disabled");

        // Enable
        facade
            .set_auto_reply("all", Some("I'm away"), Some(120))
            .await
            .expect("set");

        let config = facade.query_auto_reply().await.expect("query after set");
        assert_eq!(config.mode, "all");
        assert_eq!(config.message, Some("I'm away".into()));
        assert_eq!(config.cooldown_secs, Some(120));

        // Invalid mode
        let result = facade.set_auto_reply("invalid", None, None).await;
        assert!(result.is_err());

        // Disable
        facade
            .set_auto_reply("disabled", None, None)
            .await
            .expect("disable");
        let config = facade.query_auto_reply().await.expect("final query");
        assert_eq!(config.mode, "disabled");
    })
    .await;
}

// ── Devices & Peer Discovery ───────────────────────────────────────────

#[tokio::test]
async fn query_devices_after_announce() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-dev")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob-dev")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        bob.announce().await;

        await_identity_resolved(
            &alice.app_context,
            &bob.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        let facade = facade_for(&alice);
        let devices = facade.query_devices(false).await.expect("query_devices");

        assert!(
            !devices.is_empty(),
            "should see at least one device after announce"
        );

        let bob_device = devices
            .iter()
            .find(|d| d.destination_hash == bob.delivery_hash)
            .expect("bob should be in device list");
        assert!(bob_device.announce_count >= 1);
    })
    .await;
}

#[tokio::test]
async fn search_peers_finds_by_name() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-search")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob-search")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        bob.announce().await;

        await_identity_resolved(
            &alice.app_context,
            &bob.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        let facade = facade_for(&alice);

        // Search by name
        let results = facade.search_peers("bob", 10).await.expect("search");
        assert!(
            !results.is_empty(),
            "should find bob by name"
        );

        // Search by hash prefix
        let prefix = &bob.delivery_hash[..8];
        let results = facade.search_peers(prefix, 10).await.expect("search prefix");
        assert!(
            !results.is_empty(),
            "should find bob by hash prefix"
        );

        // Search for nonexistent
        let results = facade.search_peers("zzzzzzz", 10).await.expect("search miss");
        assert!(results.is_empty());
    })
    .await;
}

// ── Block & Bookmark ───────────────────────────────────────────────────

#[tokio::test]
async fn block_and_unblock_peer() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-block")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let facade = facade_for(&alice);
        let peer = "deadbeefdeadbeefdeadbeefdeadbeef";

        // Block
        facade.block_peer(peer).await.expect("block");

        let blocked = facade.blocked_peers().await.expect("list blocked");
        assert!(
            blocked.contains(&peer.to_string()),
            "peer should be in blocked list"
        );

        // Unblock
        facade.unblock_peer(peer).await.expect("unblock");

        let blocked = facade.blocked_peers().await.expect("list after unblock");
        assert!(
            !blocked.contains(&peer.to_string()),
            "peer should not be in blocked list after unblock"
        );
    })
    .await;
}

#[tokio::test]
async fn bookmark_peer() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-bm")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob-bm")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        bob.announce().await;

        await_identity_resolved(
            &alice.app_context,
            &bob.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        let facade = facade_for(&alice);

        // Bookmark bob
        facade
            .bookmark_peer(&bob.delivery_hash)
            .await
            .expect("bookmark");

        // Verify via node store
        let node = alice
            .app_context
            .node_store()
            .get(&bob.delivery_hash)
            .expect("get node")
            .expect("node should exist");
        assert!(node.bookmarked, "node should be bookmarked");

        // Unbookmark
        facade
            .unbookmark_peer(&bob.delivery_hash)
            .await
            .expect("unbookmark");

        let node = alice
            .app_context
            .node_store()
            .get(&bob.delivery_hash)
            .expect("get node")
            .expect("node should exist");
        assert!(!node.bookmarked, "node should not be bookmarked");
    })
    .await;
}

// ── Path Info ──────────────────────────────────────────────────────────

#[tokio::test]
async fn query_path_info_for_known_peer() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-path")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob-path")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        bob.announce().await;

        await_identity_resolved(
            &alice.app_context,
            &bob.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        // Request path first
        alice.app_context.transport().request_path(&bob.delivery_addr).await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        let facade = facade_for(&alice);
        let path = facade
            .query_path_info(&bob.delivery_hash)
            .await
            .expect("query_path_info");

        assert_eq!(path.destination_hash, bob.delivery_hash);
        // Hops may or may not be populated depending on path table state
    })
    .await;
}
