use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rand_core::{OsRng, RngCore};
use rns_core::identity::PrivateIdentity;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::atomic_write_private;
use crate::identity_store::load_or_create_identity;

const PROFILE_FORMAT_VERSION: u32 = 1;
const MAX_COPY_ENTRIES: usize = 100_000;
const MAX_COPY_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;
const PRIVATE_IDENTITY_BYTES: u64 = 64;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileStorage {
    Quick,
    Local,
    /// Encrypted removable media, resolved by a stable selector.
    Portable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileManifest {
    pub format_version: u32,
    pub id: String,
    pub display_name: String,
    pub storage: ProfileStorage,
    pub generation: u64,
    pub created_at_unix: u64,
    /// Present only for Portable storage: how the media is recognised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portable: Option<PortableBinding>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ProfilePaths {
    pub root: PathBuf,
    pub manifest: PathBuf,
    pub config: PathBuf,
    pub public_identity: PathBuf,
    pub pages: PathBuf,
    pub identity: PathBuf,
    pub custody: PathBuf,
    pub messages: PathBuf,
    pub nodes: PathBuf,
    pub files: PathBuf,
    pub snapshots: PathBuf,
    pub runtime_root: PathBuf,
    pub socket: PathBuf,
}

impl ProfilePaths {
    fn for_roots(root: PathBuf, runtime_root: PathBuf) -> Self {
        Self {
            manifest: root.join("manifest.toml"),
            config: root.join("config/config.toml"),
            public_identity: root.join("config/public-identity.toml"),
            pages: root.join("config/pages"),
            identity: root.join("identity/identity"),
            custody: root.join("identity/custody.toml"),
            messages: root.join("data/messages.db"),
            nodes: root.join("data/nodes.db"),
            files: root.join("data/files"),
            snapshots: root.join("snapshots/generations"),
            socket: runtime_root.join("ipc.sock"),
            root,
            runtime_root,
        }
    }

    fn durable_entries(&self) -> [PathBuf; 10] {
        [
            self.manifest.clone(),
            self.config.parent().expect("config path has parent").to_path_buf(),
            self.pages.clone(),
            self.identity.parent().expect("identity path has parent").to_path_buf(),
            self.root.join("data"),
            self.files.clone(),
            self.snapshots.clone(),
            self.config.clone(),
            self.identity.clone(),
            self.custody.clone(),
        ]
    }
}

#[derive(Debug)]
pub struct StoppedManagedProfile {
    manifest: ProfileManifest,
    paths: ProfilePaths,
    cleanup_durable_on_drop: bool,
    _lease: File,
}

#[derive(Debug)]
pub struct PendingPromotion {
    profile: Option<StoppedManagedProfile>,
    source: Option<StoppedManagedProfile>,
}

#[must_use = "a running managed profile must be shut down explicitly"]
pub struct RunningManagedProfile {
    daemon: Option<crate::daemon::DaemonHandle>,
    profile: Option<StoppedManagedProfile>,
}

#[must_use = "a running promotion must be shut down explicitly"]
pub struct RunningPromotion {
    running: Option<RunningManagedProfile>,
    source: Option<StoppedManagedProfile>,
}

impl fmt::Debug for RunningManagedProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunningManagedProfile")
            .field("profile", &self.profile)
            .field("daemon", &self.daemon.as_ref().map(|_| "running"))
            .finish()
    }
}

impl fmt::Debug for RunningPromotion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunningPromotion")
            .field("running", &self.running)
            .field("source", &self.source)
            .finish()
    }
}

impl RunningPromotion {
    pub fn identity_hash(&self) -> &str {
        self.running.as_ref().expect("running promotion has daemon").identity_hash()
    }

    pub fn paths(&self) -> &ProfilePaths {
        self.running.as_ref().expect("running promotion has daemon").paths()
    }

    pub async fn shutdown(mut self) -> Result<StoppedManagedProfile, PromotionCleanupFailure> {
        let destination =
            self.running.take().expect("running promotion has daemon").shutdown().await;
        let mut source = self.source.take().expect("running promotion has source");
        if let Err(error) = fs::remove_dir_all(&source.paths.root) {
            return Err(PromotionCleanupFailure {
                error: ProfileError::Io { action: "remove promoted Quick source", source: error },
                source: Box::new(source),
                destination: Box::new(destination),
            });
        }
        source.cleanup_durable_on_drop = false;
        drop(source);
        Ok(destination)
    }
}

impl Drop for RunningPromotion {
    fn drop(&mut self) {
        if let Some(source) = self.source.take() {
            // RunningManagedProfile fails closed on accidental drop; retain the
            // Quick source lease and data under the same condition.
            std::mem::forget(source);
        }
    }
}

impl RunningManagedProfile {
    pub fn identity_hash(&self) -> &str {
        self.daemon
            .as_ref()
            .expect("running profile has daemon")
            .app_context
            .identity()
            .identity_hash()
    }

    pub fn paths(&self) -> &ProfilePaths {
        &self.profile.as_ref().expect("running profile has lease").paths
    }

    /// The running daemon's application context.
    pub fn app_context(&self) -> &std::sync::Arc<crate::app_context::AppContext> {
        &self.daemon.as_ref().expect("running profile has daemon").app_context
    }

    pub async fn shutdown(mut self) -> StoppedManagedProfile {
        let daemon = self.daemon.take().expect("running profile has daemon");
        let profile = self.profile.take().expect("running profile has lease");
        daemon.shutdown().await;
        profile
    }
}

impl Drop for RunningManagedProfile {
    fn drop(&mut self) {
        let Some(daemon) = self.daemon.take() else {
            return;
        };
        let profile = self.profile.take().expect("running profile has lease");
        // Async shutdown cannot be completed from Drop. Leak both owners so an
        // accidental drop fails closed instead of releasing a live profile lease.
        std::mem::forget((daemon, profile));
    }
}

#[derive(Debug)]
pub struct ManagedStartFailure {
    error: anyhow::Error,
    profile: Box<StoppedManagedProfile>,
}

impl ManagedStartFailure {
    pub fn into_profile(self) -> StoppedManagedProfile {
        *self.profile
    }

    fn into_parts(self) -> (anyhow::Error, StoppedManagedProfile) {
        (self.error, *self.profile)
    }
}

#[derive(Debug)]
pub struct PromotionRestartFailure {
    error: anyhow::Error,
    source: Box<StoppedManagedProfile>,
    destination: Option<Box<StoppedManagedProfile>>,
}

#[derive(Debug)]
pub struct PromotionCleanupFailure {
    error: ProfileError,
    source: Box<StoppedManagedProfile>,
    destination: Box<StoppedManagedProfile>,
}

impl PromotionCleanupFailure {
    pub fn into_profiles(self) -> (StoppedManagedProfile, StoppedManagedProfile) {
        (*self.source, *self.destination)
    }
}

impl fmt::Display for PromotionCleanupFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for PromotionCleanupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl PromotionRestartFailure {
    pub fn into_source(self) -> StoppedManagedProfile {
        *self.source
    }

    pub fn into_profiles(self) -> (StoppedManagedProfile, Option<StoppedManagedProfile>) {
        (*self.source, self.destination.map(|profile| *profile))
    }
}

impl fmt::Display for PromotionRestartFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for PromotionRestartFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

impl fmt::Display for ManagedStartFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for ManagedStartFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

impl PendingPromotion {
    pub fn profile(&self) -> &StoppedManagedProfile {
        self.profile.as_ref().expect("pending promotion has destination")
    }

    pub async fn start(mut self) -> Result<RunningPromotion, PromotionRestartFailure> {
        let destination = self.profile.take().expect("pending promotion has destination");
        let source = self.source.take().expect("pending promotion has source");
        let expected_identity = match profile_identity_hash(&source.paths.identity) {
            Ok(identity) => identity,
            Err(error) => {
                let (error, destination) = rollback_promoted_destination(destination, error.into());
                return Err(PromotionRestartFailure {
                    error,
                    source: Box::new(source),
                    destination: destination.map(Box::new),
                });
            }
        };
        match destination.start().await {
            Ok(running) if running.identity_hash() == expected_identity => {
                Ok(RunningPromotion { running: Some(running), source: Some(source) })
            }
            Ok(running) => {
                let actual_identity = running.identity_hash().to_owned();
                let destination = running.shutdown().await;
                let (error, destination) = rollback_promoted_destination(
                    destination,
                    anyhow::anyhow!(
                        "promoted identity changed from {expected_identity} to {actual_identity}"
                    ),
                );
                Err(PromotionRestartFailure {
                    error,
                    source: Box::new(source),
                    destination: destination.map(Box::new),
                })
            }
            Err(failure) => {
                let (error, destination) = failure.into_parts();
                let (error, destination) = rollback_promoted_destination(destination, error);
                Err(PromotionRestartFailure {
                    error,
                    source: Box::new(source),
                    destination: destination.map(Box::new),
                })
            }
        }
    }
}

impl PendingPromotion {
    /// Publish the promoted profile without restarting into it. Both the
    /// promoted destination and the Quick source stay leased by the caller,
    /// so the source is not removed until the operator restarts from the
    /// destination and releases it.
    pub fn publish_without_restart(mut self) -> (StoppedManagedProfile, StoppedManagedProfile) {
        let promoted = self.profile.take().expect("pending promotion has profile");
        let source = self.source.take().expect("pending promotion has source");
        (promoted, source)
    }
}

impl Drop for PendingPromotion {
    fn drop(&mut self) {
        if let Some(profile) = self.profile.take() {
            let destination = profile.paths.root.clone();
            drop(profile);
            let _ = fs::remove_dir_all(destination);
        }
    }
}

#[derive(Debug)]
pub struct PromotionFailure {
    error: ProfileError,
    profile: Box<StoppedManagedProfile>,
}

impl PromotionFailure {
    pub fn error(&self) -> &ProfileError {
        &self.error
    }

    pub fn into_profile(self) -> StoppedManagedProfile {
        *self.profile
    }
}

