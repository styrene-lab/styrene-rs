//! The typed operator profile lifecycle over the daemon facade: inventory,
//! creation, promotion, snapshots, restore, export, import, adoption,
//! progress, restart-required outcomes, and authorization.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use styrene_ipc::IpcError;
use styrene_ipc::traits::{Daemon, DaemonProfiles};
use styrene_ipc::types::{
    ProfileAdoptRequest, ProfileCreateRequest, ProfileExportRequest, ProfileOperationState,
    ProfilePromoteRequest, ProfileRestoreRequest, ProfileSnapshotRequest, ProfileStorageKind,
};
use styrene_rbac::{RbacPolicy, Role};
use styrened::app_context::AppContext;
use styrened::daemon_facade::DaemonFacade;
use styrened::operator_profile::StoppedManagedProfile;
use styrened::profile_manager::{ProfileManager, ProfileRoots};
use styrened::services::PolicyService;
use styrened::storage::messages::MessagesStore;
use styrened::transport::mesh_transport::MeshTransport;
use styrened::transport::null_transport::NullTransport;

fn roots(test: &str) -> (tempfile::TempDir, ProfileRoots) {
    let temp = tempfile::tempdir().expect("test root");
    let profiles_parent = temp.path().join(test).join("profiles");
    let runtime_parent = temp.path().join("r");
    fs::create_dir_all(&profiles_parent).unwrap();
    fs::create_dir_all(&runtime_parent).unwrap();
    (temp, ProfileRoots { profiles_parent, runtime_parent })
}

fn context(default_role: Role) -> Arc<AppContext> {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    let node_store = Arc::new(styrene_services::node_store::NodeStore::open(":memory:").unwrap());
    let mut policy = RbacPolicy::new(default_role);
    policy.normalize_quiet();
    Arc::new(AppContext::with_policy(
        transport,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        store,
        node_store,
        PolicyService::new(policy),
    ))
}

fn facade(roots: &ProfileRoots, role: Role) -> DaemonFacade {
    let manager = ProfileManager::new(roots.clone(), None);
    DaemonFacade::new(context(role), "caller".into()).with_profiles(Arc::new(manager))
}

#[tokio::test]
async fn facades_without_a_manager_report_profiles_unavailable() {
    let facade = DaemonFacade::new(context(Role::Admin), "caller".into());
    let error = facade.profile_inventory().await.unwrap_err();
    assert!(matches!(error, IpcError::Unavailable { .. }), "unexpected: {error:?}");
    let stub: Arc<dyn Daemon> = Arc::new(styrene_ipc::StubDaemon);
    assert!(matches!(stub.profile_inventory().await, Err(IpcError::NotImplemented { .. })));
}

#[tokio::test]
async fn profile_mutations_require_the_profile_manage_capability() {
    let (_temp, roots) = roots("authz");
    let facade = facade(&roots, Role::Peer);
    let inventory = facade.profile_inventory().await.expect("peers may read the inventory");
    assert!(inventory.profiles.is_empty());
    let mut request = ProfileCreateRequest::default();
    request.storage = ProfileStorageKind::Quick;
    request.display_name = "Denied".into();
    let error = facade.create_profile(request).await.unwrap_err();
    assert!(matches!(error, IpcError::Denied { ref capability } if capability == "profile.manage"));
    assert!(
        fs::read_dir(&roots.profiles_parent).unwrap().next().is_none(),
        "denied call creates nothing"
    );
}

#[tokio::test]
async fn inventory_describes_ownership_persistence_custody_and_policy() {
    let (_temp, roots) = roots("inventory");
    let facade = facade(&roots, Role::Admin);
    let mut quick = ProfileCreateRequest::default();
    quick.storage = ProfileStorageKind::Quick;
    quick.display_name = "Field session".into();
    let created = facade.create_profile(quick).await.expect("quick profile");
    assert_eq!(created.progress.state, ProfileOperationState::Completed);
    assert!(!created.restart_required);
    let quick_info = created.profile.expect("profile info");
    assert_eq!(quick_info.storage, ProfileStorageKind::Quick);
    assert!(quick_info.ownership.held_by_daemon);
    assert!(!quick_info.ownership.leased_elsewhere);
    assert!(quick_info.persistence.removed_on_release);
    assert!(!quick_info.persistence.durable);
    assert_eq!(quick_info.custody.backend, "file");
    assert_eq!(quick_info.custody.fingerprint.len(), 32);
    assert!(quick_info.custody.identity_available);
    assert!(quick_info.network_policy.conservative_defaults);

    let mut local = ProfileCreateRequest::default();
    local.storage = ProfileStorageKind::Local;
    local.display_name = "Home node".into();
    local.root = Some(roots.profiles_parent.join("home").display().to_string());
    let created = facade.create_profile(local).await.expect("local profile");
    let local_info = created.profile.expect("profile info");
    assert!(local_info.persistence.durable);
    assert!(!local_info.ownership.held_by_daemon);
    assert!(!local_info.ownership.leased_elsewhere);

    // A profile leased by another owner is reported, not stolen.
    let elsewhere = roots.profiles_parent.join("elsewhere");
    let owner =
        StoppedManagedProfile::create_local(&elsewhere, &roots.runtime_parent, "Other owner")
            .unwrap();
    let inventory = facade.profile_inventory().await.unwrap();
    assert_eq!(inventory.profiles.len(), 3);
    let other = inventory.profiles.iter().find(|p| p.display_name == "Other owner").unwrap();
    assert!(other.ownership.leased_elsewhere);
    assert!(inventory.active_profile_id.is_none());
    drop(owner);

    let progress = facade.profile_operation(&created.progress.operation_id).await.unwrap();
    assert_eq!(progress.kind, "create");
    assert_eq!(progress.profile_id.as_deref(), Some(local_info.id.as_str()));
    assert!(matches!(facade.profile_operation("missing").await, Err(IpcError::NotFound { .. })));
}

