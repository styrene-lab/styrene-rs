use std::fs;
use std::path::{Path, PathBuf};

use styrened::operator_profile::{ProfileStorage, StoppedManagedProfile};
use styrened::storage::messages::{MessageRecord, MessagesStore};

fn roots(test: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("create test root");
    let profiles = temp.path().join(test).join("profiles");
    let runtime = temp.path().join("r");
    fs::create_dir_all(&profiles).expect("create profile parent");
    fs::create_dir_all(&runtime).expect("create runtime parent");
    (temp, profiles, runtime)
}

fn assert_beneath(path: &Path, root: &Path) {
    assert!(path.starts_with(root), "{} is not beneath {}", path.display(), root.display());
}

#[test]
fn quick_profile_creates_one_private_durable_root_and_host_runtime() {
    let (_temp, profiles, runtime) = roots("quick");
    let profile = StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session")
        .expect("create Quick profile");

    assert_eq!(profile.manifest().storage, ProfileStorage::Quick);
    assert_eq!(profile.manifest().generation, 1);
    assert!(profile.paths().manifest.is_file());
    assert_eq!(fs::read(&profile.paths().identity).expect("read identity").len(), 64);

    for durable in [
        &profile.paths().manifest,
        &profile.paths().config,
        &profile.paths().public_identity,
        &profile.paths().pages,
        &profile.paths().identity,
        &profile.paths().custody,
        &profile.paths().messages,
        &profile.paths().nodes,
        &profile.paths().files,
        &profile.paths().snapshots,
    ] {
        assert_beneath(durable, &profile.paths().root);
    }
    assert_beneath(&profile.paths().runtime_root, &runtime.canonicalize().unwrap());
    assert!(!profile.paths().socket.starts_with(&profile.paths().root));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&profile.paths().root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&profile.paths().runtime_root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&profile.paths().identity).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn local_profile_reopens_the_same_manifest_and_paths() {
    let (_temp, profiles, runtime) = roots("local");
    let root = profiles.join("operator-home");
    let created = StoppedManagedProfile::create_local(&root, &runtime, "Home node")
        .expect("create Local profile");
    fs::write(&created.paths().config, "role = \"client\"\n").expect("write config");
    let identity = fs::read(&created.paths().identity).expect("read identity");
    drop(created);

    let reopened = StoppedManagedProfile::open(&root, &runtime).expect("open Local profile");
    assert_eq!(reopened.manifest().storage, ProfileStorage::Local);
    assert_eq!(reopened.manifest().display_name, "Home node");
    assert_eq!(fs::read(&reopened.paths().identity).unwrap(), identity);
    assert_eq!(fs::read_to_string(&reopened.paths().config).unwrap(), "role = \"client\"\n");
}

#[test]
fn second_writer_cannot_open_a_leased_profile() {
    let (_temp, profiles, runtime) = roots("lease");
    let root = profiles.join("operator-home");
    let owner = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();

    let error = StoppedManagedProfile::open(&root, &runtime)
        .expect_err("second writer must not acquire the profile");
    assert!(error.to_string().contains("already in use"));
    drop(owner);
    StoppedManagedProfile::open(&root, &runtime).expect("released profile should reopen");
}

#[tokio::test]
async fn managed_daemon_consumes_profile_paths_and_returns_stopped_lease() {
    let (_temp, profiles, runtime) = roots("managed-daemon");
    let profile =
        StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    fs::write(&profile.paths().config, "role = \"propagation_client\"\n").unwrap();
    let root = profile.paths().root.clone();
    let socket = profile.paths().socket.clone();
    let config = profile.paths().config.clone();
    let pages = profile.paths().pages.clone();
    let files = profile.paths().files.clone();
    let identity = fs::read(&profile.paths().identity).unwrap();
    let expected_identity =
        rns_core::identity::PrivateIdentity::from_private_key_bytes(&identity).unwrap();
    let expected_identity = hex::encode(expected_identity.address_hash().as_slice());

    let running = profile.start().await.expect("start managed daemon");
    assert_eq!(running.identity_hash(), expected_identity);
    assert_eq!(running.paths().config, config);
    assert_eq!(running.paths().pages, pages);
    assert_eq!(running.paths().files, files);
    assert!(root.join("data/messages.db").is_file());
    assert!(root.join("data/nodes.db").is_file());
    assert!(socket.exists());

    let stopped = running.shutdown().await;
    assert!(!socket.exists());
    let error = StoppedManagedProfile::open(&root, &runtime)
        .expect_err("returned stopped handle must retain the profile lease");
    assert!(error.to_string().contains("already in use"));
    drop(stopped);
    assert!(!root.exists(), "released Quick profile should be removed");
}