impl fmt::Display for PromotionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for PromotionFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl StoppedManagedProfile {
    pub async fn start(self) -> Result<RunningManagedProfile, ManagedStartFailure> {
        if let Err(error) = validate_profile_paths(&self.paths)
            .and_then(|()| validate_identity(&self.paths.identity))
        {
            return Err(ManagedStartFailure { error: error.into(), profile: Box::new(self) });
        }
        let paths = crate::daemon::ManagedDaemonPaths {
            db: self.paths.messages.clone(),
            nodes: self.paths.nodes.clone(),
            config: self.paths.config.clone(),
            identity: self.paths.identity.clone(),
            socket: self.paths.socket.clone(),
            pages: self.paths.pages.clone(),
            files: self.paths.files.clone(),
            display_name: self.manifest.display_name.clone(),
            profile_root: self.paths.root.clone(),
            profiles_parent: self
                .paths
                .root
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.paths.root.clone()),
            runtime_parent: self
                .paths
                .runtime_root
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.paths.runtime_root.clone()),
            profile_id: self.manifest.id.clone(),
        };
        match crate::daemon::start_managed(paths).await {
            Ok(daemon) => Ok(RunningManagedProfile { daemon: Some(daemon), profile: Some(self) }),
            Err(error) => Err(ManagedStartFailure { error, profile: Box::new(self) }),
        }
    }

    pub fn create_quick(
        profile_parent: &Path,
        runtime_parent: &Path,
        display_name: &str,
    ) -> Result<Self, ProfileError> {
        ensure_supported_platform()?;
        let parent = validate_existing_directory(profile_parent)?;
        let id = random_id();
        let root = parent.join(format!(".quick-{id}"));
        Self::create(root, runtime_parent, display_name, ProfileStorage::Quick, id)
    }

    pub fn create_local(
        root: &Path,
        runtime_parent: &Path,
        display_name: &str,
    ) -> Result<Self, ProfileError> {
        ensure_supported_platform()?;
        let id = random_id();
        Self::create(root.to_path_buf(), runtime_parent, display_name, ProfileStorage::Local, id)
    }

    pub fn open(root: &Path, runtime_parent: &Path) -> Result<Self, ProfileError> {
        ensure_supported_platform()?;
        reject_symlink(root)?;
        let root = root
            .canonicalize()
            .map_err(|source| ProfileError::Io { action: "resolve profile root", source })?;
        validate_private_directory(&root)?;
        let lease = acquire_profile_lease(&root)?;
        let manifest_path = root.join("manifest.toml");
        let manifest_bytes = read_bounded_file(&manifest_path, MAX_MANIFEST_BYTES)?;
        let manifest: ProfileManifest = toml::from_str(
            std::str::from_utf8(&manifest_bytes).map_err(|_| ProfileError::InvalidManifest)?,
        )
        .map_err(|_| ProfileError::InvalidManifest)?;
        validate_manifest(&manifest)?;
        let paths = ProfilePaths::for_roots(root, PathBuf::new());
        validate_profile_paths(&paths)?;
        validate_identity(&paths.identity)?;
        let runtime_root = create_runtime_root(runtime_parent, &manifest.id)?;
        let paths = ProfilePaths::for_roots(paths.root, runtime_root);
        let cleanup_durable_on_drop = manifest.storage == ProfileStorage::Quick;
        Ok(Self { manifest, paths, cleanup_durable_on_drop, _lease: lease })
    }

    pub fn manifest(&self) -> &ProfileManifest {
        &self.manifest
    }

    pub fn paths(&self) -> &ProfilePaths {
        &self.paths
    }

    /// Promote profile data only after the runtime that owns it has stopped.
    pub fn promote_stopped_to_local(
        self,
        destination: &Path,
        runtime_parent: &Path,
    ) -> Result<PendingPromotion, PromotionFailure> {
        match self.try_promote_stopped_to_local(destination, runtime_parent) {
            Ok(profile) => Ok(PendingPromotion { profile: Some(profile), source: Some(self) }),
            Err(error) => Err(PromotionFailure { error, profile: Box::new(self) }),
        }
    }

    fn try_promote_stopped_to_local(
        &self,
        destination: &Path,
        runtime_parent: &Path,
    ) -> Result<Self, ProfileError> {
        ensure_supported_platform()?;
        if self.manifest.storage != ProfileStorage::Quick {
            return Err(ProfileError::InvalidPromotionSource);
        }
        if destination.exists() {
            return Err(ProfileError::DestinationExists(destination.to_path_buf()));
        }
        validate_promotion_source(&self.paths)?;
        validate_identity(&self.paths.identity)?;

        let destination_parent = destination
            .parent()
            .ok_or_else(|| ProfileError::InvalidPath("destination has no parent".into()))?;
        let destination_parent = validate_existing_directory(destination_parent)?;
        let destination_name =
            destination.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
                ProfileError::InvalidPath("destination has no valid file name".into())
            })?;
        let destination = destination_parent.join(destination_name);
        let stage = destination_parent.join(format!(".{destination_name}.stage-{}", random_id()));

        let result = self.stage_promotion(&stage, &destination, runtime_parent);
        if result.is_err() {
            let _ = fs::remove_dir_all(&stage);
        }
        result
    }

    fn create(
        root: PathBuf,
        runtime_parent: &Path,
        display_name: &str,
        storage: ProfileStorage,
        id: String,
    ) -> Result<Self, ProfileError> {
        validate_display_name(display_name)?;
        if root.exists() {
            return Err(ProfileError::DestinationExists(root));
        }
        let parent = root
            .parent()
            .ok_or_else(|| ProfileError::InvalidPath("profile root has no parent".into()))?;
        let parent = validate_existing_directory(parent)?;
        let name = root
            .file_name()
            .ok_or_else(|| ProfileError::InvalidPath("profile root has no file name".into()))?;
        let root = parent.join(name);
        fs::create_dir(&root)
            .map_err(|source| ProfileError::Io { action: "create profile root", source })?;
        set_private_directory(&root)
            .map_err(|source| ProfileError::Io { action: "secure profile root", source })?;

        let creation = (|| {
            let profile_root = root
                .canonicalize()
                .map_err(|source| ProfileError::Io { action: "resolve profile root", source })?;
            let lease = acquire_profile_lease(&profile_root)?;
            let paths = ProfilePaths::for_roots(profile_root, PathBuf::new());
            create_profile_directories(&paths)?;
            atomic_write_private(&paths.config, b"")
                .map_err(|source| ProfileError::Io { action: "create profile config", source })?;
            let identity = load_or_create_identity(&paths.identity)
                .map_err(|source| ProfileError::Io { action: "create profile identity", source })?;
            write_custody(
                &paths.custody,
                &CustodyRecord::file(hex::encode(identity.address_hash().as_slice())),
            )?;
            let manifest = ProfileManifest {
                format_version: PROFILE_FORMAT_VERSION,
                id,
                display_name: display_name.trim().to_owned(),
                storage,
                generation: 1,
                created_at_unix: now_unix(),
                portable: None,
            };
            write_manifest(&paths.manifest, &manifest)?;
            sync_directory(&paths.root)
                .map_err(|source| ProfileError::Io { action: "sync profile root", source })?;
            let runtime_root = create_runtime_root(runtime_parent, &manifest.id)?;
            let paths = ProfilePaths::for_roots(paths.root, runtime_root);
            Ok(Self {
                manifest,
                paths,
                cleanup_durable_on_drop: storage == ProfileStorage::Quick,
                _lease: lease,
            })
        })();
        if creation.is_err() {
            let _ = fs::remove_dir_all(&root);
        }
        creation
    }

    fn stage_promotion(
        &self,
        stage: &Path,
        destination: &Path,
        runtime_parent: &Path,
    ) -> Result<Self, ProfileError> {
        fs::create_dir(stage)
            .map_err(|source| ProfileError::Io { action: "create promotion stage", source })?;
        set_private_directory(stage)
            .map_err(|source| ProfileError::Io { action: "secure promotion stage", source })?;
        let stage = stage
            .canonicalize()
            .map_err(|source| ProfileError::Io { action: "resolve promotion stage", source })?;
        let lease = acquire_profile_lease(&stage)?;

        let mut budget = CopyBudget::default();
        for component in ["config", "identity", "data", "snapshots"] {
            let source = self.paths.root.join(component);
            if source.exists() {
                copy_tree(&source, &stage.join(component), &mut budget)?;
            }
        }

        let manifest = ProfileManifest {
            storage: ProfileStorage::Local,
            generation: self.manifest.generation.saturating_add(1),
            portable: None,
            ..self.manifest.clone()
        };
        let stage_paths = ProfilePaths::for_roots(stage.clone(), PathBuf::new());
        validate_identity(&stage_paths.identity)?;
        write_manifest(&stage_paths.manifest, &manifest)?;
        sync_tree(&stage)?;
        sync_directory(stage.parent().expect("stage has parent"))
            .map_err(|source| ProfileError::Io { action: "sync promotion parent", source })?;

        let runtime_root = create_runtime_root(runtime_parent, &manifest.id)?;
        if let Err(error) = rename_no_replace(&stage, destination) {
            let _ = fs::remove_dir_all(&runtime_root);
            return Err(error);
        }
        if let Err(source) = sync_directory(destination.parent().expect("destination has parent")) {
            let _ = fs::remove_file(destination.join("manifest.toml"));
            let _ = fs::remove_dir_all(destination);
            let _ = fs::remove_dir_all(&runtime_root);
            return Err(ProfileError::Io { action: "sync promoted profile parent", source });
        }
        let paths = ProfilePaths::for_roots(destination.to_path_buf(), runtime_root);
        Ok(Self { manifest, paths, cleanup_durable_on_drop: false, _lease: lease })
    }
}

