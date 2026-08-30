//! CLI endpoint selection for the shared IPC client.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use styrene_ipc_client::{Client, ConnectionGeneration, default_socket_path};
use tokio::net::UnixStream;

pub async fn connect(socket_path: Option<&Path>) -> Result<Client, String> {
    let path = socket_path
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("STYRENE_SOCKET").map(PathBuf::from))
        .unwrap_or_else(default_socket_path);

    if path.to_string_lossy().starts_with("tcp://") {
        return Err("TCP IPC mode has been removed for security reasons. \
             Use a Unix socket (default) or SSH tunnel for remote access."
            .into());
    }
    if !path.exists() {
        return Err(format!(
            "daemon socket not found: {}\nIs styrene daemon running?",
            path.display()
        ));
    }

    let stream = UnixStream::connect(&path)
        .await
        .map_err(|error| format!("connect {}: {error}", path.display()))?;
    static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
    let generation = ConnectionGeneration(NEXT_GENERATION.fetch_add(1, Ordering::Relaxed));
    let client = Client::from_unix_stream(stream, generation);
    client.ping().await.map_err(|error| error.to_string())?;
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_removed_tcp_endpoints() {
        let error = match connect(Some(Path::new("tcp://127.0.0.1:9999"))).await {
            Ok(_) => panic!("TCP endpoint must be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("TCP IPC mode has been removed"));
    }

    #[tokio::test]
    async fn reports_missing_local_socket_before_connecting() {
        let path = std::env::temp_dir().join(format!(
            "styrene-missing-socket-{}-{}",
            std::process::id(),
            NEXT_TEST_PATH.fetch_add(1, Ordering::Relaxed)
        ));
        let error = match connect(Some(&path)).await {
            Ok(_) => panic!("missing socket must fail"),
            Err(error) => error,
        };
        assert!(error.contains("daemon socket not found"));
        assert!(error.contains(&path.display().to_string()));
    }

    static NEXT_TEST_PATH: AtomicU64 = AtomicU64::new(1);
}
