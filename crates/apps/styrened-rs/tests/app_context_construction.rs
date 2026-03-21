//! Integration tests for AppContext — verifying the composition root
//! can be constructed and all service accessors work.

use reticulum_daemon::app_context::AppContext;
use reticulum_daemon::transport::mesh_transport::MeshTransport;
use reticulum_daemon::transport::null_transport::NullTransport;
use std::sync::Arc;

#[test]
fn app_context_constructs_with_null_transport() {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = AppContext::new(transport);

    // Verify transport is the NullTransport
    assert!(!ctx.transport().is_connected());
}

#[test]
fn app_context_all_service_accessors_work() {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = AppContext::new(transport);

    // All accessors should return valid references without panicking
    let _ = ctx.identity();
    let _ = ctx.config();
    let _ = ctx.status();
    let _ = ctx.fleet();
    let _ = ctx.auth();
    let _ = ctx.auto_reply();
    let _ = ctx.messaging();
    let _ = ctx.discovery();
    let _ = ctx.protocol();
    let _ = ctx.events();
    let _ = ctx.tunnel();
}

#[test]
fn app_context_transport_arc_returns_clone() {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = AppContext::new(transport);

    // transport_arc() returns a cloned Arc
    let arc = ctx.transport_arc();
    assert!(!arc.is_connected());
}

#[test]
fn app_context_can_be_wrapped_in_arc() {
    // Services will hold Arc<AppContext>, verify it works
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = Arc::new(AppContext::new(transport));

    let ctx_clone = ctx.clone();
    assert!(!ctx_clone.transport().is_connected());
    let _ = ctx_clone.identity();
    let _ = ctx_clone.messaging();
}
