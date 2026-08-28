use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

const CORPUS_ID: &str = "styrene-mobile-integration-v1";
const CROSS_PLATFORM_LANE_ID: &str = "mobile.messaging.cross-platform-ios-to-android";
const MAX_LAUNCH_PROFILE_BYTES: usize = 64;
const MAX_SCENARIO_DEADLINE_SECONDS: u32 = 3_600;
const REQUIRED_ARTIFACT_CLASSES: &[ArtifactClass] = &[
    ArtifactClass::RunManifest,
    ArtifactClass::Milestones,
    ArtifactClass::BoundedLogs,
    ArtifactClass::SemanticUiSnapshots,
    ArtifactClass::IosTestResults,
];
const REQUIRED_CASES: &[&str] = &[
    "mobile.identity.offline-create",
    "mobile.lifecycle.explicit-shutdown",
    "mobile.messaging.direct-bidirectional",
    "mobile.messaging.dual-simulator-roundtrip",
    "mobile.delivery.lifecycle-and-receipt",
    "mobile.delivery.route-bearer-separation",
    "mobile.attachments.small-inline",
    "mobile.people.discovery-not-connectivity",
    "mobile.network.rnode-lora-roundtrip",
    "mobile.propagation.queue-sync-state",
    "mobile.pages.structured-session",
    "mobile.settings.capability-availability",
    "mobile.diagnostics.redacted-bounded-export",
    "mobile.persistence.forced-termination",
    "mobile.accessibility.shared-semantics",
    "mobile.parity.navigation-and-terminology",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    corpus: String,
    description: String,
    execution_lanes: Vec<ExecutionLane>,
    topologies: Vec<Topology>,
    required_areas: Vec<Area>,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionLane {
    id: String,
    case: String,
    actions: Vec<String>,
    boundaries: Vec<ExecutionBoundary>,
    launch_profiles: Vec<LaunchProfile>,
    scenario_deadline_seconds: u32,
    deadlines: Vec<Deadline>,
    cleanup: Vec<Cleanup>,
    artifacts: ArtifactPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchProfile {
    boundary: ExecutionBoundary,
    profile: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Deadline {
    name: String,
    seconds: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cleanup {
    resource: CleanupResource,
    ownership: CleanupOwnership,
    always_run: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactPolicy {
    root: String,
    commit: CommitPolicy,
    required_classes: Vec<ArtifactClass>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Topology {
    id: String,
    kind: Kind,
    nodes: u32,
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    area: Area,
    priority: Priority,
    title: String,
    maturity: Maturity,
    kind: Kind,
    evidence_scope: EvidenceScope,
    topology: String,
    platforms: Vec<Platform>,
    ui_surfaces: Vec<String>,
    actions: Vec<String>,
    assertions: Vec<Assertion>,
    existing_tests: Vec<String>,
    missing_capabilities: Vec<String>,
    not_proven: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Assertion {
    source: AssertionSource,
    claim: String,
    oracle: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Area {
    Identity,
    Lifecycle,
    Messaging,
    Delivery,
    Attachments,
    People,
    Network,
    Propagation,
    Pages,
    Settings,
    Diagnostics,
    Persistence,
    Accessibility,
    Parity,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Priority {
    P0,
    P1,
    P2,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Maturity {
    Executable,
    Partial,
    Blocked,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Offline,
    Loopback,
    Simulator,
    Device,
    Hardware,
    LiveInterop,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum EvidenceScope {
    InternalFixture,
    InternalRuntime,
    HostSimulator,
    HostDevice,
    HardwareObservation,
    UpstreamInterop,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Platform {
    Rust,
    IosSimulator,
    IosDevice,
    AndroidJvm,
    AndroidInstrumentation,
    AndroidDevice,
    RnodeHardware,
    PythonUpstream,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum AssertionSource {
    Daemon,
    DurableStore,
    HostState,
    Platform,
    Hardware,
    Accessibility,
    Upstream,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ExecutionBoundary {
    LocalHost,
    IosSimulator,
    AndroidEmulator,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
enum CleanupResource {
    IosApp,
    AndroidApp,
    Hub,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum CleanupOwnership {
    RunnerOwned,
    RunnerStartedOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum CommitPolicy {
    Forbidden,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ArtifactClass {
    RunManifest,
    Milestones,
    BoundedLogs,
    SemanticUiSnapshots,
    IosTestResults,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn corpus_path() -> PathBuf {
    workspace_root().join("tests/fixtures/mobile-integration-v1/corpus.json")
}

fn read_corpus() -> Corpus {
    let path = corpus_path();
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn is_stable_id(value: &str, separator: char) -> bool {
    !value.is_empty()
        && value.split(separator).all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

fn is_launch_profile(value: &str) -> bool {
    value.len() <= MAX_LAUNCH_PROFILE_BYTES
        && value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_execution_lanes(corpus: &Corpus) -> Result<(), String> {
    if corpus.execution_lanes.is_empty() {
        return Err("at least one execution lane is required".to_owned());
    }

    let cases: HashMap<_, _> = corpus.cases.iter().map(|case| (case.id.as_str(), case)).collect();
    let mut lane_ids = HashSet::new();
    for lane in &corpus.execution_lanes {
        if !is_stable_id(&lane.id, '.') {
            return Err(format!("invalid execution lane ID: {}", lane.id));
        }
        if !lane_ids.insert(lane.id.as_str()) {
            return Err(format!("duplicate execution lane: {}", lane.id));
        }
        let case = cases
            .get(lane.case.as_str())
            .ok_or_else(|| format!("{}: unknown case {}", lane.id, lane.case))?;
        if lane.actions.is_empty() {
            return Err(format!("{}: at least one case action is required", lane.id));
        }
        let mut actions = HashSet::new();
        for action in &lane.actions {
            if !actions.insert(action.as_str()) {
                return Err(format!("{}: duplicate action {action}", lane.id));
            }
            if !case.actions.contains(action) {
                return Err(format!(
                    "{}: action {action} is not declared by {}",
                    lane.id, lane.case
                ));
            }
        }

        let boundaries: HashSet<_> = lane.boundaries.iter().copied().collect();
        if boundaries.len() != lane.boundaries.len() {
            return Err(format!("{}: execution boundaries must be unique", lane.id));
        }
        let required_boundaries = HashSet::from([
            ExecutionBoundary::LocalHost,
            ExecutionBoundary::IosSimulator,
            ExecutionBoundary::AndroidEmulator,
        ]);
        if boundaries != required_boundaries {
            return Err(format!(
                "{}: cross-platform lane must explicitly name local_host, ios_simulator, and android_emulator boundaries",
                lane.id
            ));
        }

        let mut profile_boundaries = HashSet::new();
        let mut profiles = HashSet::new();
        for profile in &lane.launch_profiles {
            if !matches!(
                profile.boundary,
                ExecutionBoundary::IosSimulator | ExecutionBoundary::AndroidEmulator
            ) {
                return Err(format!(
                    "{}: launch profile cannot target {:?}",
                    lane.id, profile.boundary
                ));
            }
            if !profile_boundaries.insert(profile.boundary) {
                return Err(format!(
                    "{}: duplicate launch profile boundary {:?}",
                    lane.id, profile.boundary
                ));
            }
            if !is_launch_profile(&profile.profile) {
                return Err(format!(
                    "{}: invalid launch profile {}; expected 1-{MAX_LAUNCH_PROFILE_BYTES} ASCII bytes, alphanumeric first, then alphanumeric, '.', '_', or '-'",
                    lane.id, profile.profile
                ));
            }
            if !profiles.insert(profile.profile.as_str()) {
                return Err(format!("{}: launch profiles must be distinct", lane.id));
            }
        }
        let required_profile_boundaries =
            HashSet::from([ExecutionBoundary::IosSimulator, ExecutionBoundary::AndroidEmulator]);
        if profile_boundaries != required_profile_boundaries {
            return Err(format!(
                "{}: each mobile execution boundary requires one launch profile",
                lane.id
            ));
        }

        if lane.scenario_deadline_seconds == 0
            || lane.scenario_deadline_seconds > MAX_SCENARIO_DEADLINE_SECONDS
        {
            return Err(format!(
                "{}: scenario deadline must be between 1 and {MAX_SCENARIO_DEADLINE_SECONDS} seconds",
                lane.id
            ));
        }
        if lane.deadlines.is_empty() {
            return Err(format!("{}: at least one named deadline is required", lane.id));
        }
        let mut deadline_names = HashSet::new();
        for deadline in &lane.deadlines {
            if !is_stable_id(&deadline.name, '-') {
                return Err(format!("{}: invalid deadline name {}", lane.id, deadline.name));
            }
            if !deadline_names.insert(deadline.name.as_str()) {
                return Err(format!("{}: duplicate deadline {}", lane.id, deadline.name));
            }
            if deadline.seconds == 0 || deadline.seconds > lane.scenario_deadline_seconds {
                return Err(format!(
                    "{}: deadline {} must be positive and no greater than the scenario deadline",
                    lane.id, deadline.name
                ));
            }
        }

        let expected_cleanup = HashMap::from([
            (CleanupResource::IosApp, CleanupOwnership::RunnerOwned),
            (CleanupResource::AndroidApp, CleanupOwnership::RunnerOwned),
            (CleanupResource::Hub, CleanupOwnership::RunnerStartedOnly),
        ]);
        let mut cleanup_resources = HashSet::new();
        for cleanup in &lane.cleanup {
            if !cleanup_resources.insert(cleanup.resource) {
                return Err(format!(
                    "{}: duplicate cleanup resource {:?}",
                    lane.id, cleanup.resource
                ));
            }
            if !cleanup.always_run {
                return Err(format!(
                    "{}: cleanup for {:?} must always run",
                    lane.id, cleanup.resource
                ));
            }
            if expected_cleanup.get(&cleanup.resource) != Some(&cleanup.ownership) {
                return Err(format!(
                    "{}: cleanup ownership is invalid for {:?}",
                    lane.id, cleanup.resource
                ));
            }
        }
        if cleanup_resources != expected_cleanup.keys().copied().collect() {
            return Err(format!(
                "{}: cleanup must cover both apps and the conditionally owned hub",
                lane.id
            ));
        }

        if lane.artifacts.root != "target/mobile-integration" {
            return Err(format!("{}: artifact root must be target/mobile-integration", lane.id));
        }
        if lane.artifacts.commit != CommitPolicy::Forbidden {
            return Err(format!("{}: committing artifacts must be forbidden", lane.id));
        }
        let artifact_classes: HashSet<_> =
            lane.artifacts.required_classes.iter().copied().collect();
        if artifact_classes.len() != lane.artifacts.required_classes.len() {
            return Err(format!("{}: required artifact classes must be unique", lane.id));
        }
        if artifact_classes != REQUIRED_ARTIFACT_CLASSES.iter().copied().collect() {
            return Err(format!("{}: required artifact classes are incomplete", lane.id));
        }
    }

    if !lane_ids.contains(CROSS_PLATFORM_LANE_ID) {
        return Err(format!("required execution lane is missing: {CROSS_PLATFORM_LANE_ID}"));
    }
    Ok(())
}

fn reference_path(reference: &str) -> &str {
    reference.split_once('#').map_or(reference, |(path, _)| path)
}

fn assert_repository_reference(root: &Path, case_id: &str, reference: &str) {
    let relative = Path::new(reference_path(reference));
    assert!(!relative.is_absolute(), "{case_id}: reference must be relative: {reference}");
    assert!(
        !relative.components().any(|component| component == Component::ParentDir),
        "{case_id}: reference must not traverse parents: {reference}"
    );
    assert!(root.join(relative).is_file(), "{case_id}: reference does not exist: {reference}");
}

#[test]
fn mobile_integration_corpus_is_complete_and_well_formed() {
    let corpus = read_corpus();
    let root = workspace_root();

    assert_eq!(corpus.schema_version, 1, "unsupported mobile corpus schema");
    assert_eq!(corpus.corpus, CORPUS_ID, "unexpected mobile corpus ID");
    assert!(!corpus.description.trim().is_empty(), "corpus description is required");
    assert!(!corpus.topologies.is_empty(), "at least one topology is required");
    assert!(!corpus.cases.is_empty(), "at least one case is required");
    validate_execution_lanes(&corpus)
        .unwrap_or_else(|error| panic!("invalid mobile execution lane: {error}"));

    let mut topology_ids = HashSet::new();
    let mut topology_kinds = HashMap::new();
    for topology in &corpus.topologies {
        assert!(is_stable_id(&topology.id, '-'), "invalid topology ID: {}", topology.id);
        assert!(topology_ids.insert(topology.id.as_str()), "duplicate topology: {}", topology.id);
        assert!(topology.nodes > 0, "{}: topology must contain at least one node", topology.id);
        assert!(
            !topology.description.trim().is_empty(),
            "{}: topology description is required",
            topology.id
        );
        topology_kinds.insert(topology.id.as_str(), topology.kind);
    }

    let required_areas: HashSet<_> = corpus.required_areas.iter().copied().collect();
    assert_eq!(required_areas.len(), corpus.required_areas.len(), "required areas must be unique");

    let mut case_ids = HashSet::new();
    let mut covered_areas = HashSet::new();
    let mut priorities = HashMap::<Priority, usize>::new();
    let mut maturities = HashMap::<Maturity, usize>::new();

    for case in &corpus.cases {
        assert!(is_stable_id(&case.id, '.'), "invalid case ID: {}", case.id);
        assert!(case_ids.insert(case.id.as_str()), "duplicate case ID: {}", case.id);
        assert!(!case.title.trim().is_empty(), "{}: title is required", case.id);
        assert!(
            topology_ids.contains(case.topology.as_str()),
            "{}: unknown topology {}",
            case.id,
            case.topology
        );
        assert!(!case.platforms.is_empty(), "{}: at least one platform is required", case.id);
        assert!(!case.ui_surfaces.is_empty(), "{}: at least one UI surface is required", case.id);
        assert!(!case.actions.is_empty(), "{}: at least one action is required", case.id);
        assert!(!case.assertions.is_empty(), "{}: at least one assertion is required", case.id);

        for surface in &case.ui_surfaces {
            assert!(is_stable_id(surface, '.'), "{}: invalid UI surface {surface}", case.id);
        }
        for action in &case.actions {
            assert!(is_stable_id(action, '-'), "{}: invalid action {action}", case.id);
        }
        for assertion in &case.assertions {
            assert!(!assertion.claim.trim().is_empty(), "{}: assertion claim is required", case.id);
            assert_repository_reference(&root, &case.id, &assertion.oracle);
            let _ = assertion.source;
        }
        for test in &case.existing_tests {
            assert_repository_reference(&root, &case.id, test);
        }
        for capability in &case.missing_capabilities {
            assert!(!capability.trim().is_empty(), "{}: empty missing capability", case.id);
        }
        for limitation in &case.not_proven {
            assert!(!limitation.trim().is_empty(), "{}: empty not-proven statement", case.id);
        }

        match case.maturity {
            Maturity::Executable => {
                assert!(
                    case.missing_capabilities.is_empty(),
                    "{}: executable case has missing capabilities",
                    case.id
                );
                assert!(
                    !case.existing_tests.is_empty(),
                    "{}: executable case must name an existing test",
                    case.id
                );
            }
            Maturity::Partial | Maturity::Blocked => {
                assert!(
                    !case.missing_capabilities.is_empty(),
                    "{}: incomplete case must name missing capabilities",
                    case.id
                );
            }
        }

        if case.kind == Kind::LiveInterop {
            assert_eq!(
                case.evidence_scope,
                EvidenceScope::UpstreamInterop,
                "{}: live interop requires upstream evidence scope",
                case.id
            );
            assert!(
                case.platforms.contains(&Platform::PythonUpstream),
                "{}: live interop must name the upstream platform",
                case.id
            );
        }
        if case.evidence_scope == EvidenceScope::InternalFixture {
            assert!(
                !case.not_proven.is_empty(),
                "{}: fixture evidence must state what it does not prove",
                case.id
            );
        }
        covered_areas.insert(case.area);
        *priorities.entry(case.priority).or_default() += 1;
        *maturities.entry(case.maturity).or_default() += 1;
        let _ = topology_kinds.get(case.topology.as_str());
    }

    assert_eq!(covered_areas, required_areas, "required area coverage does not match corpus cases");
    for required in REQUIRED_CASES {
        assert!(case_ids.contains(required), "required mobile case is missing: {required}");
    }
    for priority in [Priority::P0, Priority::P1] {
        assert!(
            priorities.get(&priority).copied().unwrap_or(0) > 0,
            "priority {priority:?} has no cases"
        );
    }
    for maturity in [Maturity::Executable, Maturity::Partial, Maturity::Blocked] {
        assert!(
            maturities.get(&maturity).copied().unwrap_or(0) > 0,
            "maturity {maturity:?} has no cases"
        );
    }
}

#[cfg(test)]
mod execution_lane_tests {
    use super::*;
    use serde_json::Value;

    fn corpus_value() -> Value {
        serde_json::from_slice(&std::fs::read(corpus_path()).expect("read corpus fixture"))
            .expect("parse corpus fixture as JSON")
    }

    fn validate_mutation(mutate: impl FnOnce(&mut Value)) -> String {
        let mut value = corpus_value();
        mutate(&mut value);
        let corpus: Corpus =
            serde_json::from_value(value).expect("mutation must preserve the Serde schema");
        validate_execution_lanes(&corpus).expect_err("mutated execution lane must be rejected")
    }

    #[test]
    fn closed_execution_lane_schema_rejects_unknown_fields() {
        let mut value = corpus_value();
        value["execution_lanes"][0]["runner_evidence"] = Value::Bool(true);
        let error =
            serde_json::from_value::<Corpus>(value).expect_err("unknown lane fields must fail");
        assert!(error.to_string().contains("unknown field `runner_evidence`"));
    }

    #[test]
    fn execution_lane_rejects_dangling_case_actions_and_duplicate_profiles() {
        let case_error = validate_mutation(|value| {
            value["execution_lanes"][0]["case"] =
                Value::String("mobile.messaging.missing".to_owned());
        });
        assert!(case_error.contains("unknown case"));

        let action_error = validate_mutation(|value| {
            value["execution_lanes"][0]["actions"][0] = Value::String("missing-action".to_owned());
        });
        assert!(action_error.contains("is not declared"));

        let profile_error = validate_mutation(|value| {
            let ios_profile = value["execution_lanes"][0]["launch_profiles"][0]["profile"].clone();
            value["execution_lanes"][0]["launch_profiles"][1]["profile"] = ios_profile;
        });
        assert!(profile_error.contains("must be distinct"));

        let grammar_error = validate_mutation(|value| {
            value["execution_lanes"][0]["launch_profiles"][0]["profile"] =
                Value::String("_invalid".to_owned());
        });
        assert!(grammar_error.contains("invalid launch profile"));
    }

    #[test]
    fn execution_lane_rejects_unbounded_deadlines_and_incomplete_cleanup() {
        let deadline_error = validate_mutation(|value| {
            value["execution_lanes"][0]["deadlines"][0]["seconds"] = Value::from(0);
        });
        assert!(deadline_error.contains("must be positive"));

        let cleanup_error = validate_mutation(|value| {
            value["execution_lanes"][0]["cleanup"][2]["always_run"] = Value::Bool(false);
        });
        assert!(cleanup_error.contains("must always run"));

        let ownership_error = validate_mutation(|value| {
            value["execution_lanes"][0]["cleanup"][2]["ownership"] =
                Value::String("runner_owned".to_owned());
        });
        assert!(ownership_error.contains("cleanup ownership is invalid"));
    }

    #[test]
    fn execution_lane_rejects_artifacts_outside_policy() {
        let root_error = validate_mutation(|value| {
            value["execution_lanes"][0]["artifacts"]["root"] =
                Value::String("tmp/results".to_owned());
        });
        assert!(root_error.contains("artifact root"));

        let class_error = validate_mutation(|value| {
            value["execution_lanes"][0]["artifacts"]["required_classes"]
                .as_array_mut()
                .expect("artifact classes array")
                .pop();
        });
        assert!(class_error.contains("artifact classes are incomplete"));
    }
}
