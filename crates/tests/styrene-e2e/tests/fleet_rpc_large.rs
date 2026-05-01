//! Fleet RPC with resource-sized responses.
//!
//! Tests that exec commands producing large stdout (>300 bytes, triggering
//! resource transfer on the return path) deliver complete results.

use std::time::Duration;
use styrene_e2e::helpers::{with_timeout, two_connected_nodes};

#[tokio::test]
async fn exec_with_large_stdout() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-lg-rpc", "bob-lg-rpc").await;

        // Grant alice Operator role on bob
        bob.app_context
            .auth()
            .set_role(&alice.identity_hash, styrened::services::auth::Role::Operator);

        // Execute a command that produces >1KB of output.
        // `seq 1 200` produces ~600 bytes of output (numbers 1-200, one per line).
        // On macOS `seq` might not exist, use `printf` with a loop via sh.
        let result = alice
            .app_context
            .fleet()
            .exec(
                &bob.delivery_hash,
                "sh",
                &["-c".into(), "for i in $(seq 1 200); do echo line_$i; done".into()],
                Some(30),
            )
            .await;

        let exec_result = result.expect("exec with large stdout should succeed");
        assert_eq!(exec_result.exit_code, 0, "command should exit 0");
        assert!(
            exec_result.stdout.len() > 500,
            "stdout should be >500 bytes (resource transfer), got {} bytes",
            exec_result.stdout.len()
        );
        assert!(
            exec_result.stdout.contains("line_1"),
            "stdout should contain first line"
        );
        assert!(
            exec_result.stdout.contains("line_200"),
            "stdout should contain last line"
        );
    })
    .await;
}

#[tokio::test]
async fn status_works_after_large_exec() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-post-lg", "bob-post-lg").await;

        bob.app_context
            .auth()
            .set_role(&alice.identity_hash, styrened::services::auth::Role::Operator);

        // Large exec first
        let result = alice
            .app_context
            .fleet()
            .exec(
                &bob.delivery_hash,
                "sh",
                &["-c".into(), "for i in $(seq 1 100); do echo x_$i; done".into()],
                Some(30),
            )
            .await;
        assert!(result.is_ok(), "large exec should succeed");

        // Normal status RPC after — should still work
        let status = alice
            .app_context
            .fleet()
            .device_status(&bob.delivery_hash, Some(10))
            .await
            .expect("status after large exec should succeed");

        assert!(
            status.daemon_version.is_some(),
            "status should include version"
        );
    })
    .await;
}
