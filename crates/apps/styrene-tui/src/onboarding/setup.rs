//! Setup execution — applies wizard choices to an explicit filesystem root.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::paths::StyrenePaths;

/// How the daemon should be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonMode {
    Embedded,
    Background,
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
    CreateNew,
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
    /// Apply all wizard choices. The completion marker is committed last, so a
    /// crash or validation error causes onboarding to resume on the next run.
    pub fn apply(&self, paths: &StyrenePaths) -> io::Result<()> {
        fs::create_dir_all(&paths.config_dir)?;
        fs::create_dir_all(&paths.data_dir)?;
        set_private_directory(&paths.config_dir)?;
        set_private_directory(&paths.data_dir)?;

        match &self.identity_source {
            IdentitySource::CreateNew => {
                styrened::identity_store::load_or_create_identity(&paths.identity_path())
                    .map_err(|error| io::Error::other(error.to_string()))?;
            }
            IdentitySource::ImportReticulum(source) => {
                let bytes = fs::read(source)?;
                validate_identity_bytes(&bytes)?;
                atomic_write(&paths.identity_path(), &bytes, true)?;
            }
        }

        let config = styrened::config::DaemonConfig {
            interfaces: self.interfaces.clone(),
            role: self.node_role,
            rbac: None,
        };
        let config_toml =
            toml::to_string_pretty(&config).map_err(|error| io::Error::other(error.to_string()))?;
        // Parse before commit so the persisted config is known to be consumable.
        toml::from_str::<styrened::config::DaemonConfig>(&config_toml)
            .map_err(|error| io::Error::other(error.to_string()))?;
        atomic_write(&paths.config_path(), config_toml.as_bytes(), true)?;

        if !self.display_name.is_empty() {
            let profile = format!("display_name = {:?}\n", self.display_name);
            atomic_write(&paths.profile_path(), profile.as_bytes(), true)?;
        }

        let preferences = format!("daemon_mode = {:?}\n", self.daemon_mode.as_str());
        atomic_write(&paths.tui_preferences_path(), preferences.as_bytes(), true)?;

        // Commit point. Never write this until every required artifact exists.
        atomic_write(&paths.setup_complete_path(), b"", true)
    }
}

fn validate_identity_bytes(bytes: &[u8]) -> io::Result<()> {
    if bytes.len() != 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Reticulum identity must be exactly 64 bytes, got {}", bytes.len()),
        ));
    }
    rns_core::identity::PrivateIdentity::from_private_key_bytes(bytes)
        .map(|_| ())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}")))
}

fn atomic_write(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::other("path has no parent"))?;
    fs::create_dir_all(parent)?;
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("state");
    let temporary = parent.join(format!(".{name}.tmp-{}", std::process::id()));

    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        sync_directory(parent)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sandbox() -> (PathBuf, StyrenePaths) {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root =
            std::env::temp_dir().join(format!("styrene-setup-{}-{nonce}", std::process::id()));
        let paths = StyrenePaths::new(
            root.join("config"),
            root.join("data"),
            root.join("run/styrene.sock"),
            root.join("home"),
        );
        (root, paths)
    }

    fn result(source: IdentitySource) -> SetupResult {
        SetupResult {
            identity_source: source,
            display_name: "Hermetic Node".into(),
            node_role: styrened::config::NodeRole::FullNode,
            interfaces: Vec::new(),
            daemon_mode: DaemonMode::Embedded,
            contacts: Vec::new(),
        }
    }

    #[test]
    fn fresh_setup_is_complete_and_reparseable() {
        let (root, paths) = sandbox();
        result(IdentitySource::CreateNew).apply(&paths).unwrap();
        assert!(paths.identity_path().is_file());
        assert!(paths.setup_complete_path().is_file());
        styrened::config::DaemonConfig::from_path(&paths.config_path()).unwrap();
        let _ = styrened::identity_store::load_or_create_identity(&paths.identity_path()).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_import_does_not_commit_setup() {
        let (root, paths) = sandbox();
        fs::create_dir_all(&root).unwrap();
        let source = root.join("invalid.identity");
        fs::write(&source, b"not an identity").unwrap();
        let error = result(IdentitySource::ImportReticulum(source)).apply(&paths).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!paths.identity_path().exists());
        assert!(!paths.setup_complete_path().exists());
        fs::remove_dir_all(root).unwrap();
    }
}
