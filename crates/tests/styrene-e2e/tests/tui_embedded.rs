//! The TUI's owned-profile path: an ephemeral runtime profile runs as a
//! managed Quick profile, and a Standard runtime profile runs as a managed
//! Local profile that adopts the legacy layout once.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::time::Duration;

use styrene_ipc::types::ProfileStorageKind;
use styrene_session::SessionProfile;
use styrene_tui::{RuntimeProfile, StyrenePaths, TuiOptions};

fn paths(root: &std::path::Path) -> StyrenePaths {
    let paths = StyrenePaths::new(
        root.join("config"),
        root.join("data"),
        root.join("run/styrene.sock"),
        root.join("home"),
    );
    fs::create_dir_all(&paths.config_dir).unwrap();
    fs::create_dir_all(&paths.data_dir).unwrap();
    paths
}

#[tokio::test]
async fn ghost_runtime_runs_as_a_quick_profile_that_stops_with_the_session() {
    let root = tempfile::tempdir().expect("root");
    let options = TuiOptions { paths: paths(root.path()), runtime_profile: RuntimeProfile::Ghost };
    let mut session =
        tokio::time::timeout(Duration::from_secs(30), styrene_tui::start_profile_session(&options))
            .await
            .expect("start finishes")
            .expect("quick profile starts");
    assert_eq!(session.profile(), SessionProfile::Quick);
    let profile = session.profile_info().cloned().expect("daemon describes its profile");
    assert_eq!(profile.storage, ProfileStorageKind::Quick);
    assert!(profile.persistence.removed_on_release);
    let endpoint = session.metadata().endpoint.clone();
    assert!(endpoint.exists());

    let mut connection =
        styrene_tui::connect_with_retry(&endpoint).await.expect("TUI connects to its profile");
    let mut handle = connection.take_handle();
    let status = handle.status().await.expect("status");
    assert!(status.rns_initialized);
    drop(connection);

    let profile_root = std::path::PathBuf::from(&profile.root);
    session.close().await;
    assert!(!endpoint.exists(), "session close removes the socket");
    assert!(!profile_root.exists(), "quick root goes with the session");
}

#[tokio::test]
async fn standard_runtime_adopts_the_legacy_layout_into_a_local_profile_once() {
    let root = tempfile::tempdir().expect("root");
    let paths = paths(root.path());
    // A legacy install: identity and config in the config dir, database in
    // the data dir.
    let legacy_identity = rns_core::identity::PrivateIdentity::new_from_rand(rand_core::OsRng);
    let legacy_hash = hex::encode(legacy_identity.address_hash().as_slice());
    fs::write(paths.identity_path(), legacy_identity.to_private_key_bytes()).unwrap();
    fs::write(paths.config_path(), "role = \"propagation_client\"\n").unwrap();
    fs::create_dir_all(paths.config_dir.join("pages")).unwrap();
    fs::write(paths.config_dir.join("pages/index.mu"), b"legacy page").unwrap();
    let options = TuiOptions { paths: paths.clone(), runtime_profile: RuntimeProfile::Standard };

    let mut session =
        tokio::time::timeout(Duration::from_secs(30), styrene_tui::start_profile_session(&options))
            .await
            .expect("start finishes")
            .expect("local profile starts");
    assert_eq!(session.profile(), SessionProfile::Local);
    let profile = session.profile_info().cloned().expect("daemon describes its profile");
    assert_eq!(profile.storage, ProfileStorageKind::Local);
    assert!(profile.persistence.durable);
    assert_eq!(profile.custody.fingerprint, legacy_hash, "legacy identity is the profile identity");
    let profile_root = std::path::PathBuf::from(&profile.root);
    assert_eq!(
        fs::read_to_string(profile_root.join("config/config.toml")).unwrap(),
        "role = \"propagation_client\"\n"
    );
    assert_eq!(fs::read(profile_root.join("config/pages/index.mu")).unwrap(), b"legacy page");
    // The legacy files are untouched.
    assert_eq!(fs::read(paths.identity_path()).unwrap(), legacy_identity.to_private_key_bytes());
    let mut connection =
        styrene_tui::connect_with_retry(&session.metadata().endpoint).await.unwrap();
    let identity = connection.take_handle().identity().await.expect("identity");
    assert_eq!(identity.identity_hash, legacy_hash);
    drop(connection);
    session.close().await;
    assert!(profile_root.join("manifest.toml").is_file(), "local profile persists");

    // A second start reopens the same profile rather than adopting again.
    let mut again = styrene_tui::start_profile_session(&options).await.expect("reopen");
    assert_eq!(again.profile_info().map(|p| p.id.clone()), Some(profile.id.clone()));
    assert_eq!(again.profile_info().map(|p| p.generation), Some(profile.generation));
    again.close().await;
}
