use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

const CORPUS_ID: &str = "styrene-mobile-integration-v1";
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
    topologies: Vec<Topology>,
    required_areas: Vec<Area>,
    cases: Vec<Case>,
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
    Ffi,
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
