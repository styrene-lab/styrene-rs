use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

const CORPUS_ID: &str = "styrene-mobile-custody-handoff-v1";
const BACKEND_CONTRACT_REVISION: &str = "899da81302c5f4e92f60a2fdaf396c26e813ba76";
const FRONTEND_HANDOFF_REVISION: &str = "a98c5c42db89818f2206bb8498c1c3632d638fdc";
const REQUIRED_HOST_ORCHESTRATION: &[&str] = &[
    "approved_clean_reset",
    "install_baseline_package",
    "launch_baseline_package",
    "restart_application",
    "force_terminate_application",
    "relaunch_after_forced_termination",
    "install_upgrade_in_place",
    "relaunch_after_upgrade",
];
const REQUIRED_LIFECYCLE: &[&str] = &[
    "approved_clean_reset",
    "first_launch",
    "graceful_restart",
    "forced_termination",
    "relaunch",
    "in_place_upgrade",
    "post_upgrade_relaunch",
];
const REQUIRED_ARTIFACTS: &[&str] = &[
    "application_sha256",
    "backend_revision",
    "custody_snapshot",
    "frontend_revision",
    "host_attestation",
    "os_version",
    "package_versions",
    "public_identity_comparison",
    "screenshots",
    "test_result",
];
const PROHIBITED_METADATA: &[&str] = &[
    "device_identifier",
    "development_team_identifier",
    "hardware_udid",
    "private_identity_material",
    "provisioning_profile",
    "signing_identity",
];
const REQUIRED_ASSERTIONS: &[&str] =
    &["custody_matches_expected", "identity_stable", "upgrade_preserves_identity"];
