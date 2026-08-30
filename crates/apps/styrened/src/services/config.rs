//! ConfigService — configuration loading and persistence.
//!
//! Owns: 13.1 config load/save, 13.3 hardware/system info.
//! Package: E
//!
//! Note: this wraps the `DaemonConfig` model from `crate::config`,
//! adding service-layer operations (load, reload, interface enumeration).

use crate::config::{AutoReplySettings, DaemonConfig, InterfaceConfig, NodeRole};
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
            state: Mutex::new(ConfigState { path: Some(path.to_path_buf()), config: Some(config) }),
        })
    }

    /// Create an empty ConfigService (no config file).
    pub fn new() -> Self {
        Self { state: Mutex::new(ConfigState { path: None, config: None }) }
    }

    /// Load configuration from a path (sets both path and config atomically).
    pub fn load(&self, path: &Path) -> Result<(), std::io::Error> {
        let config = DaemonConfig::from_path(path)?;
        let mut s = self.state.lock().unwrap();
        s.path = Some(path.to_path_buf());
        s.config = Some(config);
        Ok(())
    }

    /// Set the durable configuration path, loading it when present.
    pub fn load_or_default(&self, path: &Path) -> Result<(), std::io::Error> {
        let config =
            if path.exists() { DaemonConfig::from_path(path)? } else { DaemonConfig::default() };
        let mut state = self.state.lock().unwrap();
        state.path = Some(path.to_path_buf());
        state.config = Some(config);
        Ok(())
    }

    /// Reload configuration from disk.
    pub fn reload(&self) -> Result<(), std::io::Error> {
        let path = {
            let s = self.state.lock().unwrap();
            s.path.clone()
        };
        let Some(path) = path else {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no config path set"));
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
        self.state.lock().unwrap().config.as_ref().map(|c| c.role).unwrap_or_default()
    }

    pub fn auto_reply(&self) -> AutoReplySettings {
        self.state
            .lock()
            .unwrap()
            .config
            .as_ref()
            .map(|config| config.auto_reply.clone())
            .unwrap_or_default()
    }

    pub fn set_auto_reply(&self, settings: AutoReplySettings) -> Result<(), std::io::Error> {
        let mut state = self.state.lock().unwrap();
        if state.path.is_none() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no config path set"));
        }
        let config = state.config.get_or_insert_with(DaemonConfig::default);
        let previous = std::mem::replace(&mut config.auto_reply, settings);
        if let (Some(path), Some(config)) = (state.path.as_ref(), state.config.as_ref()) {
            let result = toml::to_string_pretty(config)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
                .and_then(|bytes| crate::config::atomic_write_private(path, bytes.as_bytes()));
            if let Err(error) = result {
                state.config.as_mut().expect("config initialized above").auto_reply = previous;
                return Err(error);
            }
        }
        Ok(())
    }

    /// Save the current configuration to disk.
    ///
    /// Writes the in-memory `DaemonConfig` to the stored path as TOML.
    /// Returns an error if no path is set or no config is loaded.
    pub fn save(&self) -> Result<(), std::io::Error> {
        let s = self.state.lock().unwrap();
        let path = s.path.as_ref().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no config path set")
        })?;
        let config = s
            .config
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no config loaded"))?;
        let toml_str = toml::to_string_pretty(config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, toml_str)
    }

    /// Update a config field from a ConfigSnapshot and save.
    pub fn apply_snapshot(
        &self,
        snapshot: &styrene_ipc::types::ConfigSnapshot,
    ) -> Result<(), std::io::Error> {
        let mut s = self.state.lock().unwrap();
        let config = s.config.get_or_insert_with(DaemonConfig::default);

        // Apply known fields from the snapshot
        if let Some(role_val) = snapshot.values.get("role")
            && let Some(role_str) = role_val.as_str()
        {
            config.role = match role_str {
                "hub" => NodeRole::Hub,
                "propagation_client" => NodeRole::PropagationClient,
                _ => NodeRole::FullNode,
            };
        }

        // Save to disk if path is set
        let path = s.path.clone();
        if let (Some(path), Some(config)) = (path, s.config.as_ref()) {
            let toml_str = toml::to_string_pretty(config)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            std::fs::write(path, toml_str)?;
        }

        Ok(())
    }

    /// Get all configured interfaces.
    pub fn interfaces(&self) -> Vec<InterfaceConfig> {
        self.state.lock().unwrap().config.as_ref().map(|c| c.interfaces.clone()).unwrap_or_default()
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

    #[test]
    fn auto_reply_update_is_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        let service = ConfigService::with_path(&path).unwrap();
        service
            .set_auto_reply(AutoReplySettings {
                mode: crate::config::AutoReplySettingMode::Echo,
                message: String::new(),
                cooldown_secs: 0,
            })
            .unwrap();
        assert_eq!(
            DaemonConfig::from_path(path).unwrap().auto_reply.mode,
            crate::config::AutoReplySettingMode::Echo
        );
    }

    #[test]
    fn first_boot_path_persists_auto_reply_and_missing_path_is_rejected() {
        let settings = AutoReplySettings {
            mode: crate::config::AutoReplySettingMode::Echo,
            message: String::new(),
            cooldown_secs: 0,
        };
        let service = ConfigService::new();
        assert_eq!(
            service.set_auto_reply(settings.clone()).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        service.load_or_default(&path).unwrap();
        service.set_auto_reply(settings).unwrap();

        assert_eq!(
            DaemonConfig::from_path(path).unwrap().auto_reply.mode,
            crate::config::AutoReplySettingMode::Echo
        );
    }

    #[test]
    fn failed_auto_reply_persistence_preserves_in_memory_setting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        let service = ConfigService::with_path(&path).unwrap();
        let previous = service.auto_reply();
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(dir.path()).unwrap();
        std::fs::write(dir.path(), "not a directory").unwrap();

        assert!(
            service
                .set_auto_reply(AutoReplySettings {
                    mode: crate::config::AutoReplySettingMode::Echo,
                    message: String::new(),
                    cooldown_secs: 0,
                })
                .is_err()
        );
        assert_eq!(service.auto_reply(), previous);
    }
}