impl Drop for StoppedManagedProfile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.paths.runtime_root);
        if self.cleanup_durable_on_drop {
            let _ = fs::remove_dir_all(&self.paths.root);
        }
    }
}

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("profile destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("invalid profile manifest")]
    InvalidManifest,
    #[error("unsupported profile format version {0}")]
    UnsupportedFormat(u32),
    #[error("invalid profile path: {0}")]
    InvalidPath(String),
    #[error("profile path is a symbolic link: {0}")]
    SymbolicLink(PathBuf),
    #[error("profile copy exceeds the supported entry or byte limit")]
    CopyLimit,
    #[error("operator profiles are unsupported on this platform")]
    UnsupportedPlatform,
    #[error("invalid profile identity")]
    InvalidIdentity,
    #[error("profile path has insecure owner or permissions: {0}")]
    InsecurePermissions(PathBuf),
    #[error("profile file exceeds its supported size: {0}")]
    FileTooLarge(PathBuf),
    #[error("operator profile is already in use: {0}")]
    ProfileInUse(PathBuf),
    #[error("only a stopped Quick profile can be promoted")]
    InvalidPromotionSource,
    #[error("{action}: {source}")]
    Io {
        action: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("serialize profile manifest: {0}")]
    SerializeManifest(#[source] toml::ser::Error),
    #[error("snapshot component {0} does not match its recorded hash")]
    SnapshotTampered(String),
    #[error("snapshot is missing component {0}")]
    SnapshotMissingComponent(String),
    #[error("profile database: {0}")]
    Database(String),
    #[error("invalid identity custody record")]
    InvalidCustody,
    #[error("profile identity is unavailable: its private key is held outside the profile")]
    IdentityUnavailable,
    #[error("unknown recovery slot {0}")]
    UnknownRecoverySlot(String),
    #[error("identity custody is not held by hardware")]
    NotHardwareCustody,
    #[error("identity custody cryptography failed: {0}")]
    Custody(String),
    #[error("portable media is unsupported: {0}")]
    UnsupportedPortableMedia(String),
    #[error("portable profile was not found on any offered media")]
    PortableNotFound,
    #[error("portable media is no longer present")]
    MediaMissing,
}

impl ProfileError {
    /// A reportable copy: identical for value variants, and an `Io` variant
    /// re-created from its kind and message.
    pub fn clone_for_report(&self) -> Self {
        match self {
            Self::Io { action, source } => {
                Self::Io { action, source: io::Error::new(source.kind(), source.to_string()) }
            }
            Self::DestinationExists(path) => Self::DestinationExists(path.clone()),
            Self::InvalidManifest => Self::InvalidManifest,
            Self::UnsupportedFormat(version) => Self::UnsupportedFormat(*version),
            Self::InvalidPath(message) => Self::InvalidPath(message.clone()),
            Self::SymbolicLink(path) => Self::SymbolicLink(path.clone()),
            Self::CopyLimit => Self::CopyLimit,
            Self::UnsupportedPlatform => Self::UnsupportedPlatform,
            Self::InvalidIdentity => Self::InvalidIdentity,
            Self::InsecurePermissions(path) => Self::InsecurePermissions(path.clone()),
            Self::FileTooLarge(path) => Self::FileTooLarge(path.clone()),
            Self::ProfileInUse(path) => Self::ProfileInUse(path.clone()),
            Self::InvalidPromotionSource => Self::InvalidPromotionSource,
            Self::SerializeManifest(_) => Self::InvalidManifest,
            Self::SnapshotTampered(component) => Self::SnapshotTampered(component.clone()),
            Self::SnapshotMissingComponent(component) => {
                Self::SnapshotMissingComponent(component.clone())
            }
            Self::Database(message) => Self::Database(message.clone()),
            Self::InvalidCustody => Self::InvalidCustody,
            Self::IdentityUnavailable => Self::IdentityUnavailable,
            Self::UnknownRecoverySlot(id) => Self::UnknownRecoverySlot(id.clone()),
            Self::NotHardwareCustody => Self::NotHardwareCustody,
            Self::Custody(message) => Self::Custody(message.clone()),
            Self::UnsupportedPortableMedia(reason) => {
                Self::UnsupportedPortableMedia(reason.clone())
            }
            Self::PortableNotFound => Self::PortableNotFound,
            Self::MediaMissing => Self::MediaMissing,
        }
    }
}

fn validate_manifest(manifest: &ProfileManifest) -> Result<(), ProfileError> {
    if manifest.format_version != PROFILE_FORMAT_VERSION {
        return Err(ProfileError::UnsupportedFormat(manifest.format_version));
    }
    validate_display_name(&manifest.display_name)?;
    if manifest.id.len() != 32 || !manifest.id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ProfileError::InvalidManifest);
    }
    if manifest.generation == 0 {
        return Err(ProfileError::InvalidManifest);
    }
    match (manifest.storage, manifest.portable.as_ref()) {
        (ProfileStorage::Portable, Some(binding)) if binding.is_valid() => {}
        (ProfileStorage::Portable, _) => return Err(ProfileError::InvalidManifest),
        (_, Some(_)) => return Err(ProfileError::InvalidManifest),
        (_, None) => {}
    }
    Ok(())
}

fn validate_display_name(display_name: &str) -> Result<(), ProfileError> {
    let display_name = display_name.trim();
    if display_name.is_empty()
        || display_name.chars().count() > 64
        || display_name.chars().any(char::is_control)
    {
        return Err(ProfileError::InvalidManifest);
    }
    Ok(())
}

fn create_profile_directories(paths: &ProfilePaths) -> Result<(), ProfileError> {
    for path in [
        paths.config.parent().expect("config has parent"),
        paths.pages.as_path(),
        paths.identity.parent().expect("identity has parent"),
        paths.messages.parent().expect("messages has parent"),
        paths.files.as_path(),
        paths.snapshots.as_path(),
    ] {
        fs::create_dir_all(path)
            .map_err(|source| ProfileError::Io { action: "create profile directory", source })?;
        set_private_directory(path)
            .map_err(|source| ProfileError::Io { action: "secure profile directory", source })?;
    }
    Ok(())
}

fn validate_profile_paths(paths: &ProfilePaths) -> Result<(), ProfileError> {
    reject_symlink(&paths.root)?;
    let mut entries = 0;
    for path in paths.durable_entries() {
        if path.exists() {
            validate_tree(&path, &mut entries)?;
        }
        if !path.starts_with(&paths.root) {
            return Err(ProfileError::InvalidPath(format!(
                "{} escapes {}",
                path.display(),
                paths.root.display()
            )));
        }
    }
    Ok(())
}

fn validate_promotion_source(paths: &ProfilePaths) -> Result<(), ProfileError> {
    reject_symlink(&paths.root)?;
    for path in [
        paths.root.join("config"),
        paths.root.join("identity"),
        paths.root.join("data"),
        paths.root.join("snapshots"),
    ] {
        if path.exists() {
            reject_symlink(&path)?;
        }
    }
    Ok(())
}

fn validate_tree(path: &Path, entries: &mut usize) -> Result<(), ProfileError> {
    reject_symlink(path)?;
    *entries = entries.saturating_add(1);
    if *entries > MAX_COPY_ENTRIES {
        return Err(ProfileError::CopyLimit);
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| ProfileError::Io { action: "inspect profile entry", source })?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)
            .map_err(|source| ProfileError::Io { action: "read profile directory", source })?
        {
            validate_tree(
                &entry
                    .map_err(|source| ProfileError::Io {
                        action: "read profile directory entry",
                        source,
                    })?
                    .path(),
                entries,
            )?;
        }
    } else if !metadata.is_file() {
        return Err(ProfileError::InvalidPath(format!(
            "unsupported profile entry {}",
            path.display()
        )));
    }
    Ok(())
}

fn validate_identity(path: &Path) -> Result<(), ProfileError> {
    profile_identity_hash(path).map(|_| ())
}

fn profile_identity_hash(path: &Path) -> Result<String, ProfileError> {
    if matches!(fs::symlink_metadata(path), Err(ref error) if error.kind() == io::ErrorKind::NotFound)
    {
        return Err(ProfileError::IdentityUnavailable);
    }
    let bytes = read_bounded_file(path, PRIVATE_IDENTITY_BYTES)?;
    if bytes.len() as u64 != PRIVATE_IDENTITY_BYTES {
        return Err(ProfileError::InvalidIdentity);
    }
    validate_private_file(path)?;
    let identity = PrivateIdentity::from_private_key_bytes(&bytes)
        .map_err(|_| ProfileError::InvalidIdentity)?;
    Ok(hex::encode(identity.address_hash().as_slice()))
}

fn rollback_promoted_destination(
    destination: StoppedManagedProfile,
    mut error: anyhow::Error,
) -> (anyhow::Error, Option<StoppedManagedProfile>) {
    let destination_root = destination.paths.root.clone();
    if let Err(remove_error) = fs::remove_dir_all(&destination_root) {
        error = anyhow::anyhow!(
            "{error:#}; remove failed promoted destination {}: {remove_error}",
            destination_root.display()
        );
        return (error, Some(destination));
    }
    drop(destination);
    (error, None)
}

#[cfg(unix)]
fn read_bounded_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, ProfileError> {
    use rustix::fs::{Mode, OFlags, open};

    let fd = open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| ProfileError::Io {
        action: "open profile file",
        source: io::Error::from_raw_os_error(error.raw_os_error()),
    })?;
    let file = File::from(fd);
    let metadata = file
        .metadata()
        .map_err(|source| ProfileError::Io { action: "inspect profile file", source })?;
    if !metadata.is_file() {
        return Err(ProfileError::InvalidPath(format!("{} is not a regular file", path.display())));
    }
    if metadata.len() > max_bytes {
        return Err(ProfileError::FileTooLarge(path.to_path_buf()));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| ProfileError::Io { action: "read profile file", source })?;
    if bytes.len() as u64 > max_bytes {
        return Err(ProfileError::FileTooLarge(path.to_path_buf()));
    }
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_bounded_file(_path: &Path, _max_bytes: u64) -> Result<Vec<u8>, ProfileError> {
    Err(ProfileError::UnsupportedPlatform)
}

#[cfg(unix)]
fn validate_private_directory(path: &Path) -> Result<(), ProfileError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = fs::symlink_metadata(path)
        .map_err(|source| ProfileError::Io { action: "inspect profile directory", source })?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(ProfileError::InsecurePermissions(path.to_path_buf()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_directory(_path: &Path) -> Result<(), ProfileError> {
    Err(ProfileError::UnsupportedPlatform)
}

#[cfg(unix)]
fn validate_private_file(path: &Path) -> Result<(), ProfileError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = fs::symlink_metadata(path)
        .map_err(|source| ProfileError::Io { action: "inspect private profile file", source })?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(ProfileError::InsecurePermissions(path.to_path_buf()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_file(_path: &Path) -> Result<(), ProfileError> {
    Err(ProfileError::UnsupportedPlatform)
}

fn validate_existing_directory(path: &Path) -> Result<PathBuf, ProfileError> {
    reject_symlink(path)?;
    let path = path
        .canonicalize()
        .map_err(|source| ProfileError::Io { action: "resolve directory", source })?;
    if !path.is_dir() {
        return Err(ProfileError::InvalidPath(format!("{} is not a directory", path.display())));
    }
    Ok(path)
}

#[cfg(unix)]
fn acquire_profile_lease(root: &Path) -> Result<File, ProfileError> {
    use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};

    let path = root.join(".profile.lock");
    let fd = open(
        &path,
        OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|error| ProfileError::Io {
        action: "open profile lease",
        source: io::Error::from_raw_os_error(error.raw_os_error()),
    })?;
    let file = File::from(fd);
    validate_private_file(&path)?;
    flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
        let source = io::Error::from_raw_os_error(error.raw_os_error());
        if source.kind() == io::ErrorKind::WouldBlock {
            ProfileError::ProfileInUse(root.to_path_buf())
        } else {
            ProfileError::Io { action: "lock operator profile", source }
        }
    })?;
    Ok(file)
}

#[cfg(not(unix))]
fn acquire_profile_lease(_root: &Path) -> Result<File, ProfileError> {
    Err(ProfileError::UnsupportedPlatform)
}

fn reject_symlink(path: &Path) -> Result<(), ProfileError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(ProfileError::SymbolicLink(path.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ProfileError::Io { action: "inspect profile path", source }),
    }
}