const REQUIRED_FORBIDDEN_OUTCOMES: &[&str] =
    &["false_secure_claim", "identity_rotation", "plaintext_fallback", "secret_artifact"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    corpus: String,
    registered: bool,
    claim_status: ClaimStatus,
    authority: Authority,
    reset_requires_explicit_approval: bool,
    prohibited_metadata: Vec<String>,
    scenarios: Vec<Scenario>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ClaimStatus {
    Unevidenced,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Authority {
    backend_contract_revision: String,
    integration_corpus: String,
    integration_case: String,
    openspec_task: String,
    frontend_repository: String,
    frontend_handoff_revision: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    id: String,
    host: String,
    platform: Platform,
    execution_state: ExecutionState,
    runner: Option<Runner>,
    host_orchestration: Vec<String>,
    lifecycle: Vec<String>,
    expected_custody: ExpectedCustody,
    assertion_ids: Vec<String>,
    forbidden_outcome_ids: Vec<String>,
    required_artifacts: Vec<String>,
    blocker_ids: Vec<String>,
    evidence: Vec<String>,
    limits: Limits,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Platform {
    IosDevice,
    AndroidDevice,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ExecutionState {
    HandoffOnly,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Runner {
    path: String,
    tests: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedCustody {
    requested_backend: String,
    active_backend: String,
    protection: String,
    authentication: String,
    availability: String,
    downgrade: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Limits {
    timeout_secs: u32,
    max_artifacts: u32,
    max_artifact_bytes: u64,
}

#[derive(Deserialize)]
struct IntegrationCorpus {
    cases: Vec<IntegrationCase>,
}

#[derive(Deserialize)]
struct IntegrationCase {
    id: String,
    priority: String,
    kind: String,
    platforms: Vec<String>,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn corpus_path() -> PathBuf {
    workspace_root().join("tests/interop/handoffs/mobile-custody-v1.json")
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn read_corpus() -> Corpus {
    read_json(&corpus_path()).unwrap_or_else(|error| panic!("{error}"))
}

fn exact_set(values: &[String], expected: &[&str]) -> bool {
    values.len() == expected.len()
        && values.iter().map(String::as_str).collect::<HashSet<_>>()
            == expected.iter().copied().collect::<HashSet<_>>()
}

fn repository_path(root: &Path, value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute()
        || value.contains('\\')
        || path.components().any(|component| !matches!(component, Component::Normal(_)))
        || !root.join(path).exists()
    {
        return Err(format!("invalid repository path: {value}"));
    }
    Ok(())
}

fn validate(corpus: &Corpus) -> Result<(), String> {
    if corpus.schema_version != 1
        || corpus.corpus != CORPUS_ID
        || corpus.registered
        || corpus.claim_status != ClaimStatus::Unevidenced
        || !corpus.reset_requires_explicit_approval
    {
        return Err("custody handoff must remain unregistered and unevidenced".into());
    }
    if corpus.authority.backend_contract_revision != BACKEND_CONTRACT_REVISION
        || corpus.authority.frontend_handoff_revision != FRONTEND_HANDOFF_REVISION
        || corpus.authority.frontend_repository != "https://github.com/styrene-lab/styrene-ui.git"
        || corpus.authority.integration_case != "mobile.identity.device-custody"
        || corpus.authority.openspec_task
            != "openspec/archive/2026-09-02-complete-mobile-p0-backend-contracts/tasks.md#3.5"
    {
        return Err("invalid custody handoff authority".into());
    }

    let root = workspace_root();
    repository_path(&root, &corpus.authority.integration_corpus)?;
    repository_path(&root, corpus.authority.openspec_task.split_once('#').unwrap().0)?;
    let integration: IntegrationCorpus =
        read_json(&root.join(&corpus.authority.integration_corpus))?;
    let case = integration
        .cases
        .iter()
        .find(|case| case.id == corpus.authority.integration_case)
        .ok_or("custody integration case is absent")?;
    if case.priority != "p0"
        || case.kind != "device"
        || !case.platforms.iter().any(|value| value == "ios_device")
        || !case.platforms.iter().any(|value| value == "android_device")
    {
        return Err("custody integration authority is not a physical P0 case".into());
    }
    if !exact_set(&corpus.prohibited_metadata, PROHIBITED_METADATA) {
        return Err("custody handoff prohibited metadata is incomplete".into());
    }
    if corpus.scenarios.len() != 2 {
        return Err("custody handoff requires exactly iOS and Android scenarios".into());
    }

    let mut scenario_ids = HashSet::new();
    for scenario in &corpus.scenarios {
        if !scenario_ids.insert(scenario.id.as_str())
            || scenario.execution_state != ExecutionState::HandoffOnly
            || !exact_set(&scenario.host_orchestration, REQUIRED_HOST_ORCHESTRATION)
            || !exact_set(&scenario.lifecycle, REQUIRED_LIFECYCLE)
            || !exact_set(&scenario.required_artifacts, REQUIRED_ARTIFACTS)
            || !exact_set(&scenario.assertion_ids, REQUIRED_ASSERTIONS)
            || !exact_set(&scenario.forbidden_outcome_ids, REQUIRED_FORBIDDEN_OUTCOMES)
            || scenario.blocker_ids.is_empty()
            || !scenario.evidence.is_empty()
            || scenario.limits.timeout_secs == 0
            || scenario.limits.timeout_secs > 600
            || scenario.limits.max_artifacts == 0
            || scenario.limits.max_artifacts > 32
            || scenario.limits.max_artifact_bytes == 0
            || scenario.limits.max_artifact_bytes > 16 * 1024 * 1024
        {
            return Err(format!("invalid or falsely evidenced scenario: {}", scenario.id));
        }
        if scenario.expected_custody.protection != "platform_protected"
            || scenario.expected_custody.availability != "available"
            || scenario.expected_custody.downgrade != "none"
        {
            return Err(format!("invalid expected custody state: {}", scenario.id));
        }

        match scenario.platform {
            Platform::IosDevice => {
                let runner = scenario
                    .runner
                    .as_ref()
                    .ok_or("iOS custody handoff requires the tracked XCUITest runner")?;
                if scenario.id != "mobile-custody-ios-device"
                    || scenario.host != "Chriss-MacBook-Pro"
                    || scenario.expected_custody.requested_backend != "keychain"
                    || scenario.expected_custody.active_backend != "keychain"
                    || scenario.expected_custody.authentication != "device_authentication"
                    || runner.path != "tests/xcui/StyreneMobileUITests.swift"
                    || !exact_set(
                        &scenario.blocker_ids,
                        &["first_unlock_policy_packaged", "physical_ios_execution"],
                    )
                    || !exact_set(
                        &runner.tests,
                        &[
                            "testPhysicalIdentityCustodySurvivesTerminationAndRestart",
                            "testPhysicalRestoredIdentityCustody",
                        ],
                    )
                {
                    return Err("invalid iOS custody handoff".into());
                }
            }
            Platform::AndroidDevice => {
                if scenario.id != "mobile-custody-android-device"
                    || scenario.host != "nucleus"
                    || scenario.expected_custody.requested_backend != "android_keystore"
                    || scenario.expected_custody.active_backend != "android_keystore"
                    || scenario.expected_custody.authentication != "none"
                    || scenario.runner.is_some()
                    || !exact_set(
                        &scenario.blocker_ids,
                        &["android_packaged_runner", "physical_android_execution"],
                    )
                {
                    return Err("invalid Android custody handoff".into());
                }
            }
        }
    }
    if scenario_ids != HashSet::from(["mobile-custody-ios-device", "mobile-custody-android-device"])
    {
        return Err("custody handoff scenario IDs are incomplete".into());
    }
    Ok(())
}

#[test]
fn physical_mobile_custody_handoff_is_closed_and_unevidenced() {
    validate(&read_corpus()).unwrap_or_else(|error| panic!("invalid custody handoff: {error}"));
}

#[cfg(test)]
mod mutation_tests {
    use super::*;
    use serde_json::Value;

    fn corpus_value() -> Value {
        serde_json::from_slice(&std::fs::read(corpus_path()).expect("read custody handoff"))
            .expect("parse custody handoff")
    }

    fn validation_error(mutate: impl FnOnce(&mut Value)) -> String {
        let mut value = corpus_value();
        mutate(&mut value);
        let corpus: Corpus =
            serde_json::from_value(value).expect("mutation must preserve the closed schema");
        validate(&corpus).expect_err("mutation must be rejected")
    }

    #[test]
    fn closed_schema_rejects_pass_claims() {
        let mut value = corpus_value();
        value["passed"] = Value::Bool(true);
        let error = serde_json::from_value::<Corpus>(value).expect_err("unknown pass field");
        assert!(error.to_string().contains("unknown field `passed`"));

        let error = validation_error(|value| {
            value["scenarios"][0]["evidence"] =
                Value::Array(vec![Value::String("local-result.xcresult".into())]);
        });
        assert!(error.contains("falsely evidenced"));
    }

    #[test]
    fn wrong_host_or_missing_lifecycle_stage_is_rejected() {
        let host = validation_error(|value| {
            value["scenarios"][0]["host"] = Value::String("nucleus".into());
        });
        assert!(host.contains("invalid iOS custody handoff"));

        let lifecycle = validation_error(|value| {
            value["scenarios"][1]["lifecycle"].as_array_mut().expect("lifecycle array").pop();
        });
        assert!(lifecycle.contains("falsely evidenced"));
    }

    #[test]
    fn android_requires_a_blocker_until_a_runner_exists() {
        let error = validation_error(|value| {
            value["scenarios"][1]["blocker_ids"] =
                Value::Array(vec![Value::String("physical_android_execution".into())]);
        });
        assert!(error.contains("invalid Android custody handoff"));
    }

    #[test]
    fn revisions_and_prohibited_metadata_are_non_vacuous() {
        let revision = validation_error(|value| {
            value["authority"]["frontend_handoff_revision"] =
                Value::String("0000000000000000000000000000000000000000".into());
        });
        assert!(revision.contains("invalid custody handoff authority"));

        let metadata = validation_error(|value| {
            value["prohibited_metadata"].as_array_mut().expect("metadata array").pop();
        });
        assert!(metadata.contains("prohibited metadata is incomplete"));
    }

    #[test]
    fn arbitrary_scenario_text_is_rejected() {
        let mut value = corpus_value();
        value["scenarios"][0]["notes"] =
            Value::String("local paths and device metadata do not belong here".into());
        let error = serde_json::from_value::<Corpus>(value)
            .expect_err("closed scenario schema must reject arbitrary text");
        assert!(error.to_string().contains("unknown field `notes`"));
    }
}
