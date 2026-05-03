//! Delivery edge cases — retry, error propagation, and failure handling.

use std::time::Duration;
use styrene_e2e::helpers::{await_inbound_message, two_connected_nodes, with_timeout, SETTLE};
use styrene_e2e::node::TestNodeBuilder;
use styrene_ipc::traits::*;
use styrene_rbac::{Role, RosterEntry};
use styrened::daemon_facade::DaemonFacade;

// ── Message Retry ──────────────────────────────────────────────────────

#[tokio::test]
async fn failed_message_has_failed_receipt_status() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-retry").tcp_server("127.0.0.1:0").build().await;

        // Send to unknown peer — will fail after 12s identity resolution timeout
        let fake_hash = "deadbeefdeadbeefdeadbeefdeadbeef";
        let result = alice.send_chat(fake_hash, "will fail").await;

        // Whether Ok or Err, check the store has a failed record
        let store = alice.app_context.store().lock().expect("lock");
        let msgs = store.list_messages(100, None).expect("list");
        let failed: Vec<_> = msgs
            .iter()
            .filter(|m| {
                m.direction == "out"
                    && m.receipt_status.as_deref().map(|s| s.contains("failed")).unwrap_or(false)
            })
            .collect();

        assert!(
            !failed.is_empty(),
            "should have at least one failed outbound message, got: {:?}",
            msgs.iter().map(|m| (&m.direction, &m.receipt_status)).collect::<Vec<_>>()
        );
    })
    .await;
}

#[tokio::test]
async fn retry_inbound_message_rejected() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-retry-in", "bob-retry-in").await;

        // Send a message so bob has an inbound record
        alice.send_chat(&bob.delivery_hash, "original").await.expect("send");
        await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;

        // Get the message ID from bob's store
        let msg_id = {
            let store = bob.app_context.store().lock().expect("lock");
            let msgs = store.list_messages(100, None).expect("list");
            let inbound = msgs.iter().find(|m| m.direction == "in").expect("inbound");
            inbound.id.clone()
        };

        // Try to retry an inbound message — should be rejected
        let facade = DaemonFacade::new(bob.app_context.clone(), bob.identity_hash.clone());
        let result = facade.retry_message(&msg_id).await;
        assert!(result.is_err(), "retrying an inbound message should be rejected");
    })
    .await;
}

// ── Fleet Exec Error Propagation ───────────────────────────────────────

#[tokio::test]
async fn fleet_exec_nonexistent_command() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-execerr", "bob-execerr").await;

        let entry = RosterEntry::new(&alice.identity_hash, Role::Admin);
        bob.app_context.policy().grant(entry, bob.app_context.store()).expect("grant");

        // Execute a command that doesn't exist
        let result = alice
            .app_context
            .fleet()
            .exec(&bob.delivery_hash, "this_command_definitely_does_not_exist_xyz", &[], Some(20))
            .await;

        let exec_result = result.expect("RPC should succeed even if command fails");
        assert_ne!(exec_result.exit_code, 0, "nonexistent command should have non-zero exit code");
        assert!(!exec_result.stderr.is_empty(), "stderr should contain error message, got empty");
    })
    .await;
}

#[tokio::test]
async fn fleet_exec_failing_command() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-execfail", "bob-execfail").await;

        let entry = RosterEntry::new(&alice.identity_hash, Role::Admin);
        bob.app_context.policy().grant(entry, bob.app_context.store()).expect("grant");

        // Execute `false` — exits with code 1
        let result =
            alice.app_context.fleet().exec(&bob.delivery_hash, "false", &[], Some(10)).await;

        let exec_result = result.expect("RPC should succeed");
        assert_ne!(exec_result.exit_code, 0, "`false` should exit with non-zero");
    })
    .await;
}

#[tokio::test]
async fn fleet_exec_preserves_stderr() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-stderr", "bob-stderr").await;

        let entry = RosterEntry::new(&alice.identity_hash, Role::Admin);
        bob.app_context.policy().grant(entry, bob.app_context.store()).expect("grant");

        // Write to stderr via sh
        let result = alice
            .app_context
            .fleet()
            .exec(
                &bob.delivery_hash,
                "sh",
                &["-c".into(), "echo err_output >&2; exit 42".into()],
                Some(20),
            )
            .await;

        let exec_result = result.expect("RPC should succeed");
        assert_eq!(exec_result.exit_code, 42);
        assert!(
            exec_result.stderr.contains("err_output"),
            "stderr should contain 'err_output', got: '{}'",
            exec_result.stderr
        );
    })
    .await;
}