fn create_runtime_root(parent: &Path, _profile_id: &str) -> Result<PathBuf, ProfileError> {
    let parent = validate_existing_directory(parent)?;
    for _ in 0..16 {
        let candidate = parent.join(&random_id()[..16]);
        match fs::create_dir(&candidate) {
            Ok(()) => {
                if let Err(source) = set_private_directory(&candidate) {
                    let _ = fs::remove_dir(&candidate);
                    return Err(ProfileError::Io {
                        action: "secure profile runtime directory",
                        source,
                    });
                }
                return candidate.canonicalize().map_err(|source| ProfileError::Io {
                    action: "resolve profile runtime directory",
                    source,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(ProfileError::Io {
                    action: "create profile runtime directory",
                    source,
                });
            }
        }
    }
    Err(ProfileError::InvalidPath("could not allocate unique runtime directory".into()))
}

#[cfg(unix)]
fn rename_no_replace(source: &Path, destination: &Path) -> Result<(), ProfileError> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE).map_err(|error| {
        let source = io::Error::from_raw_os_error(error.raw_os_error());
        if source.kind() == io::ErrorKind::AlreadyExists {
            ProfileError::DestinationExists(destination.to_path_buf())
        } else {
            ProfileError::Io { action: "commit promoted profile", source }
        }
    })
}

#[cfg(not(unix))]
fn rename_no_replace(_source: &Path, _destination: &Path) -> Result<(), ProfileError> {
    Err(ProfileError::UnsupportedPlatform)
}

#[cfg(unix)]
fn ensure_supported_platform() -> Result<(), ProfileError> {
    Ok(())
}

#[cfg(not(unix))]
fn ensure_supported_platform() -> Result<(), ProfileError> {
    Err(ProfileError::UnsupportedPlatform)
}

fn write_manifest(path: &Path, manifest: &ProfileManifest) -> Result<(), ProfileError> {
    validate_manifest(manifest)?;
    let bytes = toml::to_string_pretty(manifest).map_err(ProfileError::SerializeManifest)?;
    atomic_write_private(path, bytes.as_bytes())
        .map_err(|source| ProfileError::Io { action: "write profile manifest", source })
}

#[derive(Default)]
struct CopyBudget {
    entries: usize,
    bytes: u64,
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    budget: &mut CopyBudget,
) -> Result<(), ProfileError> {
    reject_symlink(source)?;
    let metadata = fs::symlink_metadata(source)
        .map_err(|source| ProfileError::Io { action: "inspect promotion source", source })?;
    budget.entries = budget.entries.saturating_add(1);
    if budget.entries > MAX_COPY_ENTRIES {
        return Err(ProfileError::CopyLimit);
    }
    if metadata.is_dir() {
        fs::create_dir(destination)
            .map_err(|source| ProfileError::Io { action: "create promoted directory", source })?;
        set_private_directory(destination)
            .map_err(|source| ProfileError::Io { action: "secure promoted directory", source })?;
        for entry in fs::read_dir(source)
            .map_err(|source| ProfileError::Io { action: "read promotion source", source })?
        {
            let entry = entry
                .map_err(|source| ProfileError::Io { action: "read promotion entry", source })?;
            copy_tree(&entry.path(), &destination.join(entry.file_name()), budget)?;
        }
        sync_directory(destination)
            .map_err(|source| ProfileError::Io { action: "sync promoted directory", source })?;
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(ProfileError::InvalidPath(format!(
            "unsupported profile entry {}",
            source.display()
        )));
    }
    budget.bytes = budget.bytes.saturating_add(metadata.len());
    if budget.bytes > MAX_COPY_BYTES {
        return Err(ProfileError::CopyLimit);
    }
    fs::copy(source, destination)
        .map_err(|source| ProfileError::Io { action: "copy promoted file", source })?;
    set_private_file(destination)
        .map_err(|source| ProfileError::Io { action: "secure promoted file", source })?;
    File::open(destination)
        .and_then(|file| file.sync_all())
        .map_err(|source| ProfileError::Io { action: "sync promoted file", source })
}

fn sync_tree(path: &Path) -> Result<(), ProfileError> {
    reject_symlink(path)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| ProfileError::Io { action: "inspect promoted profile", source })?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)
            .map_err(|source| ProfileError::Io { action: "read promoted profile", source })?
        {
            sync_tree(
                &entry
                    .map_err(|source| ProfileError::Io {
                        action: "read promoted profile entry",
                        source,
                    })?
                    .path(),
            )?;
        }
        sync_directory(path)
            .map_err(|source| ProfileError::Io { action: "sync promoted profile", source })?;
    } else if metadata.is_file() {
        File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|source| ProfileError::Io { action: "sync promoted profile file", source })?;
    }
    Ok(())
}

fn random_id() -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
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
fn set_private_file(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

// ── Snapshots ────────────────────────────────────────────────────────────────

const SNAPSHOT_FORMAT_VERSION: u32 = 1;
const MAX_SNAPSHOT_MANIFEST_BYTES: u64 = 1024 * 1024;

/// One hashed file inside a snapshot generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentRecord {
    pub sha256: String,
    pub bytes: u64,
}

/// What a snapshot generation captured. Component keys are paths relative to
/// the snapshot root, such as `data/messages.db`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotManifest {
    pub format_version: u32,
    pub snapshot_id: String,
    pub profile_id: String,
    pub profile_generation: u64,
    pub display_name: String,
    pub identity_hash: String,
    pub created_at_unix: u64,
    pub components: std::collections::BTreeMap<String, ComponentRecord>,
}

/// An immutable snapshot generation on disk, verified against its manifest.
#[derive(Debug)]
pub struct SnapshotRef {
    root: PathBuf,
    manifest: SnapshotManifest,
}

impl SnapshotRef {
    /// Open a snapshot and verify every recorded component hash. A snapshot
    /// with a missing or altered component is rejected.
    pub fn open(root: &Path) -> Result<Self, ProfileError> {
        reject_symlink(root)?;
        let root = root
            .canonicalize()
            .map_err(|source| ProfileError::Io { action: "resolve snapshot root", source })?;
        let manifest_bytes =
            read_bounded_file(&root.join("manifest.toml"), MAX_SNAPSHOT_MANIFEST_BYTES)?;
        let manifest: SnapshotManifest = toml::from_str(
            std::str::from_utf8(&manifest_bytes).map_err(|_| ProfileError::InvalidManifest)?,
        )
        .map_err(|_| ProfileError::InvalidManifest)?;
        if manifest.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(ProfileError::UnsupportedFormat(manifest.format_version));
        }
        let snapshot = Self { root, manifest };
        snapshot.verify()?;
        Ok(snapshot)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> &SnapshotManifest {
        &self.manifest
    }

    /// Recompute every component hash and compare with the manifest.
    pub fn verify(&self) -> Result<(), ProfileError> {
        let mut actual = std::collections::BTreeMap::new();
        hash_tree(&self.root, &self.root, &mut actual)?;
        actual.remove("manifest.toml");
        for (component, record) in &self.manifest.components {
            match actual.get(component) {
                None => return Err(ProfileError::SnapshotMissingComponent(component.clone())),
                Some(found) if found != record => {
                    return Err(ProfileError::SnapshotTampered(component.clone()));
                }
                Some(_) => {}
            }
        }
        for component in actual.keys() {
            if !self.manifest.components.contains_key(component) {
                return Err(ProfileError::SnapshotTampered(component.clone()));
            }
        }
        Ok(())
    }
}

/// How the databases of a snapshot are produced.
enum DatabaseSource<'a> {
    /// The profile is stopped: open each database read-only and back it up.
    Stopped,
    /// The profile is running: back up through the daemon's live connections.
    Running(&'a crate::app_context::AppContext),
}

impl StoppedManagedProfile {
    /// Capture one coherent snapshot generation of a stopped profile.
    pub fn snapshot(&self) -> Result<SnapshotRef, ProfileError> {
        write_snapshot(&self.paths, &self.manifest, DatabaseSource::Stopped)
    }

    /// Every verified snapshot generation of this profile, oldest first.
    pub fn snapshots(&self) -> Result<Vec<SnapshotRef>, ProfileError> {
        list_snapshots(&self.paths.snapshots)
    }

    /// Restore a verified snapshot to an unused destination as a new Local
    /// profile generation. The snapshot is only read.
    pub fn restore_snapshot(
        snapshot: &SnapshotRef,
        destination: &Path,
        runtime_parent: &Path,
    ) -> Result<Self, ProfileError> {
        ensure_supported_platform()?;
        snapshot.verify()?;
        if destination.exists() {
            return Err(ProfileError::DestinationExists(destination.to_path_buf()));
        }
        let destination_parent = destination
            .parent()
            .ok_or_else(|| ProfileError::InvalidPath("destination has no parent".into()))?;
        let destination_parent = validate_existing_directory(destination_parent)?;
        let destination_name =
            destination.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
                ProfileError::InvalidPath("destination has no valid file name".into())
            })?;
        let destination = destination_parent.join(destination_name);
        let stage = destination_parent.join(format!(".{destination_name}.restore-{}", random_id()));
        let result = restore_into_stage(snapshot, &stage, &destination, runtime_parent);
        if result.is_err() {
            let _ = fs::remove_dir_all(&stage);
        }
        result
    }
}

impl RunningManagedProfile {
    /// Capture one coherent snapshot generation while the daemon runs. The
    /// databases come from SQLite's online backup over the live connections;
    /// every other component is copied afterwards.
    pub fn snapshot(&self) -> Result<SnapshotRef, ProfileError> {
        let daemon = self.daemon.as_ref().expect("running profile has daemon");
        let profile = self.profile.as_ref().expect("running profile has profile");
        write_snapshot(
            profile.paths(),
            profile.manifest(),
            DatabaseSource::Running(&daemon.app_context),
        )
    }

    pub fn snapshots(&self) -> Result<Vec<SnapshotRef>, ProfileError> {
        let profile = self.profile.as_ref().expect("running profile has profile");
        list_snapshots(&profile.paths().snapshots)
    }
}

