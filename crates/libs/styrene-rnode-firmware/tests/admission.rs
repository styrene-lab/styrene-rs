use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use styrene_rnode_firmware::{
    ArchiveMember, ArtifactDecisionReason, ExecutorClass, FirmwareManifest, FirmwareOperation,
    ManifestArtifact, ManifestImage, ManifestRecovery, ManifestTarget, MemoryRegion, Sha256Digest,
    SignedFirmwareManifest, TargetObservation, admit_artifact,
};

fn digest(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

type Fixture =
    (SigningKey, SignedFirmwareManifest, Vec<u8>, Vec<(String, Vec<u8>)>, TargetObservation);

fn fixture() -> Fixture {
    let archive = b"synthetic archive".to_vec();
    let application = b"application image".to_vec();
    let manifest = FirmwareManifest {
        schema_version: 1,
        manifest_id: "synthetic-exact-v1".into(),
        firmware_version: "1.86".into(),
        operations: vec![FirmwareOperation::Upgrade],
        target: ManifestTarget {
            board: "exact-board".into(),
            radio_variant: "sx1262".into(),
            hardware_revision: "rev-a".into(),
            executor: ExecutorClass::HostSerialEsp,
        },
        artifact: ManifestArtifact {
            archive_sha256: digest(&archive),
            max_expanded_bytes: 1024,
            expected_members: vec!["application.bin".into()],
        },
        images: vec![ManifestImage {
            member: "application.bin".into(),
            region: MemoryRegion { offset: 0x10000, length: application.len() as u64 },
            sha256: digest(&application),
            application: true,
        }],
        protected_regions: vec![MemoryRegion { offset: 0x9000, length: 0x1000 }],
        recovery: ManifestRecovery {
            executor: ExecutorClass::HostSerialEsp,
            procedure_id: "synthetic-recovery".into(),
            physical_mode: "rom_serial_bootloader".into(),
            tool_id: "bounded_host_serial_esp".into(),
            power_condition: "stable_usb_power".into(),
        },
    };
    let payload = serde_json::to_vec(&manifest).expect("manifest payload");
    let key = SigningKey::from_bytes(&[0x42; 32]);
    let signed = SignedFirmwareManifest {
        payload: payload.clone(),
        signature: key.sign(&payload).to_bytes().to_vec(),
    };
    let target = TargetObservation::new(
        styrene_rnode_firmware::McuFamily::Esp32,
        styrene_rnode_firmware::ConfigurationState::Yes,
    )
    .with_hardware(
        Some("exact-board".into()),
        Some("sx1262".into()),
        Some("rev-a".into()),
        Some("esp_rom_serial".into()),
    );
    (key, signed, archive, vec![("application.bin".into(), application)], target)
}

fn members(values: &[(String, Vec<u8>)]) -> Vec<ArchiveMember<'_>> {
    values.iter().map(|(path, bytes)| ArchiveMember { path, bytes }).collect()
}

fn resign(
    key: &SigningKey,
    signed: &SignedFirmwareManifest,
    mutate: impl FnOnce(&mut FirmwareManifest),
) -> SignedFirmwareManifest {
    let mut manifest: FirmwareManifest =
        serde_json::from_slice(&signed.payload).expect("fixture manifest");
    mutate(&mut manifest);
    let payload = serde_json::to_vec(&manifest).expect("mutated manifest");
    SignedFirmwareManifest { signature: key.sign(&payload).to_bytes().to_vec(), payload }
}

#[test]
fn exact_signed_bounded_artifact_is_admitted() {
    let (key, signed, archive, values, target) = fixture();
    let admitted = admit_artifact(
        Some(&signed),
        key.verifying_key().as_bytes(),
        &archive,
        &members(&values),
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect("admitted artifact");
    assert_eq!(admitted.manifest.manifest_id, "synthetic-exact-v1");
}

#[test]
fn signature_path_and_target_fail_closed() {
    let (key, mut signed, archive, mut values, mut target) = fixture();
    signed.signature[0] ^= 1;
    let error = admit_artifact(
        Some(&signed),
        key.verifying_key().as_bytes(),
        &archive,
        &members(&values),
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect_err("invalid signature");
    assert_eq!(error.reason(), ArtifactDecisionReason::ManifestSignatureInvalid);

    let (key, signed, archive, _, _) = fixture();
    values[0].0 = "../application.bin".into();
    let error = admit_artifact(
        Some(&signed),
        key.verifying_key().as_bytes(),
        &archive,
        &members(&values),
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect_err("unsafe path");
    assert_eq!(error.reason(), ArtifactDecisionReason::UnsafeArchive);

    let (key, signed, archive, values, _) = fixture();
    target.radio_variant = Some("sx1276".into());
    let error = admit_artifact(
        Some(&signed),
        key.verifying_key().as_bytes(),
        &archive,
        &members(&values),
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect_err("target mismatch");
    assert_eq!(error.reason(), ArtifactDecisionReason::TargetMismatch);
}

#[test]
fn archive_digest_members_expansion_and_layout_fail_closed() {
    let (key, signed, archive, values, target) = fixture();
    let error = admit_artifact(
        Some(&signed),
        key.verifying_key().as_bytes(),
        b"different archive",
        &members(&values),
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect_err("archive digest mismatch");
    assert_eq!(error.reason(), ArtifactDecisionReason::ArchiveDigestMismatch);

    let duplicate = vec![values[0].clone(), values[0].clone()];
    let error = admit_artifact(
        Some(&signed),
        key.verifying_key().as_bytes(),
        &archive,
        &members(&duplicate),
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect_err("duplicate member");
    assert_eq!(error.reason(), ArtifactDecisionReason::UnsafeArchive);

    let bounded = resign(&key, &signed, |manifest| manifest.artifact.max_expanded_bytes = 1);
    let error = admit_artifact(
        Some(&bounded),
        key.verifying_key().as_bytes(),
        &archive,
        &members(&values),
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect_err("expanded size");
    assert_eq!(error.reason(), ArtifactDecisionReason::UnsafeArchive);

    let overlap = resign(&key, &signed, |manifest| manifest.images[0].region.offset = 0x9000);
    let error = admit_artifact(
        Some(&overlap),
        key.verifying_key().as_bytes(),
        &archive,
        &members(&values),
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect_err("protected region overlap");
    assert_eq!(error.reason(), ArtifactDecisionReason::UnsafeLayout);
}

#[test]
fn unsigned_malformed_and_application_digest_cases_fail_closed() {
    let (key, signed, archive, values, target) = fixture();
    let error = admit_artifact(
        None,
        key.verifying_key().as_bytes(),
        &archive,
        &members(&values),
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect_err("signature required");
    assert_eq!(error.reason(), ArtifactDecisionReason::ManifestSignatureRequired);

    let payload = b"{}".to_vec();
    let malformed =
        SignedFirmwareManifest { signature: key.sign(&payload).to_bytes().to_vec(), payload };
    let error = admit_artifact(
        Some(&malformed),
        key.verifying_key().as_bytes(),
        &archive,
        &members(&values),
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect_err("manifest invalid");
    assert_eq!(error.reason(), ArtifactDecisionReason::ManifestInvalid);

    let no_application = resign(&key, &signed, |manifest| manifest.images[0].application = false);
    let error = admit_artifact(
        Some(&no_application),
        key.verifying_key().as_bytes(),
        &archive,
        &members(&values),
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect_err("application digest required");
    assert_eq!(error.reason(), ArtifactDecisionReason::ApplicationDigestRequired);
}
