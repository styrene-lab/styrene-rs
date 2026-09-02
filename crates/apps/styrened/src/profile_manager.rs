//! Backend authority for the operator profile lifecycle exposed over IPC.
//!
//! The manager owns the profiles this daemon created or adopted, describes
//! every profile it can see, and performs stopped-profile transactions.
//! Frontends select and confirm; the manager decides.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use styrene_ipc::IpcError;
use styrene_ipc::types::{
    ProfileAdoptRequest, ProfileCreateRequest, ProfileCustodyInfo, ProfileExportRequest,
    ProfileInfo, ProfileInventory, ProfileNetworkPolicy, ProfileOperationOutcome,
    ProfileOperationProgress, ProfileOperationState, ProfileOwnership, ProfilePersistence,
    ProfilePromoteRequest, ProfileRestoreRequest, ProfileSnapshotInfo, ProfileSnapshotRequest,
    ProfileStorageKind,
};

use crate::app_context::AppContext;
use crate::operator_profile::{
    CustodyBackend, MediaInspector, ProfileError, ProfileProbe, ProfileStorage, SnapshotRef,
    StoppedManagedProfile, copy_snapshot, probe_root, snapshot_running_root,
};

/// Where managed profiles and their host-private runtime roots live.
#[derive(Clone, Debug)]
pub struct ProfileRoots {
    pub profiles_parent: PathBuf,
    pub runtime_parent: PathBuf,
}

/// The profile this daemon runs from.
#[derive(Clone, Debug)]
pub struct ActiveProfile {
    pub id: String,
    pub root: PathBuf,
}

/// Backend authority for the operator profile lifecycle.
pub struct ProfileManager {
    roots: ProfileRoots,
    active: Option<ActiveProfile>,
    media: Option<Arc<dyn MediaInspector + Send + Sync>>,
    held: Mutex<HashMap<String, StoppedManagedProfile>>,
    operations: Mutex<HashMap<String, ProfileOperationProgress>>,
}

impl std::fmt::Debug for ProfileManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileManager")
            .field("roots", &self.roots)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

fn map_error(error: ProfileError) -> IpcError {
    match error {
        ProfileError::DestinationExists(path) => IpcError::Conflict {
            message: format!("destination already exists: {}", path.display()),
        },
        ProfileError::ProfileInUse(path) => {
            IpcError::Conflict { message: format!("profile is already in use: {}", path.display()) }
        }
        ProfileError::UnsupportedPortableMedia(reason) => IpcError::Unavailable { reason },
        ProfileError::UnsupportedPlatform => IpcError::Unavailable {
            reason: "operator profiles are unsupported on this platform".into(),
        },
        ProfileError::PortableNotFound | ProfileError::MediaMissing => {
            IpcError::NotFound { resource: error.to_string() }
        }
        ProfileError::Io { .. } | ProfileError::Database(_) | ProfileError::Custody(_) => {
            IpcError::Internal { message: error.to_string() }
        }
        other => IpcError::InvalidRequest { message: other.to_string() },
    }
}

fn storage_kind(storage: ProfileStorage) -> ProfileStorageKind {
    match storage {
        ProfileStorage::Quick => ProfileStorageKind::Quick,
        ProfileStorage::Local => ProfileStorageKind::Local,
        ProfileStorage::Portable => ProfileStorageKind::Portable,
    }
}

fn random_operation_id() -> String {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0_u8; 8];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

impl ProfileManager {
    pub fn new(roots: ProfileRoots, active: Option<ActiveProfile>) -> Self {
        Self {
            roots,
            active,
            media: None,
            held: Mutex::new(HashMap::new()),
            operations: Mutex::new(HashMap::new()),
        }
    }

    /// Enable Portable profile creation through `inspector`.
    #[must_use]
    pub fn with_media_inspector(
        mut self,
        inspector: Arc<dyn MediaInspector + Send + Sync>,
    ) -> Self {
        self.media = Some(inspector);
        self
    }

    pub fn roots(&self) -> &ProfileRoots {
        &self.roots
    }

    pub fn active(&self) -> Option<&ActiveProfile> {
        self.active.as_ref()
    }

