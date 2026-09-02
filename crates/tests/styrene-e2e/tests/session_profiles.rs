//! Managed and Connected sessions expose one daemon contract.
//!
//! A Quick session starts a managed profile's daemon in-process and reaches
//! it over the profile's host-private socket. A Connected session opened
//! against that same endpoint must return the same typed records and the
//! same profile truth, and closing the Quick session must take the endpoint
//! away rather than leave a runtime behind.

use std::path::PathBuf;

use styrene_ipc::types::ProfileStorageKind;
use styrene_session::{
    EmbeddedConfig, ManagedTarget, ProfileRoots, Session, SessionError, SessionProfile,
};

fn comparable_capabilities(
    status: &styrene_ipc::types::DaemonStatusInfo,
) -> Option<(u16, Vec<String>, Vec<String>)> {
    status.active_capabilities.as_ref().map(|capabilities| {
        (
            capabilities.version,
            capabilities.runtime.clone(),
            capabilities.authorized_operations.clone(),
        )
    })
}

#[tokio::test]
async fn live_and_embedded_sessions_return_equivalent_records() {
    let data = tempfile::tempdir().expect("data dir");
    let mut embedded = Session::embedded(EmbeddedConfig {
        db: Some(data.path().join("messages.db")),
        config: None,
        identity: None,
        ephemeral: true,
    })
    .await
    .expect("embedded runtime starts");
    assert_eq!(embedded.profile(), SessionProfile::Quick);
    let endpoint = embedded.metadata().endpoint.clone();
    assert!(endpoint.exists(), "embedded session owns a private socket");

    let mut live = Session::live(&endpoint).await.expect("live session over the embedded endpoint");
    assert_eq!(live.profile(), SessionProfile::Connected);
    assert!(live.generation() > embedded.generation());
    // Two IPC connections to one daemon carry distinct daemon connection generations.
    assert_ne!(live.metadata().daemon_generation, embedded.metadata().daemon_generation);

    let embedded_identity = embedded.client().identity().await.expect("embedded identity");
    let live_identity = live.client().identity().await.expect("live identity");
    assert_eq!(embedded_identity, live_identity);

    let embedded_status = embedded.client().status().await.expect("embedded status");
    let live_status = live.client().status().await.expect("live status");
    assert_eq!(embedded_status.daemon_version, live_status.daemon_version);
    assert_eq!(embedded_status.rns_initialized, live_status.rns_initialized);
    assert_eq!(comparable_capabilities(&embedded_status), comparable_capabilities(&live_status));
    assert!(comparable_capabilities(&live_status).is_some(), "daemon advertises capabilities");

    let embedded_devices = embedded.client().devices(false).await.expect("embedded devices");
    let live_devices = live.client().devices(false).await.expect("live devices");
    assert_eq!(embedded_devices, live_devices);

    // Closing the Embedded session shuts down its runtime and endpoint. The
    // Live session reports a typed failure; nothing restarts on its behalf.
    embedded.close().await;
    embedded.close().await;
    assert!(!endpoint.exists(), "embedded shutdown removes its socket");
    assert!(live.client().status().await.is_err());
    assert!(matches!(
        Session::live(&endpoint).await,
        Err(SessionError::Connect { profile: SessionProfile::Connected, .. })
    ));
    live.close().await;
}

/// Both frontends address a backend profile through the same session layer:
/// a managed session and a Connected session to its endpoint must report the
/// same profile truth, and neither derives it from a local mode name.
#[tokio::test]
async fn managed_and_connected_sessions_report_the_same_profile_truth() {
    let temp = tempfile::tempdir().expect("roots");
    let roots = ProfileRoots {
        profiles_parent: temp.path().join("p"),
        runtime_parent: temp.path().join("r"),
    };
    std::fs::create_dir_all(&roots.profiles_parent).unwrap();
    std::fs::create_dir_all(&roots.runtime_parent).unwrap();
    let mut owner =
        Session::managed(ManagedTarget::Quick { roots: roots.clone(), display_name: "Field kit" })
            .await
            .expect("quick session");
    assert_eq!(owner.profile(), SessionProfile::Quick);
    assert!(owner.profile().owns_daemon());
    let owner_profile =
        owner.profile_info().cloned().expect("managed daemon describes its profile");
    assert_eq!(owner_profile.storage, ProfileStorageKind::Quick);
    assert_eq!(owner_profile.display_name, "Field kit");
    assert!(owner_profile.ownership.active);
    assert!(owner_profile.persistence.removed_on_release);
    assert!(owner_profile.network_policy.conservative_defaults);
    assert_eq!(owner_profile.custody.backend, "file");

    let endpoint = owner.metadata().endpoint.clone();
    let mut viewer = Session::connected(&endpoint).await.expect("connected session");
    assert_eq!(viewer.profile(), SessionProfile::Connected);
    assert!(!viewer.profile().owns_daemon());
    let viewer_profile =
        viewer.profile_info().cloned().expect("connected session sees profile truth");
    assert_eq!(viewer_profile, owner_profile);
    let owner_inventory = owner.client().profile_inventory().await.expect("owner inventory");
    let viewer_inventory = viewer.client().profile_inventory().await.expect("viewer inventory");
    assert_eq!(owner_inventory, viewer_inventory);
    assert_eq!(owner_inventory.active_profile_id.as_deref(), Some(owner_profile.id.as_str()));

    // A Local profile persists across sessions and is reopened, not recreated.
    let local_root = roots.profiles_parent.join("home");
    let mut local = Session::managed(ManagedTarget::Local {
        root: local_root.clone(),
        runtime_parent: roots.runtime_parent.clone(),
        display_name: Some("Home node"),
    })
    .await
    .expect("local session");
    let local_id = local.profile_info().map(|p| p.id.clone()).expect("local profile");
    let fingerprint = local.profile_info().map(|p| p.custody.fingerprint.clone()).unwrap();
    local.close().await;
    assert!(local_root.join("manifest.toml").is_file(), "local profile persists");
    let mut reopened = Session::managed(ManagedTarget::Local {
        root: local_root.clone(),
        runtime_parent: roots.runtime_parent.clone(),
        display_name: None,
    })
    .await
    .expect("reopen local");
    assert_eq!(reopened.profile_info().map(|p| p.id.clone()), Some(local_id));
    assert_eq!(reopened.profile_info().map(|p| p.custody.fingerprint.clone()), Some(fingerprint));
    reopened.close().await;

    viewer.close().await;
    let quick_root = PathBuf::from(owner_profile.root);
    owner.close().await;
    assert!(!quick_root.exists(), "quick profile is removed when its session closes");
    assert!(!endpoint.exists());
}