fn list_snapshots(generations: &Path) -> Result<Vec<SnapshotRef>, ProfileError> {
    if !generations.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<PathBuf> = fs::read_dir(generations)
        .map_err(|source| ProfileError::Io { action: "read snapshot generations", source })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('g'))
        })
        .collect();
    names.sort();
    names.iter().map(|path| SnapshotRef::open(path)).collect()
}

fn write_snapshot(
    paths: &ProfilePaths,
    profile: &ProfileManifest,
    databases: DatabaseSource<'_>,
) -> Result<SnapshotRef, ProfileError> {
    ensure_supported_platform()?;
    validate_profile_paths(paths)?;
    let identity_hash = profile_identity_hash(&paths.identity)?;
    fs::create_dir_all(&paths.snapshots)
        .map_err(|source| ProfileError::Io { action: "create snapshot generations", source })?;
    set_private_directory(&paths.snapshots)
        .map_err(|source| ProfileError::Io { action: "secure snapshot generations", source })?;
    let snapshot_id = random_id();
    let stage = paths.snapshots.join(format!(".stage-{snapshot_id}"));
    let result = (|| {
        fs::create_dir(&stage)
            .map_err(|source| ProfileError::Io { action: "create snapshot stage", source })?;
        set_private_directory(&stage)
            .map_err(|source| ProfileError::Io { action: "secure snapshot stage", source })?;
        let mut budget = CopyBudget::default();
        for component in ["config", "identity"] {
            let source = paths.root.join(component);
            if source.exists() {
                copy_tree(&source, &stage.join(component), &mut budget)?;
            }
        }
        let data = stage.join("data");
        fs::create_dir(&data)
            .map_err(|source| ProfileError::Io { action: "create snapshot data", source })?;
        set_private_directory(&data)
            .map_err(|source| ProfileError::Io { action: "secure snapshot data", source })?;
        if paths.files.exists() {
            copy_tree(&paths.files, &data.join("files"), &mut budget)?;
        }
        let messages = data.join("messages.db");
        let nodes = data.join("nodes.db");
        match databases {
            DatabaseSource::Stopped => {
                if paths.messages.exists() {
                    backup_stopped_database(&paths.messages, &messages)?;
                }
                if paths.nodes.exists() {
                    backup_stopped_database(&paths.nodes, &nodes)?;
                }
            }
            DatabaseSource::Running(app_context) => {
                app_context
                    .store()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .backup_to(&messages)
                    .map_err(|error| ProfileError::Database(error.to_string()))?;
                app_context
                    .node_store()
                    .backup_to(&nodes)
                    .map_err(|error| ProfileError::Database(error.to_string()))?;
            }
        }
        for database in [&messages, &nodes] {
            if database.exists() {
                set_private_file(database).map_err(|source| ProfileError::Io {
                    action: "secure snapshot database",
                    source,
                })?;
            }
        }
        let mut components = std::collections::BTreeMap::new();
        hash_tree(&stage, &stage, &mut components)?;
        let manifest = SnapshotManifest {
            format_version: SNAPSHOT_FORMAT_VERSION,
            snapshot_id: snapshot_id.clone(),
            profile_id: profile.id.clone(),
            profile_generation: profile.generation,
            display_name: profile.display_name.clone(),
            identity_hash,
            created_at_unix: now_unix(),
            components,
        };
        let bytes = toml::to_string_pretty(&manifest).map_err(ProfileError::SerializeManifest)?;
        atomic_write_private(&stage.join("manifest.toml"), bytes.as_bytes())
            .map_err(|source| ProfileError::Io { action: "write snapshot manifest", source })?;
        make_files_read_only(&stage)?;
        sync_tree(&stage)?;
        let final_root = paths.snapshots.join(format!("g{}-{snapshot_id}", profile.generation));
        rename_no_replace(&stage, &final_root)?;
        sync_directory(&paths.snapshots)
            .map_err(|source| ProfileError::Io { action: "sync snapshot generations", source })?;
        Ok(SnapshotRef { root: final_root, manifest })
    })();
    if result.is_err() {
        let _ = make_files_writable(&stage);
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

fn restore_into_stage(
    snapshot: &SnapshotRef,
    stage: &Path,
    destination: &Path,
    runtime_parent: &Path,
) -> Result<StoppedManagedProfile, ProfileError> {
    fs::create_dir(stage)
        .map_err(|source| ProfileError::Io { action: "create restore stage", source })?;
    set_private_directory(stage)
        .map_err(|source| ProfileError::Io { action: "secure restore stage", source })?;
    let stage = stage
        .canonicalize()
        .map_err(|source| ProfileError::Io { action: "resolve restore stage", source })?;
    let lease = acquire_profile_lease(&stage)?;
    let mut budget = CopyBudget::default();
    for component in ["config", "identity", "data"] {
        let source = snapshot.root.join(component);
        if source.exists() {
            copy_tree(&source, &stage.join(component), &mut budget)?;
        }
    }
    let stage_paths = ProfilePaths::for_roots(stage.clone(), PathBuf::new());
    create_profile_directories(&stage_paths)?;
    if profile_identity_hash(&stage_paths.identity)? != snapshot.manifest.identity_hash {
        return Err(ProfileError::InvalidIdentity);
    }
    let manifest = ProfileManifest {
        format_version: PROFILE_FORMAT_VERSION,
        id: snapshot.manifest.profile_id.clone(),
        display_name: snapshot.manifest.display_name.clone(),
        storage: ProfileStorage::Local,
        generation: snapshot.manifest.profile_generation.saturating_add(1),
        created_at_unix: now_unix(),
        portable: None,
    };
    write_manifest(&stage_paths.manifest, &manifest)?;
    sync_tree(&stage)?;
    sync_directory(stage.parent().expect("stage has parent"))
        .map_err(|source| ProfileError::Io { action: "sync restore parent", source })?;
    let runtime_root = create_runtime_root(runtime_parent, &manifest.id)?;
    if let Err(error) = rename_no_replace(&stage, destination) {
        let _ = fs::remove_dir_all(&runtime_root);
        return Err(error);
    }
    if let Err(source) = sync_directory(destination.parent().expect("destination has parent")) {
        let _ = fs::remove_dir_all(destination);
        let _ = fs::remove_dir_all(&runtime_root);
        return Err(ProfileError::Io { action: "sync restored profile parent", source });
    }
    let paths = ProfilePaths::for_roots(destination.to_path_buf(), runtime_root);
    Ok(StoppedManagedProfile { manifest, paths, cleanup_durable_on_drop: false, _lease: lease })
}

fn backup_stopped_database(source: &Path, destination: &Path) -> Result<(), ProfileError> {
    use rusqlite::{Connection, OpenFlags};
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(source, flags)
        .map_err(|error| ProfileError::Database(error.to_string()))?;
    let mut target =
        Connection::open(destination).map_err(|error| ProfileError::Database(error.to_string()))?;
    let backup = rusqlite::backup::Backup::new(&conn, &mut target)
        .map_err(|error| ProfileError::Database(error.to_string()))?;
    backup
        .run_to_completion(64, std::time::Duration::from_millis(5), None)
        .map_err(|error| ProfileError::Database(error.to_string()))?;
    drop(backup);
    target
        .pragma_update(None, "journal_mode", "delete")
        .map_err(|error| ProfileError::Database(error.to_string()))?;
    Ok(())
}

fn hash_tree(
    root: &Path,
    path: &Path,
    out: &mut std::collections::BTreeMap<String, ComponentRecord>,
) -> Result<(), ProfileError> {
    reject_symlink(path)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| ProfileError::Io { action: "inspect snapshot entry", source })?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)
            .map_err(|source| ProfileError::Io { action: "read snapshot directory", source })?
        {
            let entry = entry
                .map_err(|source| ProfileError::Io { action: "read snapshot entry", source })?;
            hash_tree(root, &entry.path(), out)?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(ProfileError::InvalidPath(format!(
            "unsupported snapshot entry {}",
            path.display()
        )));
    }
    let relative = path.strip_prefix(root).map_err(|_| {
        ProfileError::InvalidPath(format!("{} escapes the snapshot", path.display()))
    })?;
    let key = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    if key == "manifest.toml" {
        return Ok(());
    }
    if key.ends_with("-wal") || key.ends_with("-shm") || key.ends_with("-journal") {
        return Err(ProfileError::InvalidPath(format!(
            "snapshot contains live database sidecar {key}"
        )));
    }
    let (sha256, bytes) = sha256_file(path)?;
    out.insert(key, ComponentRecord { sha256, bytes });
    Ok(())
}

fn sha256_file(path: &Path) -> Result<(String, u64), ProfileError> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path)
        .map_err(|source| ProfileError::Io { action: "open snapshot file", source })?;
    let mut hasher = Sha256::new();
    let bytes = io::copy(&mut file, &mut hasher)
        .map_err(|source| ProfileError::Io { action: "hash snapshot file", source })?;
    Ok((hex::encode(hasher.finalize()), bytes))
}

#[cfg(unix)]
fn make_files_read_only(root: &Path) -> Result<(), ProfileError> {
    use std::os::unix::fs::PermissionsExt;
    set_tree_file_mode(root, fs::Permissions::from_mode(0o400))
}

#[cfg(unix)]
fn make_files_writable(root: &Path) -> Result<(), ProfileError> {
    use std::os::unix::fs::PermissionsExt;
    set_tree_file_mode(root, fs::Permissions::from_mode(0o600))
}

#[cfg(unix)]
fn set_tree_file_mode(path: &Path, permissions: fs::Permissions) -> Result<(), ProfileError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| ProfileError::Io { action: "inspect snapshot entry", source })?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)
            .map_err(|source| ProfileError::Io { action: "read snapshot directory", source })?
        {
            let entry = entry
                .map_err(|source| ProfileError::Io { action: "read snapshot entry", source })?;
            set_tree_file_mode(&entry.path(), permissions.clone())?;
        }
    } else if metadata.is_file() {
        fs::set_permissions(path, permissions)
            .map_err(|source| ProfileError::Io { action: "set snapshot file mode", source })?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_files_read_only(_root: &Path) -> Result<(), ProfileError> {
    Ok(())
}

#[cfg(not(unix))]
fn make_files_writable(_root: &Path) -> Result<(), ProfileError> {
    Ok(())
}

// ── Identity custody ─────────────────────────────────────────────────────────

const CUSTODY_FORMAT_VERSION: u32 = 1;
const MAX_CUSTODY_BYTES: u64 = 256 * 1024;
const RECOVERY_KDF: &str = "argon2id";
const RECOVERY_KDF_MEMORY_KIB: u32 = 64 * 1024;
const RECOVERY_KDF_ITERATIONS: u32 = 3;
const RECOVERY_KDF_PARALLELISM: u32 = 1;
const RECOVERY_SALT_BYTES: usize = 16;
const RECOVERY_NONCE_BYTES: usize = 24;

/// Who holds the daemon's private Reticulum identity key.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CustodyBackend {
    /// The private key file inside the profile's identity directory.
    File,
    /// Key material held by hardware outside the profile. The profile keeps
    /// only the public fingerprint and any enrolled recovery slots.
    Hardware,
}

