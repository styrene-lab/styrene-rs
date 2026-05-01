//! RBAC enforcement scenarios.
//!
//! Tests that DaemonFacade correctly denies operations based on caller role.
//! The IPC server currently uses a single DaemonFacade for all clients
//! (per-client auth via SO_PEERCRED is future work), so these tests
//! exercise the auth gate directly through DaemonFacade with explicit
//! caller identities.

use std::time::Duration;

use styrene_e2e::helpers::{with_timeout, two_connected_nodes, SETTLE};
use styrene_e2e::node::TestNodeBuilder;
use styrened::daemon_facade::DaemonFacade;
use styrene_ipc::error::IpcError;
use styrene_ipc::traits::*;
use styrene_ipc::types::*;

#[tokio::test]
async fn blocked_caller_denied_all_operations() {
    with_timeout(async {
        let node = TestNodeBuilder::new("rbac-blocked")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let blocked_id = "blocked_peer_identity_hash_here";
        node.app_context.auth().block(blocked_id);

        let facade = DaemonFacade::new(node.app_context.clone(), blocked_id.into());

        // Status query should be denied
        let result = facade.query_status().await;
        assert!(
            matches!(result, Err(IpcError::Unavailable { .. })),
            "blocked caller should be denied status, got: {:?}",
            result
        );

        // Identity query should be denied
        let result = facade.query_identity().await;
        assert!(matches!(result, Err(IpcError::Unavailable { .. })));

        // Announce should be denied
        let result = facade.announce().await;
        assert!(matches!(result, Err(IpcError::Unavailable { .. })));

        // Send chat should be denied
        let mut req = SendChatRequest::default();
        req.peer_hash = "deadbeef".into();
        req.content = "test".into();
        let result = facade.send_chat(req).await;
        assert!(matches!(result, Err(IpcError::Unavailable { .. })));
    })
    .await;
}

#[tokio::test]
async fn peer_role_can_chat_but_not_exec() {
    with_timeout(async {
        let node = TestNodeBuilder::new("rbac-peer")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        // Default role is Peer for unknown callers
        let peer_id = "unknown_peer_identity_hash_1234";
        let facade = DaemonFacade::new(node.app_context.clone(), peer_id.into());

        // Peer can query status (Status capability is in Peer role)
        let result = facade.query_status().await;
        assert!(result.is_ok(), "peer should be able to query status");

        // Peer can query identity
        let result = facade.query_identity().await;
        assert!(result.is_ok(), "peer should be able to query identity");

        // Peer CANNOT exec (Exec capability requires Operator or higher)
        let result = facade
            .exec("target", "ls", vec![], None)
            .await;
        assert!(
            matches!(result, Err(IpcError::Unavailable { .. })),
            "peer should be denied exec, got: {:?}",
            result
        );

        // Peer CANNOT reboot
        let result = facade
            .reboot_device("target", None, None)
            .await;
        assert!(
            matches!(result, Err(IpcError::Unavailable { .. })),
            "peer should be denied reboot"
        );
    })
    .await;
}

#[tokio::test]
async fn operator_role_can_exec() {
    with_timeout(async {
        let node = TestNodeBuilder::new("rbac-operator")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let operator_id = "operator_identity_hash_1234abcd";
        node.app_context
            .auth()
            .set_role(operator_id, styrened::services::auth::Role::Operator);

        let facade = DaemonFacade::new(node.app_context.clone(), operator_id.into());

        // Operator can query status
        let result = facade.query_status().await;
        assert!(result.is_ok());

        // Operator can exec (Exec capability)
        // Will fail with transport error (no real connection), but should NOT
        // fail with Unavailable (auth denied).
        let result = facade
            .exec("target", "ls", vec![], None)
            .await;
        match result {
            Err(IpcError::Unavailable { .. }) => {
                panic!("operator should NOT be denied exec");
            }
            Err(IpcError::Internal { .. }) => {
                // Expected — transport not connected for the exec target
            }
            Ok(_) => {
                // Would only happen if target was reachable — fine
            }
            Err(other) => {
                panic!("unexpected error: {:?}", other);
            }
        }
    })
    .await;
}

#[tokio::test]
async fn unblock_restores_access() {
    with_timeout(async {
        let node = TestNodeBuilder::new("rbac-unblock")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let peer_id = "toggle_block_identity_hash_1234";

        // Block, verify denied
        node.app_context.auth().block(peer_id);
        let facade = DaemonFacade::new(node.app_context.clone(), peer_id.into());
        let result = facade.query_status().await;
        assert!(matches!(result, Err(IpcError::Unavailable { .. })));

        // Unblock, verify restored
        node.app_context.auth().unblock(peer_id);
        let result = facade.query_status().await;
        assert!(result.is_ok(), "unblocked peer should have access restored");
    })
    .await;
}