    fn describe(&self, probe: &ProfileProbe) -> ProfileInfo {
        let held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let held_by_daemon = held.contains_key(probe.root.to_string_lossy().as_ref());
        let active = self.active.as_ref().is_some_and(|active| active.id == probe.manifest.id);
        let mut info = ProfileInfo::default();
        info.id = probe.manifest.id.clone();
        info.display_name = probe.manifest.display_name.clone();
        info.storage = storage_kind(probe.manifest.storage);
        info.generation = probe.manifest.generation;
        info.root = probe.root.display().to_string();
        info.created_at_unix = probe.manifest.created_at_unix;
        let mut ownership = ProfileOwnership::default();
        ownership.leased_elsewhere = probe.leased && !held_by_daemon && !active;
        ownership.held_by_daemon = held_by_daemon;
        ownership.active = active;
        info.ownership = ownership;
        let mut persistence = ProfilePersistence::default();
        persistence.durable = probe.manifest.storage != ProfileStorage::Quick;
        persistence.removed_on_release = probe.manifest.storage == ProfileStorage::Quick;
        persistence.snapshot_count = probe.snapshot_count;
        info.persistence = persistence;
        let mut custody = ProfileCustodyInfo::default();
        custody.backend = match probe.custody.as_ref().map(|custody| custody.backend) {
            Some(CustodyBackend::Hardware) => "hardware".into(),
            _ => "file".into(),
        };
        custody.fingerprint =
            probe.custody.as_ref().map(|custody| custody.fingerprint.clone()).unwrap_or_default();
        custody.recovery_slots = probe
            .custody
            .as_ref()
            .map(|custody| u32::try_from(custody.recovery_slots.len()).unwrap_or(u32::MAX))
            .unwrap_or(0);
        custody.identity_available = probe.identity_available;
        info.custody = custody;
        let mut network_policy = ProfileNetworkPolicy::default();
        network_policy.conservative_defaults = true;
        info.network_policy = network_policy;
        info.volume_selector =
            probe.manifest.portable.as_ref().map(|binding| binding.volume_selector.clone());
        info
    }

    fn info_for_root(&self, root: &Path) -> Result<ProfileInfo, IpcError> {
        probe_root(root).map(|probe| self.describe(&probe)).map_err(map_error)
    }

