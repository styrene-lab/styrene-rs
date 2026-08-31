use serde::Deserialize;
use styrene_rnode_firmware::{
    ArchiveDigestMatch, ArchiveFinding, ArtifactAdmissionFacts, ArtifactDecision,
    ArtifactDecisionReason, CapabilityDecision, CapabilityReason, CapabilityRequest,
    ConfigurationState, ExecutorClass, FirmwareEvent, FirmwareOperation, FirmwareWorkflow,
    HostClass, LayoutFinding, ManifestSignatureState, McuFamily, TargetMatch, TargetObservation,
};

const CAPABILITIES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../tests/fixtures/rnode-firmware-provisioning-v1/capabilities.json"
));
const ARTIFACTS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../tests/fixtures/rnode-firmware-provisioning-v1/artifacts.json"
));
const WORKFLOWS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../tests/fixtures/rnode-firmware-provisioning-v1/workflows.json"
));

#[derive(Deserialize)]
struct CapabilityCorpus {
    targets: Vec<CapabilityTarget>,
    cases: Vec<CapabilityCase>,
}

#[derive(Deserialize)]
struct CapabilityTarget {
    id: String,
    mcu_family: McuFamily,
    board: Option<String>,
    radio_variant: Option<String>,
    hardware_revision: Option<String>,
    bootloader: String,
    configured: ConfigurationState,
    physical_acceptance: bool,
}

#[derive(Deserialize)]
struct CapabilityCase {
    id: String,
    host: HostClass,
    target: String,
    operation: FirmwareOperation,
    executor: Option<ExecutorClass>,
    expected: CapabilityExpected,
}

#[derive(Deserialize)]
struct CapabilityExpected {
    decision: CapabilityDecision,
    reason: CapabilityReason,
}

#[derive(Deserialize)]
struct ArtifactCorpus {
    cases: Vec<ArtifactCase>,
}

#[derive(Deserialize)]
struct ArtifactCase {
    id: String,
    manifest_signature: ManifestSignatureState,
    archive_digest: ArchiveDigestMatch,
    target_match: TargetMatch,
    archive_findings: Vec<ArchiveFinding>,
    layout_findings: Vec<LayoutFinding>,
    expected: ArtifactExpected,
}

#[derive(Deserialize)]
struct ArtifactExpected {
    decision: ArtifactDecision,
    reason: ArtifactDecisionReason,
}

#[derive(Deserialize)]
struct WorkflowCorpus {
    cases: Vec<WorkflowCase>,
}

#[derive(Deserialize)]
struct WorkflowCase {
    id: String,
    host: HostClass,
    operation: FirmwareOperation,
    executor: ExecutorClass,
    actions: Vec<String>,
    expected: WorkflowExpected,
}

#[derive(Deserialize)]
struct WorkflowExpected {
    terminal: String,
    recovery_required: bool,
}

#[test]
fn capability_corpus_matches_policy() {
    let corpus: CapabilityCorpus = serde_json::from_str(CAPABILITIES).expect("capability corpus");
    for case in corpus.cases {
        let target = corpus
            .targets
            .iter()
            .find(|target| target.id == case.target)
            .unwrap_or_else(|| panic!("{} references missing target", case.id));
        let observation = TargetObservation::new(target.mcu_family, target.configured)
            .with_hardware(
                target.board.clone(),
                target.radio_variant.clone(),
                target.hardware_revision.clone(),
                Some(target.bootloader.clone()),
            );
        let request = CapabilityRequest {
            host: case.host,
            operation: case.operation,
            executor: case.executor,
            target: observation,
            physical_acceptance: target.physical_acceptance,
        };
        let actual = request.evaluate();
        assert_eq!(actual.decision, case.expected.decision, "{} decision", case.id);
        assert_eq!(actual.reason, case.expected.reason, "{} reason", case.id);
    }
}

#[test]
fn artifact_corpus_matches_admission_policy() {
    let corpus: ArtifactCorpus = serde_json::from_str(ARTIFACTS).expect("artifact corpus");
    for case in corpus.cases {
        let facts = ArtifactAdmissionFacts {
            manifest_signature: case.manifest_signature,
            archive_digest: case.archive_digest,
            target_match: case.target_match,
            archive_findings: case.archive_findings,
            layout_findings: case.layout_findings,
        };
        let actual = facts.evaluate();
        assert_eq!(actual.decision, case.expected.decision, "{} decision", case.id);
        assert_eq!(actual.reason, case.expected.reason, "{} reason", case.id);
    }
}

#[test]
fn workflow_corpus_matches_state_machine() {
    let corpus: WorkflowCorpus = serde_json::from_str(WORKFLOWS).expect("workflow corpus");
    for case in corpus.cases {
        let mut workflow = FirmwareWorkflow::new(case.host, case.operation, case.executor, 7);
        for action in &case.actions {
            workflow
                .apply(event(action))
                .unwrap_or_else(|error| panic!("{} action {action} failed: {error}", case.id));
        }
        assert_eq!(workflow.terminal_name(), case.expected.terminal, "{} terminal", case.id);
        assert_eq!(
            workflow.recovery_required(),
            case.expected.recovery_required,
            "{} recovery",
            case.id
        );
    }
}

fn event(action: &str) -> FirmwareEvent {
    match action {
        "inspect" | "inspect_nus" | "inspect_bootloader" => FirmwareEvent::Inspected,
        "admit_artifact" => FirmwareEvent::ArtifactAdmitted,
        "create_plan" | "create_recovery_plan" => FirmwareEvent::PlanCreated,
        "confirm" => FirmwareEvent::Confirmed,
        "confirm_other_digest" | "missing_confirmation" => FirmwareEvent::ConfirmationRejected,
        "replace_target" => FirmwareEvent::TargetChanged,
        "close_nus" | "discover_dfu" => FirmwareEvent::Preparing,
        "begin_write" => FirmwareEvent::WriteStarted,
        "complete_write" => FirmwareEvent::WriteCompleted,
        "reopen" | "reconnect_nus" => FirmwareEvent::Reopened,
        "verify_model_version_hash" => FirmwareEvent::Verified,
        "observe_hash_mismatch" => FirmwareEvent::VerificationFailed,
        "cancel" => FirmwareEvent::Cancelled,
        "disconnect" | "power_loss" => FirmwareEvent::Interrupted,
        "replace_generation" => FirmwareEvent::GenerationReplaced(8),
        "receive_stale_completion" => FirmwareEvent::StaleEvent(7),
        "omit_provisioning" => FirmwareEvent::ProvisioningIncomplete,
        "observe_failed_upgrade" => FirmwareEvent::RecoveryRequired,
        other => panic!("unknown corpus workflow action {other}"),
    }
}
