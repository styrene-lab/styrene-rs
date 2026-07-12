//! Explicit filesystem boundary for installation and first-run state.

use std::path::{Path, PathBuf};

use crate::runtime::RuntimeProfile;

const GHOST_MARKER: &str = ".styrene-ghost";
const GHOST_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

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
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let session = format!("{}-{nonce}", std::process::id());
        let (parent, base) = if let Some(root) = portable_root {
            let parent = root.join(".ghost");
            let base = parent.join(session);
            (parent, base)
        } else {
            let parent = std::env::temp_dir();
            let base = parent.join(format!("styrene-ghost-{session}"));
            (parent, base)
        };
        scavenge_abandoned_ghosts(&parent, portable_root.is_some());
        std::fs::create_dir_all(&base)
            .map_err(|error| format!("create ghost runtime {}: {error}", base.display()))?;
        set_private_directory(&base)
            .map_err(|error| format!("secure ghost runtime {}: {error}", base.display()))?;
        std::fs::write(base.join(GHOST_MARKER), b"styrene ghost runtime\n")
            .map_err(|error| format!("mark ghost runtime {}: {error}", base.display()))?;
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

fn scavenge_abandoned_ghosts(parent: &Path, portable: bool) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let name_matches = entry
            .file_name()
            .to_str()
            .is_some_and(|name| portable || name.starts_with("styrene-ghost-"));
        if !name_matches || !path.is_dir() || !path.join(GHOST_MARKER).is_file() {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= GHOST_MAX_AGE);
        if stale {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
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
    fn scavenger_removes_only_old_marked_directories() {
        let parent = std::env::temp_dir().join(format!("styrene-scavenge-{}", std::process::id()));
        let marked = parent.join("old");
        let unmarked = parent.join("unmarked");
        std::fs::create_dir_all(&marked).unwrap();
        std::fs::create_dir_all(&unmarked).unwrap();
        std::fs::write(marked.join(GHOST_MARKER), []).unwrap();

        // A fresh marked directory is retained. The age gate is intentionally
        // not bypassable in production; deletion mechanics are covered by the
        // GhostSession drop test.
        scavenge_abandoned_ghosts(&parent, true);
        assert!(marked.exists());
        assert!(unmarked.exists());
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn explicit_paths_derive_installation_files() {
        let paths = StyrenePaths::new("/cfg", "/data", "/run/styrene.sock", "/home/test");
        assert_eq!(paths.config_path(), PathBuf::from("/cfg/config.toml"));
        assert_eq!(paths.identity_path(), PathBuf::from("/cfg/identity"));
        assert_eq!(paths.setup_complete_path(), PathBuf::from("/cfg/setup_complete"));
        assert_eq!(paths.reticulum_dir(), PathBuf::from("/home/test/.reticulum"));
    }
}
