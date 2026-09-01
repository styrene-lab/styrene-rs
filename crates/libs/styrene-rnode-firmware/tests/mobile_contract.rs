use ed25519_dalek::{Signer, SigningKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use styrene_rnode_firmware::{
    ArchiveMember, CapabilityRequest, ConfigurationState, ExecutorClass, FirmwareManifest,
    FirmwareOperation, HostClass, ManifestArtifact, ManifestImage, ManifestRecovery,
    ManifestTarget, McuFamily, MemoryRegion, MobileDfuApply, MobileDfuEffect, MobileDfuError,
    MobileDfuWorkflow, PlanConfirmation, Sha256Digest, SignedFirmwareManifest, TargetObservation,
    admit_artifact,
};

const WORKFLOWS: &str =
    include_str!("../../../../tests/fixtures/rnode-firmware-provisioning-v1/workflows.json");
const GENERATION: u64 = 7;

#[derive(Debug, Deserialize)]
struct WorkflowCorpus {
    schema_version: u16,
    evidence_scope: String,
    cases: Vec<WorkflowCase>,
}

#[derive(Debug, Deserialize)]
struct WorkflowCase {
    id: String,
    host: String,
    operation: String,
    executor: String,
    actions: Vec<String>,
    destructive_started: bool,
    post_verified: bool,
    expected: WorkflowExpected,
}

#[derive(Debug, Deserialize)]
struct WorkflowExpected {
    terminal: String,
    recovery_required: bool,
}

fn digest(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

fn exact_target() -> TargetObservation {
    let application = b"synthetic mobile application";
    let mut target = TargetObservation::new(McuFamily::Nrf52840, ConfigurationState::Yes)
        .with_hardware(
            Some("synthetic_rak4631".into()),
            Some("synthetic_sx1262_915".into()),
            Some("synthetic-rev-a".into()),
            Some("uf2-0.4.3".into()),
        );
    target.generation = GENERATION;
    target.firmware_version = Some("1.86".into());
    target.running_application_hash = Some(digest(application));
    target.bootloader_revision = Some("0.4.3".into());
    target
}

fn mobile_workflow() -> MobileDfuWorkflow {
    MobileDfuWorkflow::new(
        confirmed_plan(),
        &CapabilityRequest {
            host: HostClass::IosMobile,
            operation: FirmwareOperation::Upgrade,
            executor: Some(ExecutorClass::IosNrfBleDfu),
            target: exact_target(),
            physical_acceptance: true,
        },
    )
    .expect("mobile workflow")
}

fn confirmed_plan() -> styrene_rnode_firmware::ConfirmedFirmwarePlan {
    let archive = b"synthetic mobile archive";
    let application = b"synthetic mobile application";
    let target = exact_target();
    let manifest = FirmwareManifest {
        schema_version: 1,
        manifest_id: "synthetic-rak4631-1.86".into(),
        firmware_version: "1.86".into(),
        operations: vec![FirmwareOperation::Upgrade],
        target: ManifestTarget {
            board: target.board.clone().expect("board"),
            radio_variant: target.radio_variant.clone().expect("radio"),
            hardware_revision: target.hardware_revision.clone().expect("revision"),
            executor: ExecutorClass::IosNrfBleDfu,
        },
        artifact: ManifestArtifact {
            archive_sha256: digest(archive),
            max_expanded_bytes: 1024,
            expected_members: vec!["application.bin".into()],
        },
        images: vec![ManifestImage {
            member: "application.bin".into(),
            region: MemoryRegion { offset: 0x26_000, length: application.len() as u64 },
            sha256: digest(application),
            application: true,
        }],
        protected_regions: vec![MemoryRegion { offset: 0, length: 0x26_000 }],
        recovery: ManifestRecovery {
            executor: ExecutorClass::HostSerialNrfDfu,
            procedure_id: "synthetic-desktop-nrf-recovery-v1".into(),
            physical_mode: "uf2-or-serial-dfu".into(),
            tool_id: "bounded_host_serial_nrf_dfu".into(),
            power_condition: "stable_usb_power".into(),
        },
    };
    let payload = serde_json::to_vec(&manifest).expect("manifest payload");
    let key = SigningKey::from_bytes(&[0x43; 32]);
    let signed =
        SignedFirmwareManifest { signature: key.sign(&payload).to_bytes().to_vec(), payload };
    let admitted = admit_artifact(
        Some(&signed),
        key.verifying_key().as_bytes(),
        archive,
        &[ArchiveMember { path: "application.bin", bytes: application }],
        &target,
        ExecutorClass::IosNrfBleDfu,
    )
    .expect("admitted synthetic artifact");
    let plan = admitted
        .dry_run_plan(FirmwareOperation::Upgrade, target.clone())
        .expect("mobile dry-run plan");
    plan.confirm(
        &PlanConfirmation {
            plan_digest: plan.digest().expect("plan digest"),
            target_generation: GENERATION,
        },
        &target,
    )
    .expect("confirmed mobile plan")
}

fn mobile_cases() -> Vec<WorkflowCase> {
    let corpus: WorkflowCorpus = serde_json::from_str(WORKFLOWS).expect("workflow corpus");
    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.evidence_scope, "synthetic_contract");
    corpus.cases.into_iter().filter(|case| case.host == "ios_mobile").collect()
}

#[test]
fn mobile_workflow_corpus_replays_against_confirmed_plan_policy() {
    let cases = mobile_cases();
    assert_eq!(cases.len(), 4);
    assert_eq!(
        cases.iter().map(|case| case.id.as_str()).collect::<Vec<_>>(),
        vec![
            "workflow.cancel-before-write",
            "workflow.ios-disconnect-during-write",
            "workflow.mobile-upgrade-verified",
            "workflow.stale-mobile-completion",
        ]
    );
    for case in cases {
        assert_eq!(case.operation, "upgrade");
        assert_eq!(case.executor, "ios_nrf_ble_dfu");
        assert_eq!(&case.actions[..3], ["inspect_nus", "create_plan", "confirm"]);
        let mut workflow = mobile_workflow();
        let expected_bytes = workflow.expected_bytes();
        let mut generation = GENERATION;
        for action in case.actions {
            match action.as_str() {
                "inspect_nus" | "create_plan" | "confirm" => {}
                "close_nus" => {
                    workflow.nus_closed(generation).unwrap();
                }
                "discover_dfu" => {
                    workflow.dfu_discovered(generation).unwrap();
                }
                "begin_write" => {
                    workflow.write_started(generation).unwrap();
                }
                "complete_write" => {
                    workflow.progress_changed(generation, expected_bytes).unwrap();
                    workflow.write_completed(generation).unwrap();
                }
                "reconnect_nus" => {
                    workflow.nus_reopened(generation).unwrap();
                }
                "verify_model_version_hash" => {
                    workflow.verify_reopened(generation, &exact_target()).unwrap();
                }
                "cancel" => {
                    workflow.cancel(generation).unwrap();
                }
                "disconnect" => {
                    workflow.interrupted(generation).unwrap();
                }
                "replace_generation" => {
                    generation += 1;
                    workflow.replace_generation(generation).unwrap();
                }
                "receive_stale_completion" => {
                    assert_eq!(
                        workflow.write_completed(GENERATION).unwrap(),
                        MobileDfuApply::IgnoredStale
                    );
                }
                other => panic!("unhandled mobile corpus action {other}"),
            }
        }
        assert_eq!(workflow.terminal_name(), case.expected.terminal, "case {}", case.id);
        assert_eq!(
            workflow.recovery_required(),
            case.expected.recovery_required,
            "case {}",
            case.id
        );
        assert_eq!(workflow.destructive_started(), case.destructive_started, "case {}", case.id);
        assert_eq!(workflow.terminal_name() == "succeeded", case.post_verified, "case {}", case.id);
    }
}

#[test]
fn mobile_workflow_enforces_nus_progress_cancellation_and_verification_boundaries() {
    let mut workflow = mobile_workflow();
    assert_eq!(workflow.required_effect(), Some(MobileDfuEffect::CloseNus));
    assert_eq!(workflow.dfu_discovered(GENERATION), Err(MobileDfuError::InvalidPhase));
    workflow.nus_closed(GENERATION).unwrap();
    workflow.dfu_discovered(GENERATION).unwrap();
    workflow.write_started(GENERATION).unwrap();
    assert_eq!(workflow.cancel(GENERATION), Err(MobileDfuError::WriteAlreadyStarted));
    assert_eq!(
        workflow.progress_changed(GENERATION, workflow.expected_bytes() + 1),
        Err(MobileDfuError::InvalidProgress)
    );
    workflow.progress_changed(GENERATION, workflow.expected_bytes()).unwrap();
    workflow.write_completed(GENERATION).unwrap();
    assert_eq!(
        workflow.verify_reopened(GENERATION, &exact_target()),
        Err(MobileDfuError::InvalidPhase)
    );
    workflow.nus_reopened(GENERATION).unwrap();
    let mut mismatch = exact_target();
    mismatch.running_application_hash = Some(digest(b"wrong application"));
    assert!(matches!(
        workflow.verify_reopened(GENERATION, &mismatch),
        Err(MobileDfuError::Verification(_))
    ));
    assert_eq!(workflow.terminal_name(), "verification_failed");
    assert!(workflow.recovery_required());
}

#[test]
fn mobile_workflow_requires_acceptance_and_generation_change_invalidates_confirmation() {
    let denied = CapabilityRequest {
        host: HostClass::IosMobile,
        operation: FirmwareOperation::Upgrade,
        executor: Some(ExecutorClass::IosNrfBleDfu),
        target: exact_target(),
        physical_acceptance: false,
    };
    assert!(matches!(
        MobileDfuWorkflow::new(confirmed_plan(), &denied),
        Err(MobileDfuError::CapabilityDenied)
    ));

    let mut workflow = mobile_workflow();
    workflow.nus_closed(GENERATION).unwrap();
    workflow.dfu_discovered(GENERATION).unwrap();
    workflow.write_started(GENERATION).unwrap();
    workflow.replace_generation(GENERATION + 1).unwrap();
    assert_eq!(workflow.terminal_name(), "failed");
    assert!(workflow.recovery_required());
    assert_eq!(workflow.progress_changed(GENERATION + 1, 1), Err(MobileDfuError::InvalidProgress));
}
