//! Environment detection — scans the filesystem for existing Styrene, Reticulum,
//! NomadNet, Sideband, and overlay network state.
//!
//! All checks are read-only. Nothing is modified on disk.

use std::path::PathBuf;

use super::reticulum::ReticulumState;

/// Complete snapshot of the operator's environment at TUI startup.
#[derive(Debug, Default)]
pub struct EnvironmentReport {
    // ── Styrene state ───────────────────────────────────────────────────────
    /// Our own identity file exists.
    pub styrene_identity_exists: bool,
    /// Our config file exists.
    pub styrene_config_exists: bool,
    /// The daemon IPC socket exists on disk (daemon may or may not respond).
    pub daemon_socket_exists: bool,
    /// The daemon responded to a ping (set asynchronously after construction).
    pub daemon_responsive: bool,
    /// The setup_complete marker exists (wizard has been completed before).
    pub setup_complete: bool,

    // ── Reticulum ecosystem ─────────────────────────────────────────────────
    /// Parsed Reticulum state (identity, interfaces, contacts) if ~/.reticulum/ found.
    pub reticulum: Option<ReticulumState>,
    /// NomadNet storage directory found.
    pub nomadnet_dir: Option<PathBuf>,
    /// Sideband config directory found.
    pub sideband_dir: Option<PathBuf>,

    // ── Overlay networks ────────────────────────────────────────────────────
    /// Yggdrasil config file found.
    pub yggdrasil_config: Option<PathBuf>,
    /// I2P data directory found.
    pub i2p_dir: Option<PathBuf>,
}

impl EnvironmentReport {
    /// Whether the setup wizard should be shown.
    pub fn needs_wizard(&self) -> bool {
        !self.setup_complete || !self.styrene_identity_exists
    }

    /// Whether any Reticulum ecosystem tools were detected.
    pub fn has_rns_ecosystem(&self) -> bool {
        self.nomadnet_dir.is_some() || self.sideband_dir.is_some()
    }

    /// Whether any overlay networks were detected.
    pub fn has_overlay(&self) -> bool {
        self.yggdrasil_config.is_some() || self.i2p_dir.is_some()
    }

    /// Whether importable contacts exist anywhere.
    pub fn has_importable_contacts(&self) -> bool {
        if let Some(ref rns) = self.reticulum {
            if !rns.known_destinations.is_empty() {
                return true;
            }
        }
        // NomadNet conversations directory contains peer hashes as subdirs
        if let Some(ref dir) = self.nomadnet_dir {
            if dir.join("storage").join("conversations").is_dir() {
                return true;
            }
        }
        false
    }

    /// Human-readable summary lines for the Welcome screen.
    pub fn summary_lines(&self) -> Vec<(bool, String)> {
        let mut lines = Vec::new();

        lines.push((
            self.styrene_identity_exists,
            if self.styrene_identity_exists {
                "Styrene identity found".into()
            } else {
                "No existing Styrene identity".into()
            },
        ));

        if self.daemon_responsive {
            lines.push((true, "Daemon is running".into()));
        } else if self.daemon_socket_exists {
            lines.push((false, "Daemon socket found but not responding".into()));
        }

        if let Some(ref rns) = self.reticulum {
            let n = rns.interfaces.len();
            let iface_desc = if n == 1 { "1 interface" } else { &format!("{n} interfaces") };
            lines.push((true, format!("Reticulum config with {iface_desc}")));
            if let Some(ref hash) = rns.identity_hash {
                lines.push((true, format!("Reticulum identity ({hash})")));
            }
            if !rns.known_destinations.is_empty() {
                let n = rns.known_destinations.len();
                let noun = if n == 1 { "contact" } else { "contacts" };
                lines.push((true, format!("{n} known {noun} in Reticulum")));
            }
        } else {
            lines.push((false, "No Reticulum config detected".into()));
        }

        if let Some(ref dir) = self.nomadnet_dir {
            lines.push((true, format!("NomadNet found ({})", dir.display())));
        }
        if let Some(ref dir) = self.sideband_dir {
            lines.push((true, format!("Sideband found ({})", dir.display())));
        }
        if let Some(ref path) = self.yggdrasil_config {
            lines.push((true, format!("Yggdrasil config ({})", path.display())));
        }
        if let Some(ref dir) = self.i2p_dir {
            lines.push((true, format!("I2P found ({})", dir.display())));
        }

        lines
    }
}

