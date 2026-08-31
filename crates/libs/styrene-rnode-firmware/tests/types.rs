use styrene_rnode_firmware::{
    ArtifactIdentity, ConfigurationState, EvidenceScope, ExecutorClass, ExpectedDeviceState,
    FirmwareEvent, FirmwareEvidence, FirmwareOperation, FirmwarePhase, FirmwarePlan,
    FirmwareProgress, FirmwareWorkflow, HostClass, ImageRegion, McuFamily, MemoryRegion,
    RecoveryPolicy, Sha256Digest, TargetObservation,
};

fn digest(value: char) -> Sha256Digest {
    Sha256Digest::new(value.to_string().repeat(64)).expect("valid test digest")
}

fn plan() -> FirmwarePlan {
    let target = TargetObservation {
        generation: 4,
        platform_code: Some(0x80),
        mcu_code: Some(0x81),
        board_code: Some(0x3a),
        product_code: Some(0x03),
        model_code: Some(0xa6),
        hardware_revision_code: Some(0x01),
        mcu_family: McuFamily::Esp32s3,
        board: Some("exact-board".into()),
        radio_variant: Some("sx1262".into()),
        hardware_revision: Some("rev-a".into()),
        bootloader: Some("esp_rom_serial".into()),
        bootloader_revision: Some("rom-v1".into()),
        configuration: ConfigurationState::Yes,
        firmware_version: Some("1.86".into()),
        running_application_hash: Some(digest('a')),
        target_application_hash: Some(digest('b')),
    };
    FirmwarePlan {
        schema_version: 1,
        operation: FirmwareOperation::Upgrade,
        target_generation: target.generation,
        target,
        artifact: ArtifactIdentity {
            manifest_entry: "exact-board-1.87".into(),
            archive_sha256: digest('c'),
            application_sha256: digest('b'),
            firmware_version: "1.87".into(),
        },
        executor: ExecutorClass::HostSerialEsp,
        image_regions: vec![ImageRegion {
            name: "application".into(),
            region: MemoryRegion { offset: 65_536, length: 131_072 },
            sha256: digest('b'),
        }],
        preserved_regions: vec![MemoryRegion { offset: 9_000, length: 4_000 }],
        recovery: RecoveryPolicy {
            executor: ExecutorClass::HostSerialEsp,
            procedure_id: "esp-rom-recovery-v1".into(),
            requires_new_confirmation: true,
        },
        expected: ExpectedDeviceState {
            board: "exact-board".into(),
            radio_variant: "sx1262".into(),
            hardware_revision: "rev-a".into(),
            firmware_version: "1.87".into(),
            running_application_hash: digest('b'),
        },
    }
}

#[test]
fn plan_digest_is_stable_and_covers_target_generation() {
    let plan = plan();
    let first = plan.digest().expect("digest plan");
    assert_eq!(first, plan.digest().expect("digest plan again"));

    let mut changed = plan;
    changed.target_generation += 1;
    assert_ne!(first, changed.digest().expect("digest changed plan"));
}

#[test]
fn progress_rejects_completed_bytes_above_total() {
    assert!(FirmwareProgress::new(FirmwarePhase::Writing, 11, 10).is_err());
    assert!(FirmwareProgress::new(FirmwarePhase::Writing, 10, 10).is_ok());
}

#[test]
fn evidence_retains_provenance_without_device_identity_fields() {
    let evidence = FirmwareEvidence {
        scope: EvidenceScope::PhysicalHardware,
        application_revision: "app-commit".into(),
        manifest_revision: "manifest-commit".into(),
        upstream_revision: "upstream-commit".into(),
        artifact_sha256: digest('d'),
        executor_version: "executor-v1".into(),
        target_class: "exact-board/rev-a/sx1262".into(),
        bootloader_revision: "rom-v1".into(),
        final_application_hash: digest('b'),
    };
    let encoded = serde_json::to_value(evidence).expect("serialize evidence");
    assert_eq!(encoded["scope"], "physical_hardware");
    assert!(encoded.get("serial_number").is_none());
    assert!(encoded.get("peripheral_id").is_none());
}

#[test]
fn verification_requires_a_completed_write() {
    let mut workflow = FirmwareWorkflow::new(
        HostClass::Desktop,
        FirmwareOperation::Upgrade,
        ExecutorClass::HostSerialEsp,
        4,
    );
    workflow.apply(FirmwareEvent::WriteStarted).expect("start write");
    workflow.apply(FirmwareEvent::Reopened).expect("reopen target");
    assert!(workflow.apply(FirmwareEvent::Verified).is_err());
    assert_ne!(workflow.terminal_name(), "succeeded");
}
