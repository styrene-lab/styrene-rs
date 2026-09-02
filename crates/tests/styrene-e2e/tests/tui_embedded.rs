//! The TUI's embedded runtime path: start the owned `styrened` runtime on the
//! configured socket, connect through the TUI daemon layer, and shut down.

use std::time::Duration;

use styrene_tui::{RuntimeProfile, StyrenePaths, TuiOptions};

#[tokio::test]
async fn tui_embedded_runtime_starts_answers_and_shuts_down() {
    let root = tempfile::tempdir().expect("root");
    let paths = StyrenePaths::new(
        root.path().join("config"),
        root.path().join("data"),
        root.path().join("run/styrene.sock"),
        root.path().join("home"),
    );
    std::fs::create_dir_all(&paths.config_dir).expect("config dir");
    std::fs::create_dir_all(&paths.data_dir).expect("data dir");
    std::fs::create_dir_all(paths.daemon_socket.parent().expect("socket dir")).expect("run dir");
    let options = TuiOptions { paths: paths.clone(), runtime_profile: RuntimeProfile::Ghost };

    let runtime = tokio::time::timeout(
        Duration::from_secs(30),
        styrene_tui::start_embedded_runtime(&options),
    )
    .await
    .expect("embedded start finishes")
    .expect("embedded runtime starts");
    assert!(paths.daemon_socket.exists(), "embedded runtime listens on the configured socket");

    let mut connection = styrene_tui::connect_with_retry(&paths.daemon_socket)
        .await
        .expect("TUI connects to its embedded runtime");
    let mut handle = connection.take_handle();
    let status = handle.status().await.expect("status");
    assert!(status.rns_initialized);
    assert!(status.connection_generation.is_some_and(|generation| generation != 0));
    let identity = handle.identity().await.expect("identity");
    assert_eq!(identity.identity_hash.len(), 32);
    drop(connection);

    runtime.shutdown().await;
    assert!(!paths.daemon_socket.exists(), "shutdown removes the socket");
}