#[tokio::test]
async fn newly_created_profile_starts_with_default_config() {
    let (_temp, profiles, runtime) = roots("default-config");
    let profile =
        StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    assert_eq!(fs::read(&profile.paths().config).unwrap(), b"");

    let running = profile.start().await.expect("start newly created profile");
    let profile = running.shutdown().await;
    drop(profile);
}

#[tokio::test]
async fn promoted_restart_preserves_identity_and_committed_state() {
    let (_temp, profiles, runtime) = roots("promoted-restart");
    let profile =
        StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    fs::write(&profile.paths().config, "role = \"propagation_client\"\n").unwrap();
    let source_root = profile.paths().root.clone();
    let identity = fs::read(&profile.paths().identity).unwrap();
    let expected_identity =
        rns_core::identity::PrivateIdentity::from_private_key_bytes(&identity).unwrap();
    let expected_identity = hex::encode(expected_identity.address_hash().as_slice());

    let running = profile.start().await.expect("start Quick daemon");
    assert_eq!(running.identity_hash(), expected_identity);
    let stopped = running.shutdown().await;
    MessagesStore::open(&stopped.paths().messages)
        .unwrap()
        .insert_message(&MessageRecord {
            id: "promoted-message".into(),
            source: "source".into(),
            destination: "destination".into(),
            title: "Promotion".into(),
            content: "Committed before restart".into(),
            timestamp: 1_788_194_121,
            direction: "in".into(),
            fields: None,
            receipt_status: None,
            read: false,
        })
        .expect("commit message state");
    styrene_services::node_store::NodeStore::open(stopped.paths().nodes.to_str().unwrap())
        .unwrap()
        .accept_announce(
            "0123456789abcdef0123456789abcdef",
            1_788_194_121,
            Some("Promoted peer"),
            Some("announce"),
            Some("node"),
            None,
        )
        .expect("commit node state");
    fs::write(stopped.paths().pages.join("index.mu"), b"page-state").unwrap();
    fs::write(stopped.paths().files.join("attachment.bin"), b"file-state").unwrap();

    let destination = profiles.join("local");
    let pending =
        stopped.promote_stopped_to_local(&destination, &runtime).expect("stage promoted profile");
    let promoted = pending.start().await.expect("restart promoted daemon");

    assert_eq!(promoted.identity_hash(), expected_identity);
    assert_eq!(fs::read(promoted.paths().pages.join("index.mu")).unwrap(), b"page-state");
    assert_eq!(fs::read(promoted.paths().files.join("attachment.bin")).unwrap(), b"file-state");

    let promoted = promoted.shutdown().await.expect("stop promoted daemon");
    let message = MessagesStore::open(&promoted.paths().messages)
        .unwrap()
        .get_message("promoted-message")
        .unwrap()
        .expect("promoted message state");
    assert_eq!(message.content, "Committed before restart");
    let node =
        styrene_services::node_store::NodeStore::open(promoted.paths().nodes.to_str().unwrap())
            .unwrap()
            .get("0123456789abcdef0123456789abcdef")
            .unwrap()
            .expect("promoted node state");
    assert_eq!(node.display_name.as_deref(), Some("Promoted peer"));
    assert!(!source_root.exists(), "confirmed restart should remove Quick source");
    assert_eq!(promoted.manifest().storage, ProfileStorage::Local);
}

#[tokio::test]
async fn promotion_restart_rejects_changed_identity_and_keeps_source() {
    let (_temp, profiles, runtime) = roots("promotion-identity-change");
    let source = StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    fs::write(&source.paths().config, "role = \"propagation_client\"\n").unwrap();
    let source_root = source.paths().root.clone();
    let destination = profiles.join("local");
    let pending = source.promote_stopped_to_local(&destination, &runtime).unwrap();
    let replacement = rns_core::identity::PrivateIdentity::new_from_rand(rand_core::OsRng);
    fs::write(&pending.profile().paths().identity, replacement.to_private_key_bytes()).unwrap();

    let failure = pending.start().await.err().expect("changed identity must reject restart");
    assert!(failure.to_string().contains("promoted identity changed"));
    let source = failure.into_source();
    assert_eq!(source.paths().root, source_root);
    assert!(source.paths().identity.is_file());
    assert!(!destination.exists());
}

