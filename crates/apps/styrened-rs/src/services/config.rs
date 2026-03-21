//! ConfigService — configuration loading and persistence.
//!
//! Owns: 13.1 config load/save, 13.3 hardware/system info.
//! Package: E
//!
//! Note: this wraps the `DaemonConfig` model from `crate::config`,
//! adding service-layer operations (load, reload, interface enumeration).

use crate::config::{DaemonConfig, InterfaceConfig, NodeRole};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Interior state — guarded by a single Mutex to prevent TOCTOU races.
struct ConfigState {
    path: Option<PathBuf>,
    config: Option<DaemonConfig>,
}

/// Service-layer configuration management.
pub struct ConfigService {
    state: Mutex<ConfigState>,
}

impl ConfigService {
    /// Create a new ConfigService. Optionally loads from the given path.
    pub fn with_path(path: &Path) -> Result<Self, std::io::Error> {
        let config = DaemonConfig::from_path(path)?;
        Ok(Self {
            state: Mutex::new(ConfigState {
                path: Some(path.to_path_buf()),
                config: Some(config),
            }),
        })
    }

    /// Create an empty ConfigService (no config file).
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ConfigState {
                path: None,
                config: None,
            }),
        }
    }

    /// Load configuration from a path (sets both path and config atomically).
    pub fn load(&self, path: &Path) -> Result<(), std::io::Error> {
        let config = DaemonConfig::from_path(path)?;
        let mut s = self.state.lock().unwrap();
        s.path = Some(path.to_path_buf());
        s.config = Some(config);
        Ok(())
    }

    /// Reload configuration from disk.
    pub fn reload(&self) -> Result<(), std::io::Error> {
        let path = {
            let s = self.state.lock().unwrap();
            s.path.clone()
        };
        let Some(path) = path else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no config path set",
            ));
        };
        let config = DaemonConfig::from_path(&path)?;
        self.state.lock().unwrap().config = Some(config);
        Ok(())
    }

    /// Get the config path, if set.
    pub fn config_path(&self) -> Option<PathBuf> {
        self.state.lock().unwrap().path.clone()
    }

    /// Check if a config is loaded.
    pub fn is_loaded(&self) -> bool {
        self.state.lock().unwrap().config.is_some()
    }

    /// Get the list of configured TCP client endpoints.
    pub fn tcp_client_endpoints(&self) -> Vec<(String, u16)> {
        self.state
            .lock()
            .unwrap()
            .config
            .as_ref()
            .map(|c| c.tcp_client_endpoints())
            .unwrap_or_default()
    }

    /// Get the configured node role (default: FullNode).
    pub fn node_role(&self) -> NodeRole {
        self.state
            .lock()
            .unwrap()
            .config
            .as_ref()
            .map(|c| c.role)
            .unwrap_or_default()
    }

    /// Get all configured interfaces.
    pub fn interfaces(&self) -> Vec<InterfaceConfig> {
        self.state
            .lock()
            .unwrap()
            .config
            .as_ref()
            .map(|c| c.interfaces.clone())
            .unwrap_or_default()
    }
}

impl Default for ConfigService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn empty_config_service() {
        let svc = ConfigService::new();
        assert!(!svc.is_loaded());
        assert!(svc.config_path().is_none());
        assert!(svc.tcp_client_endpoints().is_empty());
        assert!(svc.interfaces().is_empty());
    }

    #[test]
    fn reload_without_path_fails() {
        let svc = ConfigService::new();
        assert!(svc.reload().is_err());
    }

    #[test]
    fn load_from_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[[interfaces]]
type = "tcp_client"
enabled = true
host = "10.0.0.1"
port = 4242
name = "hub"
"#
        )
        .unwrap();

        let svc = ConfigService::with_path(&path).unwrap();
        assert!(svc.is_loaded());
        assert_eq!(svc.tcp_client_endpoints(), vec![("10.0.0.1".into(), 4242)]);
        assert_eq!(svc.interfaces().len(), 1);
    }

    #[test]
    fn reload_picks_up_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();

        let svc = ConfigService::with_path(&path).unwrap();
        assert!(svc.interfaces().is_empty());

        // Write new config
        std::fs::write(
            &path,
            r#"
[[interfaces]]
type = "tcp_server"
enabled = true
host = "0.0.0.0"
port = 4242
"#,
        )
        .unwrap();

        svc.reload().unwrap();
        assert_eq!(svc.interfaces().len(), 1);
    }

    #[test]
    fn load_on_empty_service() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
role = "hub"

[[interfaces]]
type = "tcp_server"
enabled = true
host = "0.0.0.0"
port = 4242
"#,
        )
        .unwrap();

        let svc = ConfigService::new();
        assert!(!svc.is_loaded());

        svc.load(&path).unwrap();
        assert!(svc.is_loaded());
        assert_eq!(svc.config_path(), Some(path));
        assert_eq!(svc.node_role(), NodeRole::Hub);
        assert_eq!(svc.interfaces().len(), 1);
    }

    #[test]
    fn load_then_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();

        let svc = ConfigService::new();
        svc.load(&path).unwrap();
        assert_eq!(svc.node_role(), NodeRole::FullNode);

        // Overwrite with hub
        std::fs::write(&path, r#"role = "hub""#).unwrap();
        svc.reload().unwrap();
        assert_eq!(svc.node_role(), NodeRole::Hub);
    }
}