/// One passphrase-encrypted copy of the private identity key, bound to the
/// public fingerprint it must reproduce.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverySlot {
    pub id: String,
    pub created_at_unix: u64,
    pub fingerprint: String,
    pub kdf: String,
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
    pub salt: String,
    pub nonce: String,
    pub ciphertext: String,
}

/// The custody record of the daemon RNS identity. It names one public
/// fingerprint, the backend holding the private key, and enrolled recovery
/// slots. It carries no SDK identity metadata and no unrelated signer root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CustodyRecord {
    pub format_version: u32,
    pub backend: CustodyBackend,
    /// Hex address hash of the daemon RNS identity.
    pub fingerprint: String,
    pub hardware_locator: Option<String>,
    pub recovery_slots: Vec<RecoverySlot>,
}

impl CustodyRecord {
    fn file(fingerprint: String) -> Self {
        Self {
            format_version: CUSTODY_FORMAT_VERSION,
            backend: CustodyBackend::File,
            fingerprint,
            hardware_locator: None,
            recovery_slots: Vec::new(),
        }
    }
}

/// Why a recovery slot could not prove continuity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuityFailure {
    /// No recovery slot is enrolled, or none was offered.
    NoRecoverySlot,
    /// The slot did not decrypt with the offered passphrase.
    WrongPassphrase,
    /// The slot decrypted to an identity with a different fingerprint.
    FingerprintMismatch,
    /// The slot's stored material is malformed.
    SlotCorrupt,
}

/// The outcome of a recovery or abandonment attempt. Continuity is claimed
/// only after a recovered identity reproduces the recorded fingerprint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryOutcome {
    Continuity { fingerprint: String },
    ContinuityUnavailable { fingerprint: String, reason: ContinuityFailure },
}

impl RecoveryOutcome {
    pub fn is_continuity(&self) -> bool {
        matches!(self, Self::Continuity { .. })
    }
}

impl StoppedManagedProfile {
    /// The custody record. A profile created before custody records existed
    /// receives a file-backed record derived from its identity.
    pub fn custody(&self) -> Result<CustodyRecord, ProfileError> {
        if !self.paths.custody.exists() {
            let record = CustodyRecord::file(profile_identity_hash(&self.paths.identity)?);
            write_custody(&self.paths.custody, &record)?;
            return Ok(record);
        }
        read_custody(&self.paths.custody)
    }

    /// Enroll a passphrase-protected recovery slot holding the private
    /// identity key. Requires the key to be readable now.
    pub fn enroll_recovery_slot(&self, passphrase: &[u8]) -> Result<String, ProfileError> {
        let mut record = self.custody()?;
        let key_bytes = read_bounded_file(&self.paths.identity, PRIVATE_IDENTITY_BYTES)?;
        let fingerprint = profile_identity_hash(&self.paths.identity)?;
        if fingerprint != record.fingerprint {
            return Err(ProfileError::InvalidCustody);
        }
        let slot = seal_recovery_slot(&key_bytes, passphrase, &fingerprint)?;
        let id = slot.id.clone();
        record.recovery_slots.push(slot);
        write_custody(&self.paths.custody, &record)?;
        Ok(id)
    }

    /// Check whether a slot reproduces the recorded fingerprint. Writes nothing.
    pub fn verify_recovery_slot(
        &self,
        slot_id: &str,
        passphrase: &[u8],
    ) -> Result<RecoveryOutcome, ProfileError> {
        let record = self.custody()?;
        let slot = record
            .recovery_slots
            .iter()
            .find(|slot| slot.id == slot_id)
            .ok_or_else(|| ProfileError::UnknownRecoverySlot(slot_id.to_string()))?;
        Ok(match open_recovery_slot(slot, passphrase, &record.fingerprint) {
            Ok(_) => RecoveryOutcome::Continuity { fingerprint: record.fingerprint.clone() },
            Err(reason) => RecoveryOutcome::ContinuityUnavailable {
                fingerprint: record.fingerprint.clone(),
                reason,
            },
        })
    }

    /// Hand the private key to hardware custody: the profile keeps the
    /// fingerprint and recovery slots and drops its plaintext key file.
    pub fn move_to_hardware_custody(&self, locator: &str) -> Result<(), ProfileError> {
        let mut record = self.custody()?;
        if profile_identity_hash(&self.paths.identity)? != record.fingerprint {
            return Err(ProfileError::InvalidCustody);
        }
        record.backend = CustodyBackend::Hardware;
        record.hardware_locator = Some(locator.to_string());
        write_custody(&self.paths.custody, &record)?;
        fs::remove_file(&self.paths.identity)
            .map_err(|source| ProfileError::Io { action: "release plaintext identity", source })?;
        sync_directory(self.paths.identity.parent().expect("identity has parent"))
            .map_err(|source| ProfileError::Io { action: "sync identity directory", source })?;
        Ok(())
    }

    /// Whether the private key is available to start the daemon right now.
    pub fn identity_available(&self) -> bool {
        validate_identity(&self.paths.identity).is_ok()
    }

    /// Abandon hardware custody. Continuity is claimed only when an enrolled
    /// slot decrypts with `passphrase` to the recorded fingerprint, in which
    /// case the key returns to file custody. Otherwise nothing is written and
    /// no replacement identity is created under this profile.
    pub fn abandon_hardware_custody(
        &self,
        passphrase: Option<&[u8]>,
    ) -> Result<RecoveryOutcome, ProfileError> {
        let mut record = self.custody()?;
        if record.backend != CustodyBackend::Hardware {
            return Err(ProfileError::NotHardwareCustody);
        }
        let fingerprint = record.fingerprint.clone();
        let Some(passphrase) = passphrase else {
            return Ok(RecoveryOutcome::ContinuityUnavailable {
                fingerprint,
                reason: ContinuityFailure::NoRecoverySlot,
            });
        };
        if record.recovery_slots.is_empty() {
            return Ok(RecoveryOutcome::ContinuityUnavailable {
                fingerprint,
                reason: ContinuityFailure::NoRecoverySlot,
            });
        }
        let mut last_failure = ContinuityFailure::NoRecoverySlot;
        for slot in &record.recovery_slots {
            match open_recovery_slot(slot, passphrase, &fingerprint) {
                Ok(key_bytes) => {
                    crate::identity_store::write_identity_file(&self.paths.identity, &key_bytes)
                        .map_err(|source| ProfileError::Io {
                            action: "restore identity file",
                            source,
                        })?;
                    validate_identity(&self.paths.identity)?;
                    record.backend = CustodyBackend::File;
                    record.hardware_locator = None;
                    write_custody(&self.paths.custody, &record)?;
                    return Ok(RecoveryOutcome::Continuity { fingerprint });
                }
                Err(reason) => last_failure = reason,
            }
        }
        Ok(RecoveryOutcome::ContinuityUnavailable { fingerprint, reason: last_failure })
    }
}

fn read_custody(path: &Path) -> Result<CustodyRecord, ProfileError> {
    let bytes = read_bounded_file(path, MAX_CUSTODY_BYTES)?;
    let record: CustodyRecord =
        toml::from_str(std::str::from_utf8(&bytes).map_err(|_| ProfileError::InvalidCustody)?)
            .map_err(|_| ProfileError::InvalidCustody)?;
    if record.format_version != CUSTODY_FORMAT_VERSION {
        return Err(ProfileError::UnsupportedFormat(record.format_version));
    }
    if record.fingerprint.len() != 32 || !record.fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(ProfileError::InvalidCustody);
    }
    Ok(record)
}

fn write_custody(path: &Path, record: &CustodyRecord) -> Result<(), ProfileError> {
    let bytes = toml::to_string_pretty(record).map_err(ProfileError::SerializeManifest)?;
    atomic_write_private(path, bytes.as_bytes())
        .map_err(|source| ProfileError::Io { action: "write identity custody", source })
}

fn derive_recovery_key(
    passphrase: &[u8],
    salt: &[u8],
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
) -> Result<zeroize::Zeroizing<[u8; 32]>, ProfileError> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(memory_kib, iterations, parallelism, Some(32))
        .map_err(|error| ProfileError::Custody(error.to_string()))?;
    let mut key = zeroize::Zeroizing::new([0_u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase, salt, key.as_mut())
        .map_err(|error| ProfileError::Custody(error.to_string()))?;
    Ok(key)
}

fn seal_recovery_slot(
    key_bytes: &[u8],
    passphrase: &[u8],
    fingerprint: &str,
) -> Result<RecoverySlot, ProfileError> {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{XChaCha20Poly1305, XNonce};
    let mut salt = [0_u8; RECOVERY_SALT_BYTES];
    OsRng.fill_bytes(&mut salt);
    let mut nonce = [0_u8; RECOVERY_NONCE_BYTES];
    OsRng.fill_bytes(&mut nonce);
    let key = derive_recovery_key(
        passphrase,
        &salt,
        RECOVERY_KDF_MEMORY_KIB,
        RECOVERY_KDF_ITERATIONS,
        RECOVERY_KDF_PARALLELISM,
    )?;
    let cipher = XChaCha20Poly1305::new(key.as_ref().into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload { msg: key_bytes, aad: fingerprint.as_bytes() },
        )
        .map_err(|_| ProfileError::Custody("seal recovery slot".into()))?;
    Ok(RecoverySlot {
        id: random_id(),
        created_at_unix: now_unix(),
        fingerprint: fingerprint.to_string(),
        kdf: RECOVERY_KDF.into(),
        memory_kib: RECOVERY_KDF_MEMORY_KIB,
        iterations: RECOVERY_KDF_ITERATIONS,
        parallelism: RECOVERY_KDF_PARALLELISM,
        salt: hex::encode(salt),
        nonce: hex::encode(nonce),
        ciphertext: hex::encode(ciphertext),
    })
}