#[tokio::test]
async fn managed_daemon_rejects_invalid_config_without_releasing_lease() {
    let (_temp, profiles, runtime) = roots("invalid-managed-config");
    let profile =
        StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    let root = profile.paths().root.clone();
    fs::write(&profile.paths().config, "not valid = [toml").unwrap();

    let failure = profile.start().await.err().expect("invalid managed config must fail closed");
    assert!(failure.to_string().contains("load managed daemon config"));
    let profile = failure.into_profile();
    let error = StoppedManagedProfile::open(&root, &runtime)
        .expect_err("failed start must return the leased profile");
    assert!(error.to_string().contains("already in use"));
    drop(profile);
    assert!(!root.exists());
}

#[tokio::test]
async fn managed_start_failure_after_worker_spawn_can_retry_cleanly() {
    let (_temp, profiles, runtime) = roots("managed-start-rollback");
    let profile =
        StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    fs::write(&profile.paths().config, "role = \"propagation_client\"\n").unwrap();
    fs::create_dir(&profile.paths().socket).unwrap();

    let failure = profile.start().await.err().expect("socket bind must fail");
    let profile = failure.into_profile();
    fs::remove_dir(&profile.paths().socket).unwrap();
    let running = profile.start().await.expect("retry after rolled-back startup");
    let profile = running.shutdown().await;
    drop(profile);
}

#[test]
fn abandoned_quick_process_helper() {
    let Some(profiles) = std::env::var_os("STYRENE_TEST_ABANDONED_QUICK_PROFILES") else {
        return;
    };
    let runtime = std::env::var_os("STYRENE_TEST_ABANDONED_QUICK_RUNTIME").unwrap();
    let profile = StoppedManagedProfile::create_quick(
        Path::new(&profiles),
        Path::new(&runtime),
        "Abandoned session",
    )
    .unwrap();
    std::mem::forget(profile);
}

