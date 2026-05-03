//! Setup execution — applies wizard choices to the filesystem.

use std::path::PathBuf;

/// How the daemon should be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonMode {
    /// Run daemon in-process (same tokio runtime as TUI).
    Embedded,
    /// Spawn `styrened` as a child process, connect via IPC.
    Background,
    /// Connect to an already-running daemon.
    ConnectExisting,
}

impl DaemonMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::Background => "background",
            Self::ConnectExisting => "connect",
        }
    }
}

/// Where the identity should come from.
#[derive(Debug, Clone)]
pub enum IdentitySource {
    /// Generate a new random identity.
    CreateNew,
    /// Import from an existing Reticulum identity file.
    ImportReticulum(PathBuf),
}

/// Collected wizard results, ready to be applied to the filesystem.
#[derive(Debug)]
pub struct SetupResult {
    pub identity_source: IdentitySource,
    pub display_name: String,
    pub node_role: styrened::config::NodeRole,
    pub interfaces: Vec<styrened::config::InterfaceConfig>,
    pub daemon_mode: DaemonMode,
    pub contacts: Vec<(String, String)>,
}

impl SetupResult {
    /// Apply all wizard choices: write identity, config, profile, marker.
    pub fn apply(&self) -> Result<(), std::io::Error> {
        use std::fs;

        let config_dir = styrened::config::default_config_dir();
        let data_dir = styrened::config::default_data_dir();
        fs::create_dir_all(&config_dir)?;
        fs::create_dir_all(&data_dir)?;

        // ── Identity ────────────────────────────────────────────────────────
        match &self.identity_source {
            IdentitySource::CreateNew => {
                let path = styrened::config::default_identity_path();
                // load_or_create_identity creates if missing
                styrened::identity_store::load_or_create_identity(&path)
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
            }
            IdentitySource::ImportReticulum(src) => {
                let dest = styrened::config::default_identity_path();
                let bytes = fs::read(src)?;
                // Atomic write with secure permissions (same as identity_store)
                let tmp = dest.with_extension("import");
                fs::write(&tmp, &bytes)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
                }
                fs::rename(&tmp, &dest)?;
            }
        }

        // ── Config ──────────────────────────────────────────────────────────
        let config = styrened::config::DaemonConfig {
            interfaces: self.interfaces.clone(),
            role: self.node_role,
            rbac: None,
        };
        let config_toml =
            toml::to_string_pretty(&config).map_err(|e| std::io::Error::other(e.to_string()))?;
        fs::write(config_dir.join("config.toml"), config_toml)?;

        // ── Profile ─────────────────────────────────────────────────────────
        if !self.display_name.is_empty() {
            let profile = format!("display_name = {:?}\n", self.display_name);
            fs::write(config_dir.join("profile.toml"), profile)?;
        }

        // ── TUI preferences ────────────────────────────────────────────────
        let tui_prefs = format!("daemon_mode = {:?}\n", self.daemon_mode.as_str());
        fs::write(config_dir.join("tui.toml"), tui_prefs)?;

        // ── Setup complete marker ───────────────────────────────────────────
        fs::write(config_dir.join("setup_complete"), "")?;

        Ok(())
    }
}
