use styrene_ipc::types::{
    IdentityBackupExport, IdentityBackupFormat, IdentityBackupImport, IdentityBackupMetadata,
    IdentityRestoreOutcome,
};

#[test]
fn identity_backup_dtos_round_trip_without_joining_identity_state() {
    let metadata_json = serde_json::json!({
        "contract_version": 1,
        "format": "stid_v1",
        "encrypted_size": 97
    });
    let metadata: IdentityBackupMetadata = serde_json::from_value(metadata_json.clone()).unwrap();
    assert_eq!(metadata.format, IdentityBackupFormat::StidV1);
    assert_eq!(serde_json::to_value(metadata).unwrap(), metadata_json);

    let export_json = serde_json::json!({
        "metadata": metadata_json,
        "encrypted_bytes": [83, 84, 73, 68]
    });
    let exported: IdentityBackupExport = serde_json::from_value(export_json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&exported).unwrap(), export_json);

    let import: IdentityBackupImport = serde_json::from_value(serde_json::json!({
        "encrypted_bytes": [83, 84, 73, 68]
    }))
    .unwrap();
    assert_eq!(import.encrypted_bytes, exported.encrypted_bytes);
    assert_eq!(
        serde_json::from_str::<IdentityRestoreOutcome>("\"already_present\"").unwrap(),
        IdentityRestoreOutcome::AlreadyPresent
    );

    let identity = serde_json::to_value(styrene_ipc::types::IdentityInfo::default()).unwrap();
    assert!(identity.get("encrypted_bytes").is_none());
}

#[test]
fn opaque_artifact_debug_is_redacted() {
    let mut exported = IdentityBackupExport::default();
    exported.encrypted_bytes = b"private-looking-artifact".to_vec();
    let mut imported = IdentityBackupImport::default();
    imported.encrypted_bytes = exported.encrypted_bytes.clone();

    for debug in [format!("{exported:?}"), format!("{imported:?}")] {
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("private-looking-artifact"));
    }
}

#[test]
fn additive_defaults_and_unknown_format_remain_compatible() {
    let metadata: IdentityBackupMetadata = serde_json::from_value(serde_json::json!({
        "format": "future_format"
    }))
    .unwrap();
    assert_eq!(metadata.contract_version, 0);
    assert_eq!(metadata.format, IdentityBackupFormat::Unknown);
    assert_eq!(metadata.encrypted_size, 0);
}
