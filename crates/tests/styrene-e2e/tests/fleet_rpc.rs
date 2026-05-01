//! Fleet RPC end-to-end scenarios.
//!
//! Tests the full RPC round-trip: Node A sends a StatusRequest to Node B,
//! B processes it via RpcRequestHandler and sends a StatusResponse back,
//! A's FleetService correlates the response and returns the result.
//!
//! This exercises: StyreneMessage wire encoding, LXMF wrapping, link
//! delivery, protocol dispatch, request handling, response correlation.

use std::time::Duration;
use styrene_e2e::helpers::{with_timeout, two_connected_nodes};
use styrene_rbac::{RosterEntry, Role};

#[tokio::test]
async fn device_status_rpc_roundtrip() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-rpc", "bob-rpc").await;

        // Alice queries Bob's status via RPC.
        // This requires Bob to have an RpcRequestHandler registered that
        // processes StatusRequest and sends back StatusResponse.
        let result = alice
            .app_context
            .fleet()
            .device_status(&bob.delivery_hash, Some(10))
            .await;

        match result {
            Ok(status) => {
                assert_eq!(
                    status.destination_hash, bob.delivery_hash,
                    "response should reference the queried destination"
                );
                // Bob should report some version
                assert!(
                    status.daemon_version.is_some(),
                    "status response should include daemon version"
                );
            }
            Err(e) => {
                let msg = e.to_string();
                // If this fails with "RPC timeout" — Bob has no request handler.
                // If it fails with "peer not announced" — announce exchange issue.
                panic!(
                    "device_status RPC failed: {}\n\
                     This likely means the receiving node has no RpcRequestHandler \
                     to process incoming StatusRequest messages.",
                    msg
                );
            }
        }
    })
    .await;
}

#[tokio::test]
async fn exec_rpc_roundtrip() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-exec", "bob-exec").await;

        // Grant alice Admin role on bob's policy service so exec is allowed
        let entry = RosterEntry::new(&alice.identity_hash, Role::Admin);
        bob.app_context.policy().grant(entry, bob.app_context.store()).expect("grant");

        let result = alice
            .app_context
            .fleet()
            .exec(&bob.delivery_hash, "echo", &["hello".into()], Some(10))
            .await;

        match result {
            Ok(exec_result) => {
                // The test handler should echo back
                assert_eq!(exec_result.exit_code, 0);
                assert!(
                    !exec_result.stdout.is_empty(),
                    "exec result should have stdout"
                );
            }
            Err(e) => {
                panic!(
                    "exec RPC failed: {}\n\
                     This likely means the receiving node has no RpcRequestHandler \
                     to process incoming Exec messages.",
                    e
                );
            }
        }
    })
    .await;
}

#[tokio::test]
async fn rpc_to_unannounced_peer_fails_with_timeout() {
    with_timeout(async {
        let alice = styrene_e2e::node::TestNodeBuilder::new("alice-rpc-fail")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        // Query a non-existent peer — should fail, not hang
        let result = alice
            .app_context
            .fleet()
            .device_status("deadbeefdeadbeefdeadbeefdeadbeef", Some(3))
            .await;

        assert!(result.is_err(), "RPC to unknown peer should fail");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not announced")
                || err_msg.contains("not resolved")
                || err_msg.contains("timeout")
                || err_msg.contains("failed"),
            "error should indicate delivery failure, got: {}",
            err_msg
        );
    })
    .await;
}