/// Decrypt a slot and prove it reproduces `expected_fingerprint`.
fn open_recovery_slot(
    slot: &RecoverySlot,
    passphrase: &[u8],
    expected_fingerprint: &str,
) -> Result<zeroize::Zeroizing<Vec<u8>>, ContinuityFailure> {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{XChaCha20Poly1305, XNonce};
    if slot.kdf != RECOVERY_KDF || slot.fingerprint != expected_fingerprint {
        return Err(ContinuityFailure::FingerprintMismatch);
    }
    let salt = hex::decode(&slot.salt).map_err(|_| ContinuityFailure::SlotCorrupt)?;
    let nonce = hex::decode(&slot.nonce).map_err(|_| ContinuityFailure::SlotCorrupt)?;
    let ciphertext = hex::decode(&slot.ciphertext).map_err(|_| ContinuityFailure::SlotCorrupt)?;
    if nonce.len() != RECOVERY_NONCE_BYTES || salt.is_empty() {
        return Err(ContinuityFailure::SlotCorrupt);
    }
    let key =
        derive_recovery_key(passphrase, &salt, slot.memory_kib, slot.iterations, slot.parallelism)
            .map_err(|_| ContinuityFailure::SlotCorrupt)?;
    let cipher = XChaCha20Poly1305::new(key.as_ref().into());
    let key_bytes = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload { msg: &ciphertext, aad: expected_fingerprint.as_bytes() },
        )
        .map_err(|_| ContinuityFailure::WrongPassphrase)?;
    let key_bytes = zeroize::Zeroizing::new(key_bytes);
    let identity = PrivateIdentity::from_private_key_bytes(&key_bytes)
        .map_err(|_| ContinuityFailure::SlotCorrupt)?;
    if hex::encode(identity.address_hash().as_slice()) != expected_fingerprint {
        return Err(ContinuityFailure::FingerprintMismatch);
    }
    Ok(key_bytes)
}

// ── Portable operation ───────────────────────────────────────────────────────

const PORTABLE_MARKER_FILE: &str = ".styrene-portable";

/// What the backend established about the media under a portable root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaCapability {
    /// Whether the volume is encrypted at rest.
    pub encrypted: bool,
    /// Filesystem name for disclosures, such as `apfs` or `ext4`.
    pub filesystem: String,
    /// Stable volume selector such as a volume UUID; never a mount path.
    pub volume_selector: String,
    pub posix_permissions: bool,
    pub atomic_rename: bool,
    pub durable_sync: bool,
}

impl MediaCapability {
    /// Why this media cannot host a Portable profile, if anything.
    pub fn unsupported_reason(&self) -> Option<String> {
        let mut missing = Vec::new();
        if !self.encrypted {
            missing.push("volume is not encrypted");
        }
        if !self.posix_permissions {
            missing.push("filesystem lacks private permissions");
        }
        if !self.atomic_rename {
            missing.push("filesystem lacks atomic no-replace rename");
        }
        if !self.durable_sync {
            missing.push("filesystem lacks durable sync");
        }
        if self.volume_selector.trim().is_empty() {
            missing.push("volume has no stable selector");
        }
        (!missing.is_empty()).then(|| format!("{} ({})", missing.join(", "), self.filesystem))
    }
}

/// Inspects the media beneath a path. Production adapters query the host;
/// tests supply a static description.
pub trait MediaInspector {
    fn inspect(&self, path: &Path) -> Result<MediaCapability, ProfileError>;
}

/// A fixed media description for tests and fixtures.
#[derive(Clone, Debug)]
pub struct StaticMediaInspector {
    pub capability: MediaCapability,
}

impl MediaInspector for StaticMediaInspector {
    fn inspect(&self, _path: &Path) -> Result<MediaCapability, ProfileError> {
        Ok(self.capability.clone())
    }
}

/// How a Portable profile is recognised on media, independent of mount path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortableBinding {
    pub volume_selector: String,
    pub marker: String,
    pub filesystem: String,
}

impl PortableBinding {
    fn is_valid(&self) -> bool {
        !self.volume_selector.trim().is_empty()
            && self.marker.len() == 32
            && self.marker.bytes().all(|byte| byte.is_ascii_hexdigit())
    }
}

/// The selector an operator keeps to find a Portable profile again.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableSelector {
    pub volume_selector: String,
    pub marker: String,
}

/// Whether portable media is still reachable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaStatus {
    Present,
    Missing,
}

/// What safe removal did before reporting the media removable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovalReport {
    pub root: PathBuf,
    pub quiesced: bool,
    pub checkpointed: bool,
    pub synchronized: bool,
    pub lease_released: bool,
    pub keys_cleared: bool,
    pub media_removable: bool,
}

/// A profile whose media disappeared while its daemon ran. Durable writes
/// stopped; nothing was redirected to host defaults.
#[derive(Debug)]
pub struct InterruptedProfile {
    pub root: PathBuf,
    pub runtime_root: PathBuf,
    pub status: MediaStatus,
}

fn ensure_media_supported(
    inspector: &dyn MediaInspector,
    path: &Path,
) -> Result<MediaCapability, ProfileError> {
    let capability = inspector.inspect(path)?;
    match capability.unsupported_reason() {
        Some(reason) => Err(ProfileError::UnsupportedPortableMedia(reason)),
        None => Ok(capability),
    }
}

fn ensure_runtime_off_media(runtime_parent: &Path, media_root: &Path) -> Result<(), ProfileError> {
    let runtime = validate_existing_directory(runtime_parent)?;
    let media = validate_existing_directory(media_root)?;
    if runtime.starts_with(&media) || media.starts_with(&runtime) {
        return Err(ProfileError::InvalidPath(
            "runtime root must live on host-private storage, not on the portable media".into(),
        ));
    }
    Ok(())
}

impl StoppedManagedProfile {
    /// Create a Portable profile at `root` on media beneath `media_root`.
    /// The media must be encrypted and capable; the runtime parent must be
    /// host-private. Returns the selector that finds the profile again.
    pub fn create_portable(
        media_root: &Path,
        root: &Path,
        runtime_parent: &Path,
        display_name: &str,
        inspector: &dyn MediaInspector,
    ) -> Result<(Self, PortableSelector), ProfileError> {
        ensure_supported_platform()?;
        ensure_runtime_off_media(runtime_parent, media_root)?;
        let media = validate_existing_directory(media_root)?;
        let capability = ensure_media_supported(inspector, &media)?;
        let root_parent = root
            .parent()
            .ok_or_else(|| ProfileError::InvalidPath("portable root has no parent".into()))?;
        if !validate_existing_directory(root_parent)?.starts_with(&media) {
            return Err(ProfileError::InvalidPath("portable root must sit on the media".into()));
        }
        let id = random_id();
        let marker = random_id();
        let binding = PortableBinding {
            volume_selector: capability.volume_selector.clone(),
            marker: marker.clone(),
            filesystem: capability.filesystem.clone(),
        };
        let mut profile = Self::create(
            root.to_path_buf(),
            runtime_parent,
            display_name,
            ProfileStorage::Local,
            id,
        )?;
        // Rewrite the manifest as Portable with its binding, then place the marker.
        let result = (|| {
            profile.manifest.storage = ProfileStorage::Portable;
            profile.manifest.portable = Some(binding);
            write_manifest(&profile.paths.manifest, &profile.manifest)?;
            atomic_write_private(&profile.paths.root.join(PORTABLE_MARKER_FILE), marker.as_bytes())
                .map_err(|source| ProfileError::Io { action: "write portable marker", source })?;
            sync_directory(&profile.paths.root)
                .map_err(|source| ProfileError::Io { action: "sync portable root", source })
        })();
        if let Err(error) = result {
            profile.cleanup_durable_on_drop = true;
            return Err(error);
        }
        let selector = PortableSelector { volume_selector: capability.volume_selector, marker };
        Ok((profile, selector))
    }

    /// Find and open a Portable profile by its selector on any of the offered
    /// mount points. The persisted binding, not a remembered path, decides.
    pub fn open_portable(
        selector: &PortableSelector,
        mounts: &[PathBuf],
        runtime_parent: &Path,
        inspector: &dyn MediaInspector,
    ) -> Result<Self, ProfileError> {
        ensure_supported_platform()?;
        for mount in mounts {
            let Ok(mount) = validate_existing_directory(mount) else { continue };
            let Ok(capability) = inspector.inspect(&mount) else { continue };
            if capability.volume_selector != selector.volume_selector {
                continue;
            }
            ensure_media_supported(inspector, &mount)?;
            ensure_runtime_off_media(runtime_parent, &mount)?;
            for entry in fs::read_dir(&mount)
                .map_err(|source| ProfileError::Io { action: "read portable media", source })?
                .flatten()
            {
                let candidate = entry.path();
                let marker = candidate.join(PORTABLE_MARKER_FILE);
                if !marker.is_file() {
                    continue;
                }
                let Ok(found) = read_bounded_file(&marker, 64) else { continue };
                if found != selector.marker.as_bytes() {
                    continue;
                }
                let profile = Self::open(&candidate, runtime_parent)?;
                match profile.manifest.portable.as_ref() {
                    Some(binding)
                        if binding.marker == selector.marker
                            && binding.volume_selector == selector.volume_selector => {}
                    _ => return Err(ProfileError::InvalidManifest),
                }
                return Ok(profile);
            }
        }
        Err(ProfileError::PortableNotFound)
    }

    /// The selector of a Portable profile.
    pub fn portable_selector(&self) -> Option<PortableSelector> {
        self.manifest.portable.as_ref().map(|binding| PortableSelector {
            volume_selector: binding.volume_selector.clone(),
            marker: binding.marker.clone(),
        })
    }

    /// Whether the profile's media is reachable right now.
    pub fn media_status(&self) -> MediaStatus {
        match fs::symlink_metadata(&self.paths.manifest) {
            Ok(_) => MediaStatus::Present,
            Err(_) => MediaStatus::Missing,
        }
    }

    /// Checkpoint and synchronize a stopped profile, then release its lease
    /// and report the media removable. Nothing is left open on the media.
    pub fn prepare_safe_removal(self) -> Result<RemovalReport, ProfileError> {
        if self.media_status() == MediaStatus::Missing {
            return Err(ProfileError::MediaMissing);
        }
        let root = self.paths.root.clone();
        for database in [&self.paths.messages, &self.paths.nodes] {
            if database.exists() {
                checkpoint_database(database)?;
            }
        }
        sync_tree(&root)?;
        let runtime_root = self.paths.runtime_root.clone();
        drop(self);
        let _ = fs::remove_dir_all(&runtime_root);
        Ok(RemovalReport {
            root,
            quiesced: true,
            checkpointed: true,
            synchronized: true,
            lease_released: true,
            keys_cleared: true,
            media_removable: true,
        })
    }
}

