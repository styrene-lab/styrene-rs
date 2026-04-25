//! IPC contract test — starts a Rust IPC server and runs the Python
//! contract test script against it.
//!
//! This verifies that the Python IPC client can connect to and communicate
//! with the Rust IPC server using the shared wire protocol.
//!
//! Requires: python3 with msgpack installed
//!
//! Run: cargo test -p styrene-ipc-server --test ipc_contract_python

use std::sync::Arc;
use styrene_ipc::StubDaemon;
use styrene_ipc_server::{IpcServer, IpcServerConfig};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore] // TODO: debug msgpack decode mismatch between Python and Rust frame handling
async fn python_contract_tests() {
    // Check if python3 and msgpack are available
    let python_check =
        std::process::Command::new("python3").args(["-c", "import msgpack; print('ok')"]).output();

    match python_check {
        Ok(out) if out.status.success() => {}
        _ => {
            eprintln!("SKIPPED: python3 with msgpack not available");
            return;
        }
    }

    // Create temp socket (forget dir to prevent early cleanup)
    let dir = tempfile::tempdir().expect("tempdir");
    let sock_path = dir.path().join("contract.sock");
    let _sock_path_clone = sock_path.clone(); // hold reference
    std::mem::forget(dir);

    // Start IPC server with StubDaemon
    let config = IpcServerConfig { socket_path: sock_path.clone(), event_capacity: 64 };
    let daemon: Arc<dyn styrene_ipc::traits::Daemon> = Arc::new(StubDaemon);
    let mut server = IpcServer::new(daemon, config);
    server.start().await.expect("start IPC server");

    // Give server time to bind and start accept loop
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Verify socket exists
    assert!(sock_path.exists(), "socket file not created at {}", sock_path.display());

    // Run Python contract tests (async to not block tokio runtime)
    let script_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/interop/python/ipc_contract.py");

    let output = tokio::process::Command::new("python3")
        .arg(&script_path)
        .arg(&sock_path)
        .output()
        .await
        .expect("run Python contract tests");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    eprintln!("{stdout}");
    if !stderr.is_empty() {
        eprintln!("stderr: {stderr}");
    }

    server.stop().await;

    assert!(
        output.status.success(),
        "Python contract tests failed (exit code {:?}):\n{stdout}\n{stderr}",
        output.status.code()
    );
}
