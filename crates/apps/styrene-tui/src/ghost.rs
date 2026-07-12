//! Ephemeral runtime state with deterministic cleanup.

use std::path::{Path, PathBuf};

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