#[test]
fn reopened_quick_profile_retains_cleanup_semantics() {
    let (_temp, profiles, runtime) = roots("reopened-quick-cleanup");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("abandoned_quick_process_helper")
        .env("STYRENE_TEST_ABANDONED_QUICK_PROFILES", &profiles)
        .env("STYRENE_TEST_ABANDONED_QUICK_RUNTIME", &runtime)
        .status()
        .expect("run abandoned Quick helper");
    assert!(status.success());
    let root =
        fs::read_dir(&profiles).unwrap().next().expect("helper-created Quick root").unwrap().path();

    let reopened = StoppedManagedProfile::open(&root, &runtime).expect("reopen abandoned Quick");
    drop(reopened);
    assert!(!root.exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn managed_daemon_rejects_non_utf8_node_store_path() {
    use std::os::unix::ffi::OsStringExt;

    let temp = tempfile::tempdir().unwrap();
    let profiles = temp.path().join(std::ffi::OsString::from_vec(vec![b'p', 0xff]));
    let runtime = temp.path().join("r");
    fs::create_dir(&profiles).unwrap();
    fs::create_dir(&runtime).unwrap();
    let profile =
        StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    fs::write(&profile.paths().config, "role = \"propagation_client\"\n").unwrap();

    let failure = profile.start().await.err().expect("non-UTF-8 node path must not fall back");
    assert!(failure.to_string().contains("node store path is not valid UTF-8"));
}

#[test]
fn portable_manifest_is_rejected_until_encrypted_custody_exists() {
    let (_temp, profiles, runtime) = roots("portable-disabled");
    let root = profiles.join("operator-home");
    let profile =
        StoppedManagedProfile::create_local(&root, &runtime, "Home node").expect("create profile");
    let manifest_path = profile.paths().manifest.clone();
    drop(profile);
    let manifest = fs::read_to_string(&manifest_path)
        .unwrap()
        .replace("storage = \"local\"", "storage = \"portable\"");
    fs::write(&manifest_path, manifest).unwrap();

    let error = StoppedManagedProfile::open(&root, &runtime)
        .expect_err("unencrypted Portable profile must be rejected");
    assert!(error.to_string().contains("invalid profile manifest"));
}

#[test]
fn failed_open_does_not_allocate_a_runtime_directory() {
    let (_temp, profiles, runtime) = roots("failed-open");
    let root = profiles.join("operator-home");
    let profile =
        StoppedManagedProfile::create_local(&root, &runtime, "Home node").expect("create profile");
    let identity_path = profile.paths().identity.clone();
    drop(profile);
    fs::write(identity_path, b"not-an-identity").unwrap();
    assert_eq!(fs::read_dir(&runtime).unwrap().count(), 0);

    StoppedManagedProfile::open(&root, &runtime).expect_err("invalid identity must fail open");
    assert_eq!(fs::read_dir(&runtime).unwrap().count(), 0);
}

#[test]
fn dropping_unpromoted_quick_profile_removes_sensitive_root() {
    let (_temp, profiles, runtime) = roots("quick-cleanup");
    let profile =
        StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    let root = profile.paths().root.clone();
    assert!(root.join("identity/identity").is_file());

    drop(profile);
    assert!(!root.exists());
}

#[cfg(unix)]
#[test]
fn opening_profile_rejects_world_readable_identity() {
    use std::os::unix::fs::PermissionsExt;

    let (_temp, profiles, runtime) = roots("identity-permissions");
    let root = profiles.join("operator-home");
    let profile = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    let identity = profile.paths().identity.clone();
    drop(profile);
    fs::set_permissions(&identity, fs::Permissions::from_mode(0o644)).unwrap();

    let error = StoppedManagedProfile::open(&root, &runtime)
        .expect_err("world-readable identity must be rejected");
    assert!(error.to_string().contains("insecure owner or permissions"));
}

#[test]
fn opening_profile_rejects_oversized_manifest_without_runtime_allocation() {
    let (_temp, profiles, runtime) = roots("manifest-bound");
    let root = profiles.join("operator-home");
    let profile = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    let manifest = profile.paths().manifest.clone();
    drop(profile);
    fs::write(manifest, vec![b'x'; 16 * 1024 + 1]).unwrap();

    let error = StoppedManagedProfile::open(&root, &runtime)
        .expect_err("oversized manifest must be rejected");
    assert!(error.to_string().contains("exceeds its supported size"));
    assert_eq!(fs::read_dir(runtime).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn opening_profile_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let (_temp, profiles, runtime) = roots("symlink");
    let root = profiles.join("operator-home");
    let profile =
        StoppedManagedProfile::create_local(&root, &runtime, "Home node").expect("create profile");
    let outside = profiles.join("outside");
    fs::create_dir_all(&outside).expect("create outside path");
    fs::remove_dir_all(&profile.paths().pages).expect("remove pages directory");
    symlink(&outside, &profile.paths().pages).expect("link pages outside profile");
    drop(profile);

    let error = StoppedManagedProfile::open(&root, &runtime).expect_err("symlink must be rejected");
    assert!(error.to_string().contains("symbolic link"), "unexpected error: {error}");
}

#[test]
fn stopped_quick_profile_promotes_complete_state_without_changing_identity() {
    let (_temp, profiles, runtime) = roots("promote");
    let source = StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session")
        .expect("create Quick profile");
    fs::write(&source.paths().config, "role = \"client\"\n").unwrap();
    fs::write(&source.paths().public_identity, "display_name = \"Operator\"\n").unwrap();
    fs::write(&source.paths().messages, b"messages-state").unwrap();
    fs::write(&source.paths().nodes, b"nodes-state").unwrap();
    fs::write(source.paths().pages.join("index.mu"), b"page-state").unwrap();
    fs::write(source.paths().files.join("attachment.bin"), b"file-state").unwrap();
    let identity = fs::read(&source.paths().identity).unwrap();
    let destination = profiles.join("promoted-home");

    let source_root = source.paths().root.clone();
    let pending =
        source.promote_stopped_to_local(&destination, &runtime).expect("promote stopped profile");
    let promoted = pending.profile();

    assert_eq!(promoted.manifest().storage, ProfileStorage::Local);
    assert_eq!(promoted.manifest().generation, 2);
    assert_eq!(fs::read(&promoted.paths().identity).unwrap(), identity);
    assert_eq!(fs::read_to_string(&promoted.paths().config).unwrap(), "role = \"client\"\n");
    assert_eq!(fs::read(&promoted.paths().messages).unwrap(), b"messages-state");
    assert_eq!(fs::read(&promoted.paths().nodes).unwrap(), b"nodes-state");
    assert_eq!(fs::read(promoted.paths().pages.join("index.mu")).unwrap(), b"page-state");
    assert_eq!(fs::read(promoted.paths().files.join("attachment.bin")).unwrap(), b"file-state");
    assert!(source_root.join("manifest.toml").is_file(), "promotion must not mutate the source");
    drop(pending);
    assert!(!source_root.exists());
    assert!(!destination.exists());
}

#[test]
fn promotion_refuses_existing_destination_without_mutating_source() {
    let (_temp, profiles, runtime) = roots("collision");
    let source = StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    let destination = profiles.join("existing");
    fs::create_dir(&destination).unwrap();

    let source_root = source.paths().root.clone();
    let error = source
        .promote_stopped_to_local(&destination, &runtime)
        .expect_err("existing destination must be rejected");
    assert!(error.to_string().contains("already exists"), "unexpected error: {error}");
    assert!(source_root.join("manifest.toml").is_file());
    assert!(!destination.join("manifest.toml").exists());
}

#[test]
fn promotion_rejects_local_source_without_deleting_it() {
    let (_temp, profiles, runtime) = roots("local-source");
    let root = profiles.join("local");
    let source = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    let destination = profiles.join("destination");

    let failure = source
        .promote_stopped_to_local(&destination, &runtime)
        .expect_err("Local profile must not be promoted as Quick");
    assert!(failure.to_string().contains("only a stopped Quick"));
    let source = failure.into_profile();
    assert!(source.paths().manifest.is_file());
    assert!(!destination.exists());
}

#[test]
fn dropping_unconfirmed_promotion_rolls_back_both_temporary_roots() {
    let (_temp, profiles, runtime) = roots("pending-drop");
    let source = StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    let source_root = source.paths().root.clone();
    let destination = profiles.join("destination");
    let pending = source.promote_stopped_to_local(&destination, &runtime).unwrap();
    assert!(source_root.exists());
    assert!(destination.exists());

    drop(pending);
    assert!(!source_root.exists());
    assert!(!destination.exists());
}

#[cfg(unix)]
#[test]
fn failed_promotion_removes_stage_and_does_not_publish_destination() {
    use std::os::unix::fs::symlink;

    let (_temp, profiles, runtime) = roots("failed-stage");
    let source = StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    let outside = profiles.join("outside");
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, source.paths().files.join("escape")).unwrap();
    let destination = profiles.join("destination");

    let source_root = source.paths().root.clone();
    let error = source
        .promote_stopped_to_local(&destination, &runtime)
        .expect_err("symlinked state must fail promotion");
    assert!(error.to_string().contains("symbolic link"), "unexpected error: {error}");
    assert!(source_root.join("manifest.toml").is_file());
    assert!(!destination.exists());
    let stage_prefix = format!(".{}.stage-", destination.file_name().unwrap().to_string_lossy());
    assert!(
        fs::read_dir(&profiles).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(&stage_prefix)),
        "failed promotion left a staging directory"
    );
}

#[test]
fn stale_lease_file_without_a_live_owner_is_reclaimed_and_release_is_idempotent() {
    let (_temp, profiles, runtime) = roots("stale-lease");
    let root = profiles.join("operator-home");
    let owner = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    let lease_path = root.join(".profile.lock");
    assert!(lease_path.is_file(), "an owned profile carries its lease file");
    // Releasing twice is a no-op: the second open holds the lease again and
    // its release leaves the profile reopenable.
    drop(owner);
    let again = StoppedManagedProfile::open(&root, &runtime).expect("released lease reopens");
    drop(again);
    // A lease file left behind by a process that no longer exists holds no
    // lock, so the next owner reclaims it rather than failing closed forever.
    fs::write(&lease_path, b"stale").unwrap();
    let reclaimed = StoppedManagedProfile::open(&root, &runtime).expect("stale lease reclaimed");
    assert_eq!(reclaimed.manifest().display_name, "Home node");
    assert_eq!(fs::read_dir(&runtime).unwrap().count(), 1, "one runtime root per open owner");
    drop(reclaimed);
    assert_eq!(fs::read_dir(&runtime).unwrap().count(), 0, "release removes the runtime root");
}

// ── Snapshots ────────────────────────────────────────────────────────────────

use styrened::operator_profile::SnapshotRef;

fn sha256_hex(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(fs::read(path).unwrap()))
}

