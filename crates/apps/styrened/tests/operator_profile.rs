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
