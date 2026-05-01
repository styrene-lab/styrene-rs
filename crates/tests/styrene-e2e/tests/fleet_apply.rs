//! Fleet profile application — ConfigUpdate RPC end-to-end.
//!
//! Tests the fleet apply flow: push profile bytes to a remote node,
//! remote node processes and applies, returns result.

use std::time::Duration;
use styrene_e2e::helpers::{with_timeout, two_connected_nodes};
use styrene_rbac::{RosterEntry, Role};

#[tokio::test]
async fn fleet_apply_profile_roundtrip() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-apply", "bob-apply").await;

        // Grant alice Admin role on bob (UpdateConfig requires Admin)
        let entry = RosterEntry::new(&alice.identity_hash, Role::Admin);
        bob.app_context.policy().grant(entry, bob.app_context.store()).expect("grant");

        // Create a simple profile (TOML config)
        let profile = b"role = \"full_node\"\n\n[[interfaces]]\ntype = \"tcp_server\"\nenabled = true\nhost = \"0.0.0.0\"\nport = 4242\n";

        let result = alice
            .app_context
            .fleet()
            .apply(&bob.delivery_hash, profile, false, Some(15))
            .await;

        let apply_result = result.expect(
            "fleet apply RPC should complete (not timeout) — \
             the handler exists and processes ConfigUpdate"
        );

        // The handler delegates to `nex profile apply` which won't be on
        // PATH in tests. The important thing is:
        // 1. The RPC round-trip completed (didn't timeout)
        // 2. The RBAC check passed (Admin granted)
        // 3. The profile bytes were received and decoded
        // 4. The response has the correct structure
        if !apply_result.success {
            // Expected in test: nex binary not available
            assert!(
                apply_result.stderr.contains("nex")
                    || apply_result.stderr.contains("not found")
                    || apply_result.stderr.contains("No such file"),
                "failure should be due to missing nex binary, got stderr: {}",
                apply_result.stderr
            );
        }
        // Either way, the RPC completed — handler is wired and RBAC passed
    })
    .await;
}

#[tokio::test]
async fn fleet_apply_denied_without_admin_role() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-apply-deny", "bob-apply-deny").await;

        // alice has default Peer role — should be denied UpdateConfig
        let result = alice
            .app_context
            .fleet()
            .apply(&bob.delivery_hash, b"profile data", false, Some(10))
            .await;

        let apply_result = result.expect("RPC should complete, returning denial response");
        assert!(
            !apply_result.success,
            "apply without Admin role should not succeed"
        );
        assert!(
            apply_result.stderr.contains("permission denied")
                || apply_result.stderr.contains("denied"),
            "stderr should indicate permission denied, got: {}",
            apply_result.stderr
        );
    })
    .await;
}

#[tokio::test]
async fn fleet_apply_oversized_profile_rejected_locally() {
    with_timeout(async {
        let node = styrene_e2e::node::TestNodeBuilder::new("oversized")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        // 3MB profile — exceeds the 2MB limit
        let oversized = vec![0x42u8; 3 * 1024 * 1024];

        let result = node
            .app_context
            .fleet()
            .apply("deadbeefdeadbeefdeadbeefdeadbeef", &oversized, false, Some(5))
            .await;

        assert!(result.is_err(), "oversized profile should be rejected locally");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("too large"),
            "error should mention size limit, got: {}",
            err
        );
    })
    .await;
}