impl RunningManagedProfile {
    /// Whether the media beneath the running profile is still reachable.
    pub fn media_status(&self) -> MediaStatus {
        self.profile.as_ref().expect("running profile has lease").media_status()
    }

    /// Quiesce the daemon, checkpoint and synchronize durable state, release
    /// ownership, and clear in-memory key material before the media is removed.
    pub async fn prepare_safe_removal(mut self) -> Result<RemovalReport, ProfileError> {
        let daemon = self.daemon.take().expect("running profile has daemon");
        let profile = self.profile.take().expect("running profile has lease");
        if profile.media_status() == MediaStatus::Missing {
            drop(daemon);
            return Err(ProfileError::MediaMissing);
        }
        daemon.shutdown().await;
        profile.prepare_safe_removal()
    }

    /// Stop after the media disappeared: the daemon is dropped so durable
    /// writes end, the host-private runtime root is removed, and no path is
    /// redirected to host defaults. The lease file on the vanished media is
    /// simply gone.
    pub async fn interrupt(mut self) -> InterruptedProfile {
        let daemon = self.daemon.take().expect("running profile has daemon");
        let mut profile = self.profile.take().expect("running profile has lease");
        let status = profile.media_status();
        daemon.shutdown().await;
        profile.cleanup_durable_on_drop = false;
        let root = profile.paths.root.clone();
        let runtime_root = profile.paths.runtime_root.clone();
        drop(profile);
        InterruptedProfile { root, runtime_root, status }
    }
}

fn checkpoint_database(path: &Path) -> Result<(), ProfileError> {
    let conn = rusqlite::Connection::open(path)
        .map_err(|error| ProfileError::Database(error.to_string()))?;
    conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")
        .map_err(|error| ProfileError::Database(error.to_string()))?;
    conn.pragma_update(None, "journal_mode", "delete")
        .map_err(|error| ProfileError::Database(error.to_string()))?;
    Ok(())
}

// ── Inventory probes ─────────────────────────────────────────────────────────

/// What can be learned about a profile root without taking its lease.
#[derive(Clone, Debug)]
pub struct ProfileProbe {
    pub root: PathBuf,
    pub manifest: ProfileManifest,
    pub custody: Option<CustodyRecord>,
    /// Another owner currently holds the exclusive writer lease.
    pub leased: bool,
    pub identity_available: bool,
    pub snapshot_count: u32,
}

/// Read a profile root's manifest, custody, lease state, and snapshot count
/// without acquiring its lease or mutating it.
pub fn probe_root(root: &Path) -> Result<ProfileProbe, ProfileError> {
    reject_symlink(root)?;
    let root = root
        .canonicalize()
        .map_err(|source| ProfileError::Io { action: "resolve profile root", source })?;
    let manifest_bytes = read_bounded_file(&root.join("manifest.toml"), MAX_MANIFEST_BYTES)?;
    let manifest: ProfileManifest = toml::from_str(
        std::str::from_utf8(&manifest_bytes).map_err(|_| ProfileError::InvalidManifest)?,
    )
    .map_err(|_| ProfileError::InvalidManifest)?;
    validate_manifest(&manifest)?;
    let paths = ProfilePaths::for_roots(root.clone(), PathBuf::new());
    let custody = paths.custody.exists().then(|| read_custody(&paths.custody)).transpose()?;
    let identity_available = validate_identity(&paths.identity).is_ok();
    let leased = lease_is_held(&root)?;
    let snapshot_count = list_snapshots(&paths.snapshots).map(|list| list.len()).unwrap_or(0);
    Ok(ProfileProbe {
        root,
        manifest,
        custody,
        leased,
        identity_available,
        snapshot_count: u32::try_from(snapshot_count).unwrap_or(u32::MAX),
    })
}

#[cfg(unix)]
fn lease_is_held(root: &Path) -> Result<bool, ProfileError> {
    use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};
    let path = root.join(".profile.lock");
    if !path.exists() {
        return Ok(false);
    }
    let fd = open(&path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW, Mode::empty())
        .map_err(|error| ProfileError::Io {
            action: "open profile lease",
            source: io::Error::from_raw_os_error(error.raw_os_error()),
        })?;
    let file = File::from(fd);
    match flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {
            let _ = flock(&file, FlockOperation::Unlock);
            Ok(false)
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(true),
        Err(error) => Err(ProfileError::Io {
            action: "probe profile lease",
            source: io::Error::from_raw_os_error(error.raw_os_error()),
        }),
    }
}

#[cfg(not(unix))]
fn lease_is_held(_root: &Path) -> Result<bool, ProfileError> {
    Err(ProfileError::UnsupportedPlatform)
}

/// Snapshot the profile this daemon runs from, through its live stores.
pub fn snapshot_running_root(
    root: &Path,
    app_context: &crate::app_context::AppContext,
) -> Result<SnapshotRef, ProfileError> {
    let probe = probe_root(root)?;
    let paths = ProfilePaths::for_roots(probe.root, PathBuf::new());
    write_snapshot(&paths, &probe.manifest, DatabaseSource::Running(app_context))
}

/// Copy a verified snapshot generation to an unused directory outside the
/// profile, then verify the copy.
pub fn copy_snapshot(
    snapshot: &SnapshotRef,
    destination: &Path,
) -> Result<SnapshotRef, ProfileError> {
    snapshot.verify()?;
    if destination.exists() {
        return Err(ProfileError::DestinationExists(destination.to_path_buf()));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| ProfileError::InvalidPath("destination has no parent".into()))?;
    validate_existing_directory(parent)?;
    let mut budget = CopyBudget::default();
    copy_tree(&snapshot.root, destination, &mut budget)?;
    make_files_read_only(destination)?;
    sync_tree(destination)?;
    SnapshotRef::open(destination)
}

// ── Legacy layout adoption ───────────────────────────────────────────────────

/// The pre-profile on-disk layout: separate configuration and data
/// directories with global defaults. Every path is optional; only the ones
/// that exist are adopted.
#[derive(Clone, Debug, Default)]
pub struct LegacyLayout {
    pub config: Option<PathBuf>,
    pub identity: Option<PathBuf>,
    pub messages_db: Option<PathBuf>,
    pub nodes_db: Option<PathBuf>,
    pub pages_dir: Option<PathBuf>,
    pub files_dir: Option<PathBuf>,
}

impl LegacyLayout {
    /// The layout a daemon used with these configuration and data
    /// directories.
    pub fn for_dirs(config_dir: &Path, data_dir: &Path) -> Self {
        Self {
            config: Some(config_dir.join("config.toml")),
            identity: Some(config_dir.join("identity")),
            messages_db: Some(data_dir.join("messages.db")),
            nodes_db: Some(data_dir.join("nodes.db")),
            pages_dir: Some(config_dir.join("pages")),
            files_dir: Some(data_dir.join("files")),
        }
    }

    /// Whether any adoptable state exists.
    pub fn has_state(&self) -> bool {
        [
            &self.config,
            &self.identity,
            &self.messages_db,
            &self.nodes_db,
            &self.pages_dir,
            &self.files_dir,
        ]
        .into_iter()
        .flatten()
        .any(|path| path.exists())
    }
}

impl StoppedManagedProfile {
    /// Create a Local profile at `root` and copy a legacy layout into it. The
    /// legacy files are read, never moved or changed, so the old layout stays
    /// usable. The legacy identity becomes the profile identity when present.
    pub fn adopt_legacy(
        root: &Path,
        runtime_parent: &Path,
        display_name: &str,
        legacy: &LegacyLayout,
    ) -> Result<Self, ProfileError> {
        let profile = Self::create_local(root, runtime_parent, display_name)?;
        let result = (|| {
            if let Some(identity) = legacy.identity.as_ref().filter(|path| path.is_file()) {
                let bytes = read_bounded_file(identity, PRIVATE_IDENTITY_BYTES)?;
                if bytes.len() as u64 != PRIVATE_IDENTITY_BYTES
                    || PrivateIdentity::from_private_key_bytes(&bytes).is_err()
                {
                    return Err(ProfileError::InvalidIdentity);
                }
                crate::identity_store::write_identity_file(&profile.paths.identity, &bytes)
                    .map_err(|source| ProfileError::Io {
                        action: "adopt legacy identity",
                        source,
                    })?;
                write_custody(
                    &profile.paths.custody,
                    &CustodyRecord::file(profile_identity_hash(&profile.paths.identity)?),
                )?;
            }
            if let Some(config) = legacy.config.as_ref().filter(|path| path.is_file()) {
                let bytes = fs::read(config)
                    .map_err(|source| ProfileError::Io { action: "read legacy config", source })?;
                atomic_write_private(&profile.paths.config, &bytes)
                    .map_err(|source| ProfileError::Io { action: "adopt legacy config", source })?;
            }
            for (source, destination) in [
                (legacy.messages_db.as_ref(), &profile.paths.messages),
                (legacy.nodes_db.as_ref(), &profile.paths.nodes),
            ] {
                if let Some(source) = source.filter(|path| path.is_file()) {
                    backup_stopped_database(source, destination)?;
                    set_private_file(destination).map_err(|source| ProfileError::Io {
                        action: "secure adopted database",
                        source,
                    })?;
                }
            }
            for (source, destination) in [
                (legacy.pages_dir.as_ref(), &profile.paths.pages),
                (legacy.files_dir.as_ref(), &profile.paths.files),
            ] {
                if let Some(source) = source.filter(|path| path.is_dir()) {
                    let mut budget = CopyBudget::default();
                    for entry in fs::read_dir(source).map_err(|source| ProfileError::Io {
                        action: "read legacy directory",
                        source,
                    })? {
                        let entry = entry.map_err(|source| ProfileError::Io {
                            action: "read legacy entry",
                            source,
                        })?;
                        copy_tree(
                            &entry.path(),
                            &destination.join(entry.file_name()),
                            &mut budget,
                        )?;
                    }
                }
            }
            sync_tree(&profile.paths.root)
        })();
        match result {
            Ok(()) => Ok(profile),
            Err(error) => {
                // Publish nothing: the half-adopted root goes away.
                let root = profile.paths.root.clone();
                drop(profile);
                let _ = fs::remove_dir_all(root);
                Err(error)
            }
        }
    }
}
