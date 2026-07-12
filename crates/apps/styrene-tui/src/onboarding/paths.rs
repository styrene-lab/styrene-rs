//! Explicit filesystem boundary for installation and first-run state.

use std::path::{Path, PathBuf};

use crate::runtime::RuntimeProfile;

/// All paths used by TUI environment detection and setup.
///
/// Production uses platform defaults. Tests and alternate profiles construct
/// this directly, avoiding process-global HOME/XDG mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyrenePaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub daemon_socket: PathBuf,
    pub home_dir: PathBuf,
}

impl StyrenePaths {
    pub fn new(
        config_dir: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        daemon_socket: impl Into<PathBuf>,
        home_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            config_dir: config_dir.into(),
            data_dir: data_dir.into(),
            daemon_socket: daemon_socket.into(),
            home_dir: home_dir.into(),
        }
    }

    pub fn from_defaults() -> Self {
        Self {
            config_dir: styrened::config::default_config_dir(),
            data_dir: styrened::config::default_data_dir(),
            daemon_socket: styrene_ipc_server::default_socket_path(),
            home_dir: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".")),
        }
    }

    pub fn for_profile(profile: &RuntimeProfile) -> Result<Self, String> {
        match profile {
            RuntimeProfile::Standard => Ok(Self::from_defaults()),
            RuntimeProfile::Portable { root } => Ok(Self::portable(root)),
            RuntimeProfile::Ghost => Self::ghost(None),
            RuntimeProfile::PortableGhost { root } => Self::ghost(Some(root)),
        }
    }

    fn portable(root: &Path) -> Self {
        let home = root.join("home");
        Self::new(root.join("config"), root.join("data"), root.join("run/styrene.sock"), home)
    }

    fn ghost(portable_root: Option<&PathBuf>) -> Result<Self, String> {
        let base = if let Some(root) = portable_root {
            root.join(".ghost")
        } else {
            std::env::temp_dir().join(format!("styrene-ghost-{}", std::process::id()))
        };
        std::fs::create_dir_all(&base)
            .map_err(|error| format!("create ghost runtime {}: {error}", base.display()))?;
        Ok(Self::new(
            base.join("config"),
            base.join("data"),
            base.join("run/styrene.sock"),
            base.join("home"),
        ))
    }

    pub fn config_path(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn identity_path(&self) -> PathBuf {
        self.config_dir.join("identity")
    }

    pub fn profile_path(&self) -> PathBuf {
        self.config_dir.join("profile.toml")
    }

    pub fn tui_preferences_path(&self) -> PathBuf {
        self.config_dir.join("tui.toml")
    }

    pub fn setup_complete_path(&self) -> PathBuf {
        self.config_dir.join("setup_complete")
    }

    pub fn reticulum_dir(&self) -> PathBuf {
        self.home_dir.join(".reticulum")
    }

    pub fn nomadnet_dir(&self) -> PathBuf {
        self.home_dir.join(".nomadnetwork")
    }

    pub fn i2p_dir(&self) -> PathBuf {
        self.home_dir.join(".i2pd")
    }

    pub fn sideband_dir(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            self.home_dir.join("Library").join("Application Support").join("Sideband")
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.home_dir.join(".config").join("sideband")
        }
    }

    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }
}

impl Default for StyrenePaths {
    fn default() -> Self {
        Self::from_defaults()
    }
}

#[derive(Debug, Clone)]
pub struct TuiOptions {
    pub paths: StyrenePaths,
    pub runtime_profile: RuntimeProfile,
}

impl Default for TuiOptions {
    fn default() -> Self {
        Self { paths: StyrenePaths::default(), runtime_profile: RuntimeProfile::Standard }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_paths_derive_installation_files() {
        let paths = StyrenePaths::new("/cfg", "/data", "/run/styrene.sock", "/home/test");
        assert_eq!(paths.config_path(), PathBuf::from("/cfg/config.toml"));
        assert_eq!(paths.identity_path(), PathBuf::from("/cfg/identity"));
        assert_eq!(paths.setup_complete_path(), PathBuf::from("/cfg/setup_complete"));
        assert_eq!(paths.reticulum_dir(), PathBuf::from("/home/test/.reticulum"));
    }
}