#[test]
fn stopped_profile_snapshot_records_component_hashes_in_an_immutable_generation() {
    let (_temp, profiles, runtime) = roots("snapshot-stopped");
    let root = profiles.join("operator-home");
    let profile = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    fs::write(&profile.paths().config, "role = \"client\"\n").unwrap();
    fs::write(profile.paths().pages.join("index.mu"), b"page-state").unwrap();
    fs::write(profile.paths().files.join("attachment.bin"), b"file-state").unwrap();
    MessagesStore::open(&profile.paths().messages).unwrap();

    let snapshot = profile.snapshot().expect("stopped snapshot");
    let manifest = snapshot.manifest();
    assert_eq!(manifest.profile_id, profile.manifest().id);
    assert_eq!(manifest.profile_generation, 1);
    assert_beneath(snapshot.root(), &profile.paths().snapshots);
    for component in [
        "config/config.toml",
        "config/pages/index.mu",
        "identity/identity",
        "data/files/attachment.bin",
        "data/messages.db",
    ] {
        let record =
            manifest.components.get(component).unwrap_or_else(|| panic!("{component} recorded"));
        assert_eq!(record.sha256, sha256_hex(&snapshot.root().join(component)), "{component}");
    }
    assert!(!manifest.components.keys().any(|key| key.ends_with("-wal") || key.ends_with("-shm")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode =
            fs::metadata(snapshot.root().join("identity/identity")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o400, "snapshot files are read-only");
    }
    assert!(
        fs::write(snapshot.root().join("config/config.toml"), b"x").is_err(),
        "generation is immutable"
    );
    assert_eq!(profile.snapshots().unwrap().len(), 1);
    let reopened = SnapshotRef::open(snapshot.root()).expect("snapshot verifies");
    assert_eq!(reopened.manifest(), manifest);
}

#[tokio::test]
async fn running_profile_snapshot_uses_online_backup_and_captures_committed_state() {
    let (_temp, profiles, runtime) = roots("snapshot-running");
    let profile =
        StoppedManagedProfile::create_quick(&profiles, &runtime, "Field session").unwrap();
    fs::write(&profile.paths().config, "role = \"propagation_client\"\n").unwrap();
    let running = profile.start().await.expect("start managed daemon");
    let identity_hash = running.identity_hash().to_string();
    running
        .app_context()
        .store()
        .lock()
        .unwrap()
        .insert_message(&MessageRecord {
            id: "live-message".into(),
            source: "source".into(),
            destination: "destination".into(),
            title: "Snapshot".into(),
            content: "Committed while running".into(),
            timestamp: 1_788_194_121,
            direction: "in".into(),
            fields: None,
            receipt_status: None,
            read: false,
        })
        .expect("commit live message");
    assert!(running.paths().messages.with_extension("db-wal").exists(), "live store has WAL state");

    let snapshot = running.snapshot().expect("running snapshot");
    assert_eq!(snapshot.manifest().identity_hash, identity_hash);
    assert!(snapshot.manifest().components.contains_key("data/messages.db"));
    assert!(snapshot.manifest().components.contains_key("data/nodes.db"));
    // The daemon keeps running and the live store stays usable after the backup.
    assert_eq!(running.identity_hash(), identity_hash);
    assert!(
        running
            .app_context()
            .store()
            .lock()
            .unwrap()
            .get_message("live-message")
            .unwrap()
            .is_some()
    );
    let stopped = running.shutdown().await;

    let destination = profiles.join("restored");
    let restored = StoppedManagedProfile::restore_snapshot(&snapshot, &destination, &runtime)
        .expect("restore");
    let message = MessagesStore::open(&restored.paths().messages)
        .unwrap()
        .get_message("live-message")
        .unwrap();
    assert_eq!(message.map(|m| m.content).as_deref(), Some("Committed while running"));
    drop(restored);
    drop(stopped);
}

#[test]
fn snapshot_restores_as_a_new_generation_without_modifying_the_snapshot() {
    let (_temp, profiles, runtime) = roots("snapshot-restore");
    let root = profiles.join("operator-home");
    let profile = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    fs::write(&profile.paths().config, "role = \"client\"\n").unwrap();
    fs::write(profile.paths().files.join("attachment.bin"), b"file-state").unwrap();
    let identity = fs::read(&profile.paths().identity).unwrap();
    let snapshot = profile.snapshot().unwrap();
    let before: Vec<(String, String)> = snapshot
        .manifest()
        .components
        .keys()
        .map(|key| (key.clone(), sha256_hex(&snapshot.root().join(key))))
        .collect();

    let destination = profiles.join("restored");
    let restored = StoppedManagedProfile::restore_snapshot(&snapshot, &destination, &runtime)
        .expect("restore");
    assert_eq!(restored.manifest().storage, ProfileStorage::Local);
    assert_eq!(restored.manifest().generation, 2);
    assert_eq!(restored.manifest().id, profile.manifest().id);
    assert_eq!(fs::read(&restored.paths().identity).unwrap(), identity);
    assert_eq!(fs::read_to_string(&restored.paths().config).unwrap(), "role = \"client\"\n");
    assert_eq!(fs::read(restored.paths().files.join("attachment.bin")).unwrap(), b"file-state");
    assert!(restored.paths().snapshots.is_dir());
    // Restored files are writable again; the snapshot is untouched.
    fs::write(&restored.paths().config, "role = \"full_node\"\n").unwrap();
    let after: Vec<(String, String)> = before
        .iter()
        .map(|(key, _)| (key.clone(), sha256_hex(&snapshot.root().join(key))))
        .collect();
    assert_eq!(before, after);
    snapshot.verify().expect("snapshot still verifies");
    assert!(
        StoppedManagedProfile::restore_snapshot(&snapshot, &destination, &runtime).is_err(),
        "existing destination is rejected"
    );
}

#[test]
fn tampered_snapshot_is_rejected_before_restore_publishes_anything() {
    let (_temp, profiles, runtime) = roots("snapshot-tamper");
    let root = profiles.join("operator-home");
    let profile = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    let snapshot = profile.snapshot().unwrap();
    let config = snapshot.root().join("config/config.toml");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fs::write(&config, "role = \"hub\"\n").unwrap();
    let error = SnapshotRef::open(snapshot.root()).expect_err("tampered snapshot must not open");
    assert!(error.to_string().contains("does not match"), "unexpected error: {error}");
    let destination = profiles.join("restored");
    let error = StoppedManagedProfile::restore_snapshot(&snapshot, &destination, &runtime)
        .expect_err("tampered snapshot must not restore");
    assert!(error.to_string().contains("does not match"), "unexpected error: {error}");
    assert!(!destination.exists());
    let stage_prefix = ".restored.restore-";
    assert!(
        fs::read_dir(&profiles).unwrap().all(|entry| {
            !entry.unwrap().file_name().to_string_lossy().starts_with(stage_prefix)
        })
    );
}

// ── Identity custody ─────────────────────────────────────────────────────────

use styrened::operator_profile::{ContinuityFailure, CustodyBackend, RecoveryOutcome};

fn identity_fingerprint(path: &Path) -> String {
    let bytes = fs::read(path).unwrap();
    let identity = rns_core::identity::PrivateIdentity::from_private_key_bytes(&bytes).unwrap();
    hex::encode(identity.address_hash().as_slice())
}

#[test]
fn custody_record_binds_only_the_daemon_rns_identity() {
    let (_temp, profiles, runtime) = roots("custody-boundary");
    let root = profiles.join("operator-home");
    let profile = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    let custody = profile.custody().expect("custody record");
    assert_eq!(custody.backend, CustodyBackend::File);
    assert_eq!(custody.fingerprint, identity_fingerprint(&profile.paths().identity));
    assert!(custody.recovery_slots.is_empty());
    assert!(custody.hardware_locator.is_none());
    let text = fs::read_to_string(&profile.paths().custody).unwrap();
    for foreign in ["sdk", "signer", "root_secret", "identity_id", "vault"] {
        assert!(!text.contains(foreign), "custody record must not carry {foreign}: {text}");
    }
    assert_beneath(&profile.paths().custody, &profile.paths().root);
    // A snapshot carries the custody record beside the identity it describes.
    let snapshot = profile.snapshot().unwrap();
    assert!(snapshot.manifest().components.contains_key("identity/custody.toml"));
    assert_eq!(snapshot.manifest().identity_hash, custody.fingerprint);
}

#[test]
fn recovery_enrollment_reproduces_the_fingerprint_and_rejects_wrong_passphrases() {
    let (_temp, profiles, runtime) = roots("custody-enroll");
    let root = profiles.join("operator-home");
    let profile = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    let key = fs::read(&profile.paths().identity).unwrap();
    let slot = profile.enroll_recovery_slot(b"correct horse").expect("enroll");
    let custody = profile.custody().unwrap();
    assert_eq!(custody.recovery_slots.len(), 1);
    let stored = &custody.recovery_slots[0];
    assert_eq!(stored.id, slot);
    assert_eq!(stored.kdf, "argon2id");
    assert!(
        !hex::decode(&stored.ciphertext)
            .unwrap()
            .windows(16)
            .any(|w| key.windows(16).any(|k| k == w))
    );
    assert_eq!(
        profile.verify_recovery_slot(&slot, b"correct horse").unwrap(),
        RecoveryOutcome::Continuity { fingerprint: custody.fingerprint.clone() }
    );
    assert_eq!(
        profile.verify_recovery_slot(&slot, b"wrong").unwrap(),
        RecoveryOutcome::ContinuityUnavailable {
            fingerprint: custody.fingerprint.clone(),
            reason: ContinuityFailure::WrongPassphrase,
        }
    );
    assert!(profile.verify_recovery_slot("missing", b"correct horse").is_err());
    assert_eq!(fs::read(&profile.paths().identity).unwrap(), key, "verification writes nothing");
}

#[test]
fn recovery_slot_with_mismatched_fingerprint_cannot_claim_continuity() {
    let (_temp, profiles, runtime) = roots("custody-mismatch");
    let root = profiles.join("operator-home");
    let profile = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    let slot = profile.enroll_recovery_slot(b"pass").unwrap();
    let custody_path = profile.paths().custody.clone();
    let other = rns_core::identity::PrivateIdentity::new_from_rand(rand_core::OsRng);
    let other_fingerprint = hex::encode(other.address_hash().as_slice());
    let original = profile.custody().unwrap();
    // Only the record's own fingerprint changes; the slot keeps its binding.
    let text = fs::read_to_string(&custody_path).unwrap().replacen(
        &original.fingerprint,
        &other_fingerprint,
        1,
    );
    fs::write(&custody_path, text).unwrap();
    let outcome = profile.verify_recovery_slot(&slot, b"pass").unwrap();
    assert_eq!(
        outcome,
        RecoveryOutcome::ContinuityUnavailable {
            fingerprint: other_fingerprint,
            reason: ContinuityFailure::FingerprintMismatch,
        }
    );
    assert!(!outcome.is_continuity());
}

#[tokio::test]
async fn hardware_custody_unavailable_without_a_recovery_slot_fails_closed() {
    let (_temp, profiles, runtime) = roots("custody-hardware-unavailable");
    let root = profiles.join("operator-home");
    let profile = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    let fingerprint = profile.custody().unwrap().fingerprint;
    profile.move_to_hardware_custody("token:serial-1").expect("hardware custody");
    let custody = profile.custody().unwrap();
    assert_eq!(custody.backend, CustodyBackend::Hardware);
    assert_eq!(custody.hardware_locator.as_deref(), Some("token:serial-1"));
    assert!(!profile.paths().identity.exists(), "plaintext key left the profile");
    assert!(!profile.identity_available());

    let outcome = profile.abandon_hardware_custody(None).unwrap();
    assert_eq!(
        outcome,
        RecoveryOutcome::ContinuityUnavailable {
            fingerprint: fingerprint.clone(),
            reason: ContinuityFailure::NoRecoverySlot,
        }
    );
    let outcome = profile.abandon_hardware_custody(Some(b"anything")).unwrap();
    assert!(!outcome.is_continuity());
    assert!(!profile.paths().identity.exists(), "no replacement identity is created");
    assert_eq!(
        profile.custody().unwrap().fingerprint,
        fingerprint,
        "old identity is not overwritten"
    );
    let failure = profile.start().await.err().expect("profile without its key must not start");
    assert!(failure.to_string().contains("identity"), "unexpected error: {failure}");
    let profile = failure.into_profile();
    assert!(!profile.paths().identity.exists());
}

#[tokio::test]
async fn hardware_abandonment_with_enrolled_recovery_restores_verified_continuity() {
    let (_temp, profiles, runtime) = roots("custody-hardware-recover");
    let root = profiles.join("operator-home");
    let profile = StoppedManagedProfile::create_local(&root, &runtime, "Home node").unwrap();
    fs::write(&profile.paths().config, "role = \"propagation_client\"\n").unwrap();
    let fingerprint = profile.custody().unwrap().fingerprint;
    let key = fs::read(&profile.paths().identity).unwrap();
    profile.enroll_recovery_slot(b"field passphrase").unwrap();
    profile.move_to_hardware_custody("token:serial-2").unwrap();
    assert!(!profile.identity_available());

    let wrong = profile.abandon_hardware_custody(Some(b"not it")).unwrap();
    assert_eq!(
        wrong,
        RecoveryOutcome::ContinuityUnavailable {
            fingerprint: fingerprint.clone(),
            reason: ContinuityFailure::WrongPassphrase,
        }
    );
    assert!(!profile.paths().identity.exists());
    assert_eq!(profile.custody().unwrap().backend, CustodyBackend::Hardware);

    let recovered = profile.abandon_hardware_custody(Some(b"field passphrase")).unwrap();
    assert_eq!(recovered, RecoveryOutcome::Continuity { fingerprint: fingerprint.clone() });
    assert_eq!(fs::read(&profile.paths().identity).unwrap(), key);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&profile.paths().identity).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let custody = profile.custody().unwrap();
    assert_eq!(custody.backend, CustodyBackend::File);
    assert_eq!(custody.recovery_slots.len(), 1, "slots survive recovery");
    assert!(profile.identity_available());
    assert!(
        profile.abandon_hardware_custody(Some(b"field passphrase")).is_err(),
        "not hardware custody any more"
    );
    let running = profile.start().await.expect("recovered profile starts");
    assert_eq!(running.identity_hash(), fingerprint);
    let profile = running.shutdown().await;
    drop(profile);
}
