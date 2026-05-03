//! Fleet RPC authorization scenarios.
//!
//! Tests that the RpcRequestHandler on the receiving node checks RBAC
//! before executing commands from remote peers. Without auth, any peer
//! on the mesh can execute arbitrary commands — an RCE vector.

use std::time::Duration;
use styrene_e2e::helpers::{await_inbound_count, two_connected_nodes, with_timeout};

#[tokio::test]
async fn exec_from_unknown_peer_is_denied() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-auth", "bob-auth").await;

        // Bob's auth service has alice as default Peer role.
        // Peer role should NOT have Exec capability.
        // Alice sends an exec RPC to Bob — Bob should deny it.
        let result =
            alice.app_context.fleet().exec(&bob.delivery_hash, "whoami", &[], Some(10)).await;

        // The exec should either:
        // a) Return an error (RPC response with error payload), or
        // b) Timeout because Bob refused to respond
        // It should NOT return a successful ExecResult with stdout.
        match result {
            Ok(exec_result) => {
                // If we got a response, verify it indicates denial
                // (exit_code -1 and stderr mentioning "denied" or "unauthorized")
                if exec_result.exit_code == 0 && !exec_result.stdout.is_empty() {
                    panic!(
                        "SECURITY: exec RPC succeeded from unauthorized peer!\n\
                         stdout: {}\n\
                         The RpcRequestHandler must check RBAC before executing commands.",
                        exec_result.stdout
                    );
                }
                // exit_code != 0 is acceptable (command failed or was denied)
            }
            Err(e) => {
                let msg = e.to_string();
                // Timeout or explicit denial are both acceptable
                assert!(
                    msg.contains("timeout")
                        || msg.contains("denied")
                        || msg.contains("unauthorized")
                        || msg.contains("failed"),
                    "expected auth denial or timeout, got: {}",
                    msg
                );
            }
        }
    })
    .await;
}

#[tokio::test]
async fn status_from_any_peer_is_allowed() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-status-auth", "bob-status-auth").await;

        // StatusRequest should be allowed from any peer (read-only)
        let result = alice.app_context.fleet().device_status(&bob.delivery_hash, Some(10)).await;

        // Status should succeed regardless of auth level
        assert!(
            result.is_ok(),
            "status request should be allowed from any peer, got: {:?}",
            result.err()
        );

        let status = result.expect("status");
        assert!(status.daemon_version.is_some(), "status should include version");
    })
    .await;
}

#[tokio::test]
async fn rbac_identity_hash_matches_wire_source() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-hash", "bob-hash").await;

        // Alice sends a chat to Bob — we want to verify that the source_hash
        // in Bob's inbound message matches alice.identity_hash, which is the
        // same value used by AuthService for role lookups.
        alice.send_chat(&bob.delivery_hash, "hash-check").await.expect("send");

        let msgs = await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        // The source field in the message record comes from the LXMF wire
        // (the sender's identity address hash). This must match what
        // AuthService uses for check().
        assert_eq!(
            msgs[0].source, alice.identity_hash,
            "LXMF wire source_hash must match the sender's identity_hash \
             for RBAC consistency. Wire source: {}, identity_hash: {}",
            msgs[0].source, alice.identity_hash
        );

        // And the identity_hash is what we'd pass to policy.grant()
        use styrene_rbac::{Capability, Role, RosterEntry};
        let entry = RosterEntry::new(&alice.identity_hash, Role::Admin);
        bob.app_context
            .policy()
            .grant(entry, bob.app_context.store())
            .expect("grant should succeed");
        assert!(
            bob.app_context.policy().has_capability(&msgs[0].source, Capability::RPC_EXEC),
            "RBAC check using wire source_hash should match role set via identity_hash"
        );
    })
    .await;
}

#[tokio::test]
async fn operator_grant_enables_exec_over_mesh() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-grant", "bob-grant").await;

        // Grant alice Admin role on bob's policy service BEFORE the exec call
        // (RPC_EXEC requires Admin in the new RBAC model)
        use styrene_rbac::{Role, RosterEntry};
        let entry = RosterEntry::new(&alice.identity_hash, Role::Admin);
        bob.app_context
            .policy()
            .grant(entry, bob.app_context.store())
            .expect("grant should succeed");

        // Now alice can exec on bob
        let result = alice
            .app_context
            .fleet()
            .exec(&bob.delivery_hash, "echo", &["authorized".into()], Some(10))
            .await;

        let exec_result = result.expect("authorized exec should succeed");
        assert_eq!(exec_result.exit_code, 0);
        assert!(
            exec_result.stdout.contains("authorized"),
            "stdout should contain the echoed argument"
        );
    })
    .await;
}