/// Scan the local filesystem for all detectable state. Synchronous — runs
/// before any wizard UI. The `daemon_responsive` field is left false; the
/// caller should set it after an async ping attempt.
pub fn scan_environment() -> EnvironmentReport {
    let mut report = EnvironmentReport::default();

    // ── Styrene ─────────────────────────────────────────────────────────────
    report.styrene_identity_exists = styrened::config::default_identity_path().exists();
    report.styrene_config_exists = styrened::config::default_config_path().exists();
    report.setup_complete = setup_complete_path().exists();

    let socket_path = styrene_ipc_server::default_socket_path();
    report.daemon_socket_exists = socket_path.exists();

    // ── Reticulum ───────────────────────────────────────────────────────────
    let rns_dir = home_dir().join(".reticulum");
    if rns_dir.is_dir() {
        report.reticulum = Some(super::reticulum::scan_reticulum(&rns_dir));
    }

    // ── NomadNet ────────────────────────────────────────────────────────────
    let nomad_dir = home_dir().join(".nomadnetwork");
    if nomad_dir.is_dir() {
        report.nomadnet_dir = Some(nomad_dir);
    }

    // ── Sideband ────────────────────────────────────────────────────────────
    let sideband = sideband_dir();
    if sideband.is_dir() {
        report.sideband_dir = Some(sideband);
    }

    // ── Yggdrasil ───────────────────────────────────────────────────────────
    for path in &[PathBuf::from("/etc/yggdrasil.conf"), home_dir().join(".yggdrasil.conf")] {
        if path.is_file() {
            report.yggdrasil_config = Some(path.clone());
            break;
        }
    }
    // Also check if yggdrasil binary is on PATH
    if report.yggdrasil_config.is_none() {
        if let Ok(output) = std::process::Command::new("which").arg("yggdrasil").output() {
            if output.status.success() {
                // Binary exists but no config found — still note it
                let bin = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !bin.is_empty() {
                    report.yggdrasil_config = Some(PathBuf::from(bin));
                }
            }
        }
    }

    // ── I2P ─────────────────────────────────────────────────────────────────
    let i2pd_dir = home_dir().join(".i2pd");
    if i2pd_dir.is_dir() {
        report.i2p_dir = Some(i2pd_dir);
    }

    report
}

/// Path to the setup-complete marker file.
pub fn setup_complete_path() -> PathBuf {
    styrened::config::default_config_dir().join("setup_complete")
}

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

fn sideband_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home_dir().join("Library").join("Application Support").join("Sideband")
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("sideband");
        }
        home_dir().join(".config").join("sideband")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_wizard_when_no_identity() {
        let report = EnvironmentReport {
            styrene_identity_exists: false,
            setup_complete: true,
            ..Default::default()
        };
        assert!(report.needs_wizard());
    }

    #[test]
    fn needs_wizard_when_not_complete() {
        let report = EnvironmentReport {
            styrene_identity_exists: true,
            setup_complete: false,
            ..Default::default()
        };
        assert!(report.needs_wizard());
    }

    #[test]
    fn skip_wizard_when_complete_and_identity_exists() {
        let report = EnvironmentReport {
            styrene_identity_exists: true,
            setup_complete: true,
            ..Default::default()
        };
        assert!(!report.needs_wizard());
    }

    #[test]
    fn summary_lines_fresh_install() {
        let report = EnvironmentReport::default();
        let lines = report.summary_lines();
        assert!(!lines.is_empty());
        // First line should note missing identity
        assert!(!lines[0].0);
        assert!(lines[0].1.contains("No existing"));
    }
}