    /// Every profile under the profiles root plus any held elsewhere.
    pub fn inventory(&self) -> Result<ProfileInventory, IpcError> {
        let mut roots: Vec<PathBuf> = Vec::new();
        let push = |roots: &mut Vec<PathBuf>, path: PathBuf| {
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            if !roots.iter().any(|known| known == &path) {
                roots.push(path);
            }
        };
        if let Ok(entries) = std::fs::read_dir(&self.roots.profiles_parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.join("manifest.toml").is_file() {
                    push(&mut roots, path);
                }
            }
        }
        {
            let held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
            for profile in held.values() {
                push(&mut roots, profile.paths().root.clone());
            }
        }
        if let Some(active) = &self.active {
            push(&mut roots, active.root.clone());
        }
        roots.sort();
        let mut profiles = Vec::new();
        for root in roots {
            if let Ok(probe) = probe_root(&root) {
                profiles.push(self.describe(&probe));
            }
        }
        let mut inventory = ProfileInventory::default();
        inventory.profiles = profiles;
        inventory.active_profile_id = self.active.as_ref().map(|active| active.id.clone());
        inventory.profiles_root = self.roots.profiles_parent.display().to_string();
        Ok(inventory)
    }

    fn record(
        &self,
        kind: &str,
        profile_id: Option<String>,
        result: Result<(), String>,
    ) -> ProfileOperationProgress {
        let mut progress = ProfileOperationProgress::default();
        progress.operation_id = random_operation_id();
        progress.kind = kind.into();
        progress.state = if result.is_ok() {
            ProfileOperationState::Completed
        } else {
            ProfileOperationState::Failed
        };
        progress.detail = result.err();
        progress.profile_id = profile_id;
        self.operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(progress.operation_id.clone(), progress.clone());
        progress
    }

    fn outcome(
        &self,
        kind: &str,
        result: Result<(Option<ProfileInfo>, Option<ProfileSnapshotInfo>, bool), IpcError>,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        match result {
            Ok((profile, snapshot, restart_required)) => {
                let progress = self.record(kind, profile.as_ref().map(|p| p.id.clone()), Ok(()));
                let mut outcome = ProfileOperationOutcome::default();
                outcome.progress = progress;
                outcome.profile = profile;
                outcome.snapshot = snapshot;
                outcome.restart_required = restart_required;
                Ok(outcome)
            }
            Err(error) => {
                self.record(kind, None, Err(error.to_string()));
                Err(error)
            }
        }
    }

    pub fn operation(&self, operation_id: &str) -> Result<ProfileOperationProgress, IpcError> {
        self.operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(operation_id)
            .cloned()
            .ok_or_else(|| IpcError::not_found("profile operation", operation_id))
    }

    fn hold(&self, profile: StoppedManagedProfile) -> ProfileInfo {
        let root = profile.paths().root.clone();
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(root.to_string_lossy().into_owned(), profile);
        self.info_for_root(&root).unwrap_or_default()
    }

    /// Take a held Quick profile by id.
    fn take_held_quick(&self, profile_id: &str) -> Option<StoppedManagedProfile> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let key = held
            .iter()
            .find(|(_, profile)| {
                profile.manifest().id == profile_id
                    && profile.manifest().storage == ProfileStorage::Quick
            })
            .map(|(key, _)| key.clone())?;
        held.remove(&key)
    }

    pub fn create(
        &self,
        request: ProfileCreateRequest,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let result = (|| {
            let name = request.display_name.trim();
            if name.is_empty() {
                return Err(IpcError::invalid_request("display_name is required"));
            }
            let slug: String = name
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
                .collect();
            match request.storage {
                ProfileStorageKind::Quick => {
                    let profile = StoppedManagedProfile::create_quick(
                        &self.roots.profiles_parent,
                        &self.roots.runtime_parent,
                        name,
                    )
                    .map_err(map_error)?;
                    Ok((Some(self.hold(profile)), None, false))
                }
                ProfileStorageKind::Local => {
                    let root = request
                        .root
                        .as_deref()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| self.roots.profiles_parent.join(&slug));
                    let profile = StoppedManagedProfile::create_local(
                        &root,
                        &self.roots.runtime_parent,
                        name,
                    )
                    .map_err(map_error)?;
                    let root = profile.paths().root.clone();
                    drop(profile);
                    let info = self.info_for_root(&root)?;
                    Ok((Some(info), None, false))
                }
                ProfileStorageKind::Portable => {
                    let inspector = self.media.as_ref().ok_or_else(|| IpcError::Unavailable {
                        reason: "portable media inspection is not configured".into(),
                    })?;
                    let media_root = request
                        .media_root
                        .as_deref()
                        .map(PathBuf::from)
                        .ok_or_else(|| IpcError::invalid_request("media_root is required"))?;
                    let root = request
                        .root
                        .as_deref()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| media_root.join(&slug));
                    let (profile, _selector) = StoppedManagedProfile::create_portable(
                        &media_root,
                        &root,
                        &self.roots.runtime_parent,
                        name,
                        inspector.as_ref(),
                    )
                    .map_err(map_error)?;
                    let root = profile.paths().root.clone();
                    drop(profile);
                    let info = self.info_for_root(&root)?;
                    Ok((Some(info), None, false))
                }
                ProfileStorageKind::Connected => Err(IpcError::invalid_request(
                    "connected profiles are owned externally and cannot be created here",
                )),
                _ => Err(IpcError::invalid_request("unknown profile storage")),
            }
        })();
        self.outcome("create", result)
    }

    /// Promote a held Quick profile. The destination is published now; the
    /// daemon must restart from it before the source is released.
    pub fn promote(
        &self,
        request: ProfilePromoteRequest,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let result = (|| {
            if self.active.as_ref().is_some_and(|active| active.id == request.profile_id) {
                return Err(IpcError::Conflict {
                    message: "stop the daemon before promoting the profile it runs from".into(),
                });
            }
            let source = self
                .take_held_quick(&request.profile_id)
                .ok_or_else(|| IpcError::not_found("held quick profile", &request.profile_id))?;
            let destination = PathBuf::from(&request.destination);
            let pending =
                match source.promote_stopped_to_local(&destination, &self.roots.runtime_parent) {
                    Ok(pending) => pending,
                    Err(failure) => {
                        let error = map_error(failure.error().clone_for_report());
                        let source = failure.into_profile();
                        self.hold(source);
                        return Err(error);
                    }
                };
            let (promoted, source) = pending.publish_without_restart();
            self.hold(source);
            let info = self.hold(promoted);
            Ok((Some(info), None, true))
        })();
        self.outcome("promote", result)
    }

    fn snapshot_info(snapshot: &SnapshotRef) -> ProfileSnapshotInfo {
        let manifest = snapshot.manifest();
        let mut info = ProfileSnapshotInfo::default();
        info.snapshot_id = manifest.snapshot_id.clone();
        info.profile_id = manifest.profile_id.clone();
        info.profile_generation = manifest.profile_generation;
        info.identity_fingerprint = manifest.identity_hash.clone();
        info.created_at_unix = manifest.created_at_unix;
        info.root = snapshot.root().display().to_string();
        info.component_count = u32::try_from(manifest.components.len()).unwrap_or(u32::MAX);
        info
    }

    fn take_snapshot(
        &self,
        profile_id: &str,
        app_context: &AppContext,
    ) -> Result<(SnapshotRef, PathBuf), IpcError> {
        if let Some(active) = self.active.as_ref().filter(|active| active.id == profile_id) {
            let snapshot = snapshot_running_root(&active.root, app_context).map_err(map_error)?;
            return Ok((snapshot, active.root.clone()));
        }
        let held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(profile) = held.values().find(|profile| profile.manifest().id == profile_id) {
            let snapshot = profile.snapshot().map_err(map_error)?;
            return Ok((snapshot, profile.paths().root.clone()));
        }
        drop(held);
        let root = self.find_root(profile_id)?;
        let probe = probe_root(&root).map_err(map_error)?;
        if probe.manifest.storage == ProfileStorage::Quick {
            return Err(IpcError::Conflict {
                message: "quick profile is owned by another daemon".into(),
            });
        }
        let profile =
            StoppedManagedProfile::open(&root, &self.roots.runtime_parent).map_err(map_error)?;
        let snapshot = profile.snapshot().map_err(map_error)?;
        drop(profile);
        Ok((snapshot, root))
    }

    fn find_root(&self, profile_id: &str) -> Result<PathBuf, IpcError> {
        let inventory = self.inventory()?;
        inventory
            .profiles
            .into_iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| PathBuf::from(profile.root))
            .ok_or_else(|| IpcError::not_found("profile", profile_id))
    }

    pub fn snapshot(
        &self,
        request: ProfileSnapshotRequest,
        app_context: &AppContext,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let result = (|| {
            let (snapshot, root) = self.take_snapshot(&request.profile_id, app_context)?;
            let info = self.info_for_root(&root)?;
            Ok((Some(info), Some(Self::snapshot_info(&snapshot)), false))
        })();
        self.outcome("snapshot", result)
    }

    pub fn restore(
        &self,
        request: ProfileRestoreRequest,
        kind: &str,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let result = (|| {
            let snapshot =
                SnapshotRef::open(Path::new(&request.snapshot_root)).map_err(map_error)?;
            let profile = StoppedManagedProfile::restore_snapshot(
                &snapshot,
                Path::new(&request.destination),
                &self.roots.runtime_parent,
            )
            .map_err(map_error)?;
            let root = profile.paths().root.clone();
            drop(profile);
            let info = self.info_for_root(&root)?;
            Ok((Some(info), Some(Self::snapshot_info(&snapshot)), false))
        })();
        self.outcome(kind, result)
    }

    pub fn export(
        &self,
        request: ProfileExportRequest,
        app_context: &AppContext,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let result = (|| {
            let (snapshot, root) = self.take_snapshot(&request.profile_id, app_context)?;
            let exported =
                copy_snapshot(&snapshot, Path::new(&request.destination)).map_err(map_error)?;
            let info = self.info_for_root(&root)?;
            Ok((Some(info), Some(Self::snapshot_info(&exported)), false))
        })();
        self.outcome("export", result)
    }

    pub fn adopt(&self, request: ProfileAdoptRequest) -> Result<ProfileOperationOutcome, IpcError> {
        let result = (|| {
            let root = PathBuf::from(&request.root);
            let probe = probe_root(&root).map_err(map_error)?;
            if probe.leased {
                return Err(IpcError::Conflict {
                    message: "profile is owned by another daemon".into(),
                });
            }
            let profile = StoppedManagedProfile::open(&root, &self.roots.runtime_parent)
                .map_err(map_error)?;
            let info = if probe.manifest.storage == ProfileStorage::Quick {
                self.hold(profile)
            } else {
                let root = profile.paths().root.clone();
                drop(profile);
                self.info_for_root(&root)?
            };
            Ok((Some(info), None, false))
        })();
        self.outcome("adopt", result)
    }
}
