//! Persistent preferences for otherwise ephemeral ghost sessions.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GhostPreferences {
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub interfaces: Vec<styrened::config::InterfaceConfig>,
}

impl GhostPreferences {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| toml::from_str(&content).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).map_err(io::Error::other)?;
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&temporary, content)?;
        std::fs::rename(temporary, path)
    }

    pub fn write_session_config(&self, path: &Path) -> io::Result<()> {
        let config = styrened::config::DaemonConfig {
            interfaces: self.interfaces.clone(),
            role: styrened::config::NodeRole::FullNode,
            rbac: None,
        };
        let content = toml::to_string_pretty(&config).map_err(io::Error::other)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_without_identity() {
        let root = std::env::temp_dir().join(format!("styrene-ghost-prefs-{}", std::process::id()));
        let path = root.join("ghost.toml");
        let expected = GhostPreferences { display_name: "Wraith".into(), interfaces: Vec::new() };
        expected.save(&path).unwrap();
        let actual = GhostPreferences::load(&path);
        assert_eq!(actual.display_name, expected.display_name);
        assert!(actual.interfaces.is_empty());
        assert!(!root.join("identity").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
