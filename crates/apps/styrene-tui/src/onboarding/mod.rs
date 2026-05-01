//! Operator onboarding — first-run setup wizard.
//!
//! Detects the user's environment (existing Styrene install, Reticulum config,
//! NomadNet/Sideband, overlay networks) and guides them through identity setup,
//! network configuration, and daemon startup.
#![allow(dead_code)] // Public API surface — items used by main.rs integration + future consumers

pub mod detect;
pub mod reticulum;
pub mod screens;
pub mod setup;
pub mod wizard;

pub use setup::DaemonMode;
pub use wizard::{WizardAction, WizardState};

use std::path::PathBuf;

/// Load saved TUI preferences from a previous wizard run.
pub fn load_tui_prefs() -> TuiPrefs {
    let path = tui_prefs_path();
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    toml::from_str(&content).unwrap_or_default()
}

fn tui_prefs_path() -> PathBuf {
    styrened::config::default_config_dir().join("tui.toml")
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct TuiPrefs {
    #[serde(default)]
    pub daemon_mode: Option<String>,
}

impl TuiPrefs {
    pub fn daemon_mode_or_default(&self) -> DaemonMode {
        match self.daemon_mode.as_deref() {
            Some("embedded") => DaemonMode::Embedded,
            Some("background") => DaemonMode::Background,
            Some("connect") => DaemonMode::ConnectExisting,
            _ => DaemonMode::Embedded,
        }
    }
}
