//! ConfigService — configuration loading and persistence.
//!
//! Owns: 13.1 config load/save, 13.3 hardware/system info.
//! Package: E
//!
//! Note: this wraps the `DaemonConfig` model from `crate::config`,
//! adding service-layer operations (load, reload, interface enumeration).

use crate::config::{DaemonConfig, InterfaceConfig};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Service-layer configuration management.
pub struct ConfigService {
    config_path: Option<PathBuf>,
    config: Mutex<Option<DaemonConfig>>,
}

impl ConfigService {
    /// Create a new ConfigService. Optionally loads from the given path.
    pub fn with_path(path: &Path) -> Result<Self, std::io::Error> {
        let config = DaemonConfig::from_path(path)?;
        Ok(Self {
            config_path: Some(path.to_path_buf()),
            config: Mutex::new(Some(config)),
        })
    }

    /// Create an empty ConfigService (no config file).
    pub fn new() -> Self {
        Self {
            config_path: None,
            config: Mutex::new(None),
        }
    }

    /// Reload configuration from disk.
    pub fn reload(&self) -> Result<(), std::io::Error> {
        let Some(path) = &self.config_path else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no config path set",
            ));
        };
        let config = DaemonConfig::from_path(path)?;
        *self.config.lock().unwrap() = Some(config);
        Ok(())
    }

    /// Get the config path, if set.
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// Check if a config is loaded.
    pub fn is_loaded(&self) -> bool {
        self.config.lock().unwrap().is_some()
    }

    /// Get the list of configured TCP client endpoints.
    pub fn tcp_client_endpoints(&self) -> Vec<(String, u16)> {
        self.config
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.tcp_client_endpoints())
            .unwrap_or_default()
    }

    /// Get all configured interfaces.
    pub fn interfaces(&self) -> Vec<InterfaceConfig> {
        self.config
            .lock()
            .unwrap()
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
}
