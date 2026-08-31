use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use styrene_rnode_firmware::{
    ArchiveMember, CapabilityDecision, CapabilityReason, CapabilityRequest, ConfigurationState,
    ExecutorClass, FirmwareEvent, FirmwareManifest, FirmwareOperation, FirmwareWorkflow, HostClass,
    ManifestArtifact, ManifestImage, ManifestRecovery, ManifestTarget, McuFamily, MemoryRegion,
    PlanConfirmation, Sha256Digest, SignedFirmwareManifest, TargetObservation, admit_artifact,
};

fn digest(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

fn exact_target() -> TargetObservation {
    let mut target = TargetObservation::new(McuFamily::Esp32s3, ConfigurationState::Yes)
        .with_hardware(
            Some("synthetic_exact_board".into()),
            Some("synthetic_sx1262".into()),
            Some("synthetic-rev-a".into()),
            Some("esp_rom_serial".into()),
        );
    target.generation = 7;
    target
}

fn admitted_artifact() -> styrene_rnode_firmware::AdmittedArtifact {
    let archive = b"synthetic desktop archive";
    let application = b"synthetic application";
    let target = exact_target();
    let manifest = FirmwareManifest {
        schema_version: 1,
        manifest_id: "synthetic-desktop-1.87".into(),
        firmware_version: "1.87".into(),
        operations: vec![FirmwareOperation::Upgrade, FirmwareOperation::Recovery],
        target: ManifestTarget {
            board: target.board.clone().expect("board"),
            radio_variant: target.radio_variant.clone().expect("radio"),
            hardware_revision: target.hardware_revision.clone().expect("revision"),
            executor: ExecutorClass::HostSerialEsp,
        },
        artifact: ManifestArtifact {
            archive_sha256: digest(archive),
            max_expanded_bytes: 1024,
            expected_members: vec!["application.bin".into()],
        },
        images: vec![ManifestImage {
            member: "application.bin".into(),
            region: MemoryRegion { offset: 0x1_0000, length: application.len() as u64 },
            sha256: digest(application),
            application: true,
        }],
        protected_regions: vec![MemoryRegion { offset: 0x9000, length: 0x1000 }],
        recovery: ManifestRecovery {
            executor: ExecutorClass::HostSerialEsp,
            procedure_id: "synthetic-esp-rom-recovery-v1".into(),
            physical_mode: "rom_serial_bootloader".into(),
            tool_id: "bounded_host_serial_esp".into(),
            power_condition: "stable_usb_power".into(),
        },
    };
    let payload = serde_json::to_vec(&manifest).expect("manifest payload");
    let key = SigningKey::from_bytes(&[0x42; 32]);
    let signed =
        SignedFirmwareManifest { signature: key.sign(&payload).to_bytes().to_vec(), payload };
    admit_artifact(
        Some(&signed),
        key.verifying_key().as_bytes(),
        archive,
        &[ArchiveMember { path: "application.bin", bytes: application }],
        &target,
        ExecutorClass::HostSerialEsp,
    )
    .expect("admitted synthetic artifact")
}

#[test]
fn unknown_desktop_target_can_only_be_inspected_read_only() {
    let target = TargetObservation::new(McuFamily::Esp32s3, ConfigurationState::Unknown)
        .with_hardware(None, None, None, Some("esp_rom_serial".into()));
    let inspect = CapabilityRequest {
        host: HostClass::Desktop,
        operation: FirmwareOperation::Inspect,
        executor: Some(ExecutorClass::ReadOnlySerial),
        target: target.clone(),
        physical_acceptance: false,
    }
    .evaluate();
    assert_eq!(inspect.decision, CapabilityDecision::Allow);
    assert_eq!(inspect.reason, CapabilityReason::ReadOnlyInspection);

    let destructive_inspection = CapabilityRequest {
        executor: Some(ExecutorClass::HostSerialEsp),
        ..CapabilityRequest {
            host: HostClass::Desktop,
            operation: FirmwareOperation::Inspect,
            executor: None,
            target: target.clone(),
            physical_acceptance: false,
        }
    }
    .evaluate();
    assert_eq!(destructive_inspection.decision, CapabilityDecision::Deny);
    assert_eq!(destructive_inspection.reason, CapabilityReason::ExecutorMismatch);

    let plan = CapabilityRequest {
        host: HostClass::Desktop,
        operation: FirmwareOperation::Plan,
        executor: None,
        target,
        physical_acceptance: false,
    }
    .evaluate();
    assert_eq!(plan.reason, CapabilityReason::ExactTargetUnknown);
}

#[test]
fn desktop_dry_run_is_derived_from_the_admitted_manifest() {
    let admitted = admitted_artifact();
    let target = exact_target();
    let plan = admitted
        .dry_run_plan(FirmwareOperation::Upgrade, target.clone())
        .expect("desktop dry-run plan");

    assert_eq!(plan.target, target);
    assert_eq!(plan.executor, ExecutorClass::HostSerialEsp);
    assert_eq!(plan.artifact.manifest_entry, "synthetic-desktop-1.87");
    assert_eq!(plan.artifact.archive_sha256, admitted.manifest.artifact.archive_sha256);
    assert_eq!(plan.artifact.firmware_version, "1.87");
    assert_eq!(plan.image_regions.len(), 1);
    assert_eq!(plan.image_regions[0].region.offset, 0x1_0000);
    assert_eq!(plan.preserved_regions, admitted.manifest.protected_regions);
    assert_eq!(plan.expected.running_application_hash, plan.artifact.application_sha256);
    assert!(plan.validate().is_ok());
}

#[test]
fn desktop_confirmation_rejects_a_fabricated_unsafe_plan() {
    let target = exact_target();
    let mut plan = admitted_artifact()
        .dry_run_plan(FirmwareOperation::Upgrade, target.clone())
        .expect("desktop dry-run plan");
    plan.image_regions[0].region = MemoryRegion { offset: 0x9800, length: 0x1000 };
    let confirmation = PlanConfirmation {
        plan_digest: plan.digest().expect("unsafe plan digest"),
        target_generation: target.generation,
    };
    assert!(plan.confirm(&confirmation, &target).is_err());
}

#[test]
fn desktop_plan_uses_half_open_regions_and_rejects_overflow() {
    let target = exact_target();
    let mut plan = admitted_artifact()
        .dry_run_plan(FirmwareOperation::Upgrade, target)
        .expect("desktop dry-run plan");
    let image_end = plan.image_regions[0]
        .region
        .offset
        .checked_add(plan.image_regions[0].region.length)
        .expect("bounded fixture region");
    plan.preserved_regions = vec![MemoryRegion { offset: image_end, length: 1 }];
    assert!(plan.validate().is_ok());

    plan.image_regions[0].region = MemoryRegion { offset: u64::MAX, length: 1 };
    assert!(plan.validate().is_err());
}

#[test]
fn desktop_failure_requires_a_separate_confirmed_recovery_plan() {
    let target = exact_target();
    let admitted = admitted_artifact();
    let upgrade =
        admitted.dry_run_plan(FirmwareOperation::Upgrade, target.clone()).expect("upgrade plan");
    let recovery =
        admitted.dry_run_plan(FirmwareOperation::Recovery, target.clone()).expect("recovery plan");
    assert_ne!(
        upgrade.digest().expect("upgrade digest"),
        recovery.digest().expect("recovery digest")
    );
    assert!(recovery.recovery.requires_new_confirmation);
    assert_eq!(recovery.recovery.physical_mode, "rom_serial_bootloader");
    assert_eq!(recovery.recovery.tool_id, "bounded_host_serial_esp");
    assert_eq!(recovery.recovery.power_condition, "stable_usb_power");

    let mut workflow = FirmwareWorkflow::new(
        HostClass::Desktop,
        FirmwareOperation::Upgrade,
        ExecutorClass::HostSerialEsp,
        target.generation,
    );
    workflow.apply(FirmwareEvent::WriteStarted).expect("write started");
    workflow.apply(FirmwareEvent::Interrupted).expect("write interrupted");
    assert_eq!(workflow.terminal_name(), "failed");
    assert!(workflow.recovery_required());
    assert_eq!(workflow.operation(), FirmwareOperation::Upgrade);

    let old_confirmation = PlanConfirmation {
        plan_digest: upgrade.digest().expect("upgrade digest"),
        target_generation: target.generation,
    };
    assert!(recovery.confirm(&old_confirmation, &target).is_err());
    let recovery_confirmation = PlanConfirmation {
        plan_digest: recovery.digest().expect("recovery digest"),
        target_generation: target.generation,
    };
    assert!(recovery.confirm(&recovery_confirmation, &target).is_ok());
}
