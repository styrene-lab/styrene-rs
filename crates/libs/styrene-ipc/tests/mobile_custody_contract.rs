use styrene_ipc::types::{
    IdentityCustodyAuthentication, IdentityCustodyAvailability, IdentityCustodyBackend,
    IdentityCustodyDowngrade, IdentityCustodyInfo, IdentityCustodyProtection,
};

#[test]
fn custody_dto_is_closed_and_round_trips() {
    let value = serde_json::json!({
        "requested_backend": "encrypted_file",
        "active_backend": "encrypted_file",
        "protection": "encrypted_at_rest",
        "authentication": "host_key_material",
        "availability": "available",
        "downgrade": "none",
        "failure": null
    });
    let custody: IdentityCustodyInfo = serde_json::from_value(value.clone()).unwrap();

    assert_eq!(custody.requested_backend, IdentityCustodyBackend::EncryptedFile);
    assert_eq!(custody.active_backend, Some(IdentityCustodyBackend::EncryptedFile));
    assert_eq!(custody.protection, Some(IdentityCustodyProtection::EncryptedAtRest));
    assert_eq!(custody.authentication, IdentityCustodyAuthentication::HostKeyMaterial);
    assert_eq!(custody.availability, IdentityCustodyAvailability::Available);
    assert_eq!(custody.downgrade, IdentityCustodyDowngrade::None);
    assert_eq!(serde_json::to_value(custody).unwrap(), value);

    let mut unknown = value;
    unknown["private_key"] = serde_json::json!("secret");
    assert!(serde_json::from_value::<IdentityCustodyInfo>(unknown).is_err());
}

#[test]
fn requested_active_mismatch_is_explicit_and_secret_free() {
    let value = serde_json::json!({
        "requested_backend": "keychain",
        "active_backend": "plaintext_file",
        "protection": "development_plaintext",
        "authentication": "none",
        "availability": "available",
        "downgrade": "active_backend_mismatch",
        "failure": {
            "code": "backend_failure",
            "retryable": false
        }
    });

    let custody: IdentityCustodyInfo = serde_json::from_value(value).unwrap();

    assert_eq!(custody.requested_backend, IdentityCustodyBackend::Keychain);
    assert_eq!(custody.active_backend, Some(IdentityCustodyBackend::PlaintextFile));
    assert_eq!(custody.protection, Some(IdentityCustodyProtection::DevelopmentPlaintext));
    assert_eq!(custody.downgrade, IdentityCustodyDowngrade::ActiveBackendMismatch);
    let encoded = serde_json::to_string(&custody).unwrap();
    for forbidden in ["private", "passphrase", "credential", "key_material", "export"] {
        assert!(!encoded.contains(forbidden), "mismatch projection leaked {forbidden}");
    }
}
