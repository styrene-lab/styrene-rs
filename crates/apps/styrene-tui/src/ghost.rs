//! Ephemeral runtime state with deterministic cleanup.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::StyrenePaths;

#[derive(Debug)]
pub struct GhostSession {
    root: Option<PathBuf>,
}

impl GhostSession {
    pub fn for_paths(ephemeral: bool, data_dir: &Path) -> Self {
        let root = ephemeral.then(|| data_dir.parent().map(Path::to_path_buf)).flatten();
        Self { root }
    }

    #[cfg(test)]
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }
}

impl Drop for GhostSession {
    fn drop(&mut self) {
        if let Some(root) = self.root.take() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

/// Start the real embedded daemon in an isolated ephemeral session, prove IPC
/// readiness, stop through its normal shutdown path, and verify cleanup.
pub async fn run_ghost_lifecycle_check(parent: &Path, timeout: Duration) -> anyhow::Result<()> {
    std::fs::create_dir_all(parent)?;
    let parent = parent.canonicalize()?;
    let session_root = parent.join(format!("session-{}", std::process::id()));
    if session_root.exists() {
        anyhow::bail!("ghost check session already exists: {}", session_root.display());
    }
    let paths = StyrenePaths::new(
        session_root.join("config"),
        session_root.join("data"),
        session_root.join("run/styrene.sock"),
        session_root.join("home"),
    );
    std::fs::create_dir_all(&paths.data_dir)?;
    let session = GhostSession::for_paths(true, &paths.data_dir);

    let operation = async {
        let handle = styrened::daemon::start(styrened::daemon::DaemonConfig2 {
            db: Some(paths.data_dir.join("messages.db")),
            config: None,
            identity: None,
            socket: Some(paths.daemon_socket.clone()),
            ephemeral: true,
        })
        .await?;

        let ready = crate::connect_with_retry(&paths.daemon_socket).await;
        if let Err(error) = ready {
            handle.shutdown().await;
            anyhow::bail!("ghost runtime did not become ready: {error}");
        }
        drop(ready);
        handle.shutdown().await;
        if paths.daemon_socket.exists() {
            anyhow::bail!("ghost runtime socket survived shutdown: {}", paths.daemon_socket.display());
        }
        anyhow::Ok(())
    };

    let result = tokio::time::timeout(timeout, operation)
        .await
        .map_err(|_| anyhow::anyhow!("ghost lifecycle exceeded {} seconds", timeout.as_secs()))?;
    drop(session);
    if session_root.exists() {
        anyhow::bail!("ghost session survived cleanup: {}", session_root.display());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_removes_ephemeral_root() {
        let root = std::env::temp_dir().join(format!("styrene-ghost-drop-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data")).unwrap();
        {
            let session = GhostSession::for_paths(true, &root.join("data"));
            assert_eq!(session.root(), Some(root.as_path()));
        }
        assert!(!root.exists());
    }

    #[test]
    fn persistent_profile_has_no_cleanup_root() {
        let session = GhostSession::for_paths(false, Path::new("/persistent/data"));
        assert!(session.root().is_none());
    }
}