#[tokio::test]
async fn promotion_publishes_the_destination_and_requires_a_restart() {
    let (_temp, roots) = roots("promote");
    let facade = facade(&roots, Role::Admin);
    let mut quick = ProfileCreateRequest::default();
    quick.storage = ProfileStorageKind::Quick;
    quick.display_name = "Field session".into();
    let quick = facade.create_profile(quick).await.unwrap().profile.unwrap();
    let source_root = PathBuf::from(&quick.root);
    fs::write(source_root.join("config/config.toml"), "role = \"client\"\n").unwrap();

    let mut request = ProfilePromoteRequest::default();
    request.profile_id = quick.id.clone();
    request.destination = roots.profiles_parent.join("promoted").display().to_string();
    let outcome = facade.promote_profile(request.clone()).await.expect("promotion");
    assert!(outcome.restart_required, "promotion takes effect on restart");
    let promoted = outcome.profile.unwrap();
    assert_eq!(promoted.storage, ProfileStorageKind::Local);
    assert_eq!(promoted.generation, 2);
    assert_eq!(promoted.custody.fingerprint, quick.custody.fingerprint);
    assert!(promoted.ownership.held_by_daemon);
    assert!(source_root.join("manifest.toml").is_file(), "source stays until restart confirms");
    let inventory = facade.profile_inventory().await.unwrap();
    assert_eq!(inventory.profiles.len(), 2);

    // A second promotion of the same source is refused; the destination exists.
    let error = facade.promote_profile(request).await.unwrap_err();
    assert!(matches!(error, IpcError::NotFound { .. } | IpcError::Conflict { .. }), "{error:?}");
    let mut collision = ProfilePromoteRequest::default();
    collision.profile_id = "0123456789abcdef0123456789abcdef".into();
    collision.destination = roots.profiles_parent.join("promoted").display().to_string();
    assert!(matches!(facade.promote_profile(collision).await, Err(IpcError::NotFound { .. })));
}

#[tokio::test]
async fn snapshot_restore_export_import_and_adopt_round_trip() {
    let (temp, roots) = roots("snapshots");
    let facade = facade(&roots, Role::Admin);
    let mut local = ProfileCreateRequest::default();
    local.storage = ProfileStorageKind::Local;
    local.display_name = "Home node".into();
    let local = facade.create_profile(local).await.unwrap().profile.unwrap();
    let root = PathBuf::from(&local.root);
    fs::write(root.join("data/files/note.txt"), b"kept").unwrap();

    let mut snapshot = ProfileSnapshotRequest::default();
    snapshot.profile_id = local.id.clone();
    let outcome = facade.snapshot_profile(snapshot).await.expect("snapshot");
    let snapshot = outcome.snapshot.expect("snapshot info");
    assert_eq!(snapshot.profile_id, local.id);
    assert_eq!(snapshot.identity_fingerprint, local.custody.fingerprint);
    assert!(snapshot.component_count >= 3);
    assert_eq!(outcome.profile.unwrap().persistence.snapshot_count, 1);

    let mut restore = ProfileRestoreRequest::default();
    restore.snapshot_root = snapshot.root.clone();
    restore.destination = roots.profiles_parent.join("restored").display().to_string();
    let restored = facade.restore_profile(restore.clone()).await.expect("restore").profile.unwrap();
    assert_eq!(restored.generation, 2);
    assert_eq!(restored.custody.fingerprint, local.custody.fingerprint);
    assert_eq!(
        fs::read(PathBuf::from(&restored.root).join("data/files/note.txt")).unwrap(),
        b"kept"
    );
    assert!(matches!(facade.restore_profile(restore).await, Err(IpcError::Conflict { .. })));

    let mut export = ProfileExportRequest::default();
    export.profile_id = local.id.clone();
    export.destination = temp.path().join("exported-snapshot").display().to_string();
    let exported = facade.export_profile(export).await.expect("export").snapshot.unwrap();
    assert!(PathBuf::from(&exported.root).join("manifest.toml").is_file());
    assert!(!PathBuf::from(&exported.root).starts_with(&root), "export lives outside the profile");

    let mut import = ProfileRestoreRequest::default();
    import.snapshot_root = exported.root.clone();
    import.destination = temp.path().join("imported").display().to_string();
    let imported = facade.import_profile(import).await.expect("import").profile.unwrap();
    assert_eq!(imported.custody.fingerprint, local.custody.fingerprint);
    assert!(!imported.ownership.held_by_daemon);

    let mut adopt = ProfileAdoptRequest::default();
    adopt.root = imported.root.clone();
    let adopted = facade.adopt_profile(adopt).await.expect("adopt").profile.unwrap();
    assert_eq!(adopted.id, imported.id);
    let mut tampered = ProfileRestoreRequest::default();
    tampered.snapshot_root = exported.root.clone();
    tampered.destination = temp.path().join("tampered").display().to_string();
    let config = PathBuf::from(&exported.root).join("config/config.toml");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fs::write(&config, "role = \"hub\"\n").unwrap();
    let error = facade.import_profile(tampered).await.unwrap_err();
    assert!(matches!(error, IpcError::InvalidRequest { .. }), "{error:?}");
    assert!(!temp.path().join("tampered").exists());
}
