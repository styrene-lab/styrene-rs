use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

const CORPUS_ID: &str = "styrene-mobile-application-parity-v1";
const REQUIRED_ROWS: &[&str] = &[
    "mobile.journey.identity",
    "mobile.journey.tcp-setup",
    "mobile.journey.discovery",
    "mobile.journey.conversations",
    "mobile.journey.drafts",
    "mobile.journey.direct-send",
    "mobile.journey.receipts",
    "mobile.journey.retry",
    "mobile.journey.restart",
    "mobile.journey.propagation",
    "mobile.journey.degraded-state",
];
const REQUIRED_REFERENCES: &[&str] = &[
    "sideband-2.1.0-build-20251128",
    "reticulum-meshchat-2.4.0",
    "skywave-1.0-build-5",
    "python-rns-1.4.2",
    "python-lxmf-795fdaa2",
    "nomadnet-1.2.8",
    "columba-candidate",
    "meshtastic-interaction",
    "meshcore-interaction",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    corpus: String,
    description: String,
    collected_on: String,
    references: Vec<Reference>,
    evidence: Vec<Evidence>,
    parity_rows: Vec<ParityRow>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    id: String,
    classification: Classification,
    name: String,
    version: Option<String>,
    build: Option<String>,
    platforms: Vec<String>,
    protocol_versions: ProtocolVersions,
    provenance: Provenance,
    limitations: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtocolVersions {
    rns: Option<String>,
    lxmf: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Provenance {
    status: ProvenanceStatus,
    kind: ProvenanceKind,
    locator: Option<String>,
    revision: Option<String>,
    artifact_sha256: Option<String>,
    notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    id: String,
    reference_id: String,
    method: EvidenceMethod,
    status: EvidenceStatus,
    collected_on: String,
    platform: Option<String>,
    os: Option<String>,
    citations: Vec<String>,
    artifacts: Vec<Artifact>,
    supported_journeys: Vec<String>,
    facts: Vec<String>,
    limitations: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    kind: ArtifactKind,
    locator: String,
    sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParityRow {
    id: String,
    integration_cases: Vec<String>,
    spec_references: Vec<String>,
    platforms: Vec<MobilePlatform>,
    floor_evidence_id: Option<String>,
    candidate_evidence_ids: Vec<String>,
    observed_facts: Vec<String>,
    styrene_requirement: String,
    observable_styrene_outcome: String,
    proposed_differences: Vec<String>,
    exclusions: Vec<String>,
    status: ParityStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Classification {
    ProtocolAuthority,
    ObservedRnsLxmfApplication,
    CandidateRnsLxmfApplication,
    InteractionOnlyReference,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ProvenanceStatus {
    Current,
    Unresolved,
    Conflicted,
    Stale,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ProvenanceKind {
    SourceRepository,
    Binary,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum EvidenceMethod {
    StaticArtifactInspection,
    StaticSourceInspection,
    RetainedExecution,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum EvidenceStatus {
    Executed,
    Unevidenced,
    Stale,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ArtifactKind {
    ObservationLog,
    SemanticSnapshot,
    TestResult,
    SourceManifest,
    BinaryManifest,
    ObservationSummary,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum MobilePlatform {
    Ios,
    Android,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ParityStatus {
    Matched,
    IntentionallyDifferent,
    Deferred,
    Unsupported,
    Unevidenced,
}

#[derive(Deserialize)]
struct IntegrationCorpus {
    cases: Vec<IntegrationCase>,
}

#[derive(Deserialize)]
struct IntegrationCase {
    id: String,
    priority: String,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn corpus_path() -> PathBuf {
    workspace_root().join("tests/fixtures/mobile-application-parity-v1/corpus.json")
}

fn read_corpus() -> Corpus {
    let path = corpus_path();
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn nonblank(value: &str) -> bool {
    !value.trim().is_empty()
}

fn is_date(value: &str) -> bool {
    value.len() == 10
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7) { byte == b'-' } else { byte.is_ascii_digit() }
        })
}

fn is_stable_id(value: &str) -> bool {
    nonblank(value)
        && value.split(['.', '-']).all(|segment| {
            !segment.is_empty()
                && segment.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn is_token(value: &str) -> bool {
    nonblank(value)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_revision(value: &str) -> bool {
    value.len() == 40
        && value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_citation(value: &str) -> bool {
    if value.starts_with("https://") {
        return true;
    }
    let Some((digest, member)) =
        value.strip_prefix("sha256:").and_then(|value| value.split_once("!/"))
    else {
        return false;
    };
    is_sha256(digest) && nonblank(member)
}

fn assert_nonblank_list(owner: &str, field: &str, values: &[String]) -> Result<(), String> {
    if values.is_empty() || values.iter().any(|value| !nonblank(value)) {
        return Err(format!("{owner}: {field} must contain nonblank values"));
    }
    Ok(())
}

fn assert_repository_path(root: &Path, owner: &str, value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute() || path.components().any(|component| component == Component::ParentDir) {
        return Err(format!("{owner}: repository path must be relative: {value}"));
    }
    if !root.join(path).is_file() {
        return Err(format!("{owner}: repository path does not exist: {value}"));
    }
    Ok(())
}

fn validate(corpus: &Corpus) -> Result<(), String> {
    if corpus.schema_version != 1 || corpus.corpus != CORPUS_ID {
        return Err("unsupported application parity corpus identity".into());
    }
    if !nonblank(&corpus.description) || !is_date(&corpus.collected_on) {
        return Err("description and collection date are required".into());
    }

    let mut reference_ids = HashSet::new();
    let mut references = HashMap::new();
    for reference in &corpus.references {
        if !is_stable_id(&reference.id) || !reference_ids.insert(reference.id.as_str()) {
            return Err(format!("invalid or duplicate reference ID: {}", reference.id));
        }
        if !nonblank(&reference.name) {
            return Err(format!("{}: reference name is required", reference.id));
        }
        assert_nonblank_list(&reference.id, "limitations", &reference.limitations)?;
        if reference.platforms.iter().any(|value| !is_token(value)) {
            return Err(format!("{}: invalid platform", reference.id));
        }
        for value in [
            reference.version.as_deref(),
            reference.build.as_deref(),
            reference.protocol_versions.rns.as_deref(),
            reference.protocol_versions.lxmf.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if !nonblank(value) {
                return Err(format!("{}: empty version or build", reference.id));
            }
        }
        assert_nonblank_list(&reference.id, "provenance notes", &reference.provenance.notes)?;
        if let Some(locator) = &reference.provenance.locator
            && !locator.starts_with("https://")
        {
            return Err(format!("{}: provenance locator must use HTTPS", reference.id));
        }
        if let Some(revision) = &reference.provenance.revision
            && !is_revision(revision)
        {
            return Err(format!("{}: provenance revision must be a full commit", reference.id));
        }
        if let Some(digest) = &reference.provenance.artifact_sha256
            && !is_sha256(digest)
        {
            return Err(format!("{}: invalid artifact SHA-256", reference.id));
        }
        if reference.provenance.status == ProvenanceStatus::Current {
            if reference.provenance.locator.is_none() {
                return Err(format!("{}: current provenance requires a locator", reference.id));
            }
            match reference.provenance.kind {
                ProvenanceKind::SourceRepository if reference.provenance.revision.is_none() => {
                    return Err(format!(
                        "{}: current source provenance requires a revision",
                        reference.id
                    ));
                }
                ProvenanceKind::Binary if reference.provenance.artifact_sha256.is_none() => {
                    return Err(format!(
                        "{}: current binary provenance requires a digest",
                        reference.id
                    ));
                }
                _ => {}
            }
        }
        if reference.provenance.status == ProvenanceStatus::Conflicted
            && reference.version.is_some()
        {
            return Err(format!("{}: conflicted provenance cannot select a version", reference.id));
        }
        references.insert(reference.id.as_str(), reference);
    }
    for required in REQUIRED_REFERENCES {
        if !reference_ids.contains(required) {
            return Err(format!("required reference is missing: {required}"));
        }
    }

    let required_rows: HashSet<_> = REQUIRED_ROWS.iter().copied().collect();
    let mut evidence_ids = HashSet::new();
    let mut evidence = HashMap::new();
    for item in &corpus.evidence {
        if !is_stable_id(&item.id) || !evidence_ids.insert(item.id.as_str()) {
            return Err(format!("invalid or duplicate evidence ID: {}", item.id));
        }
        let reference = references
            .get(item.reference_id.as_str())
            .ok_or_else(|| format!("{}: unknown reference {}", item.id, item.reference_id))?;
        if !is_date(&item.collected_on) || item.platform.as_deref().is_some_and(|v| !is_token(v)) {
            return Err(format!("{}: invalid date or platform", item.id));
        }
        if item.os.as_deref().is_some_and(|value| !nonblank(value)) {
            return Err(format!("{}: empty OS value", item.id));
        }
        assert_nonblank_list(&item.id, "facts", &item.facts)?;
        assert_nonblank_list(&item.id, "limitations", &item.limitations)?;
        if item.artifacts.is_empty()
            || item.citations.is_empty()
            || item.citations.iter().any(|citation| !is_citation(citation))
            || item.supported_journeys.is_empty()
        {
            return Err(format!(
                "{}: artifacts, immutable citations, and supported journeys are required",
                item.id
            ));
        }
        for artifact in &item.artifacts {
            if !artifact.locator.starts_with("https://") {
                return Err(format!("{}: artifact locator must use HTTPS", item.id));
            }
            if artifact.sha256.as_deref().is_some_and(|digest| !is_sha256(digest)) {
                return Err(format!("{}: invalid artifact digest", item.id));
            }
            let _ = artifact.kind;
        }
        for journey in &item.supported_journeys {
            if !required_rows.contains(journey.as_str()) {
                return Err(format!("{}: unknown journey {journey}", item.id));
            }
        }
        if item.status == EvidenceStatus::Executed
            && (item.method != EvidenceMethod::RetainedExecution
                || item.os.is_none()
                || reference.provenance.status != ProvenanceStatus::Current
                || reference.classification != Classification::ObservedRnsLxmfApplication
                || !item.artifacts.iter().any(|artifact| {
                    matches!(
                        artifact.kind,
                        ArtifactKind::ObservationLog
                            | ArtifactKind::SemanticSnapshot
                            | ArtifactKind::TestResult
                    )
                }))
        {
            return Err(format!("{}: executed evidence is not admissible", item.id));
        }
        evidence.insert(item.id.as_str(), item);
    }

    let integration_path =
        workspace_root().join("tests/fixtures/mobile-integration-v1/corpus.json");
    let integration: IntegrationCorpus = serde_json::from_slice(
        &std::fs::read(&integration_path)
            .map_err(|error| format!("failed to read {}: {error}", integration_path.display()))?,
    )
    .map_err(|error| format!("failed to parse {}: {error}", integration_path.display()))?;
    let integration_cases: HashMap<_, _> =
        integration.cases.iter().map(|case| (case.id.as_str(), case.priority.as_str())).collect();

    let root = workspace_root();
    let mut row_ids = HashSet::new();
    for row in &corpus.parity_rows {
        if !required_rows.contains(row.id.as_str()) || !row_ids.insert(row.id.as_str()) {
            return Err(format!("invalid or duplicate parity row: {}", row.id));
        }
        if row.platforms != [MobilePlatform::Ios, MobilePlatform::Android] {
            return Err(format!("{}: platforms must be ios and android", row.id));
        }
        if row.spec_references.is_empty() {
            return Err(format!("{}: at least one spec reference is required", row.id));
        }
        for spec in &row.spec_references {
            assert_repository_path(&root, &row.id, spec)?;
        }
        for case in &row.integration_cases {
            let priority = integration_cases
                .get(case.as_str())
                .ok_or_else(|| format!("{}: unknown integration case {case}", row.id))?;
            if *priority != "p0" {
                return Err(format!("{}: integration case {case} is not P0", row.id));
            }
        }
        for candidate in &row.candidate_evidence_ids {
            if !evidence.contains_key(candidate.as_str()) {
                return Err(format!("{}: unknown candidate evidence {candidate}", row.id));
            }
        }
        for (field, values) in [
            ("candidate evidence", &row.candidate_evidence_ids),
            ("proposed differences", &row.proposed_differences),
            ("exclusions", &row.exclusions),
        ] {
            assert_nonblank_list(&row.id, field, values)?;
        }
        if !nonblank(&row.styrene_requirement) || !nonblank(&row.observable_styrene_outcome) {
            return Err(format!("{}: requirement and outcome are required", row.id));
        }
        if row.observed_facts.iter().any(|fact| !nonblank(fact)) {
            return Err(format!("{}: observed facts cannot contain blanks", row.id));
        }
        match row.status {
            ParityStatus::Matched | ParityStatus::IntentionallyDifferent => {
                let floor_id = row
                    .floor_evidence_id
                    .as_deref()
                    .ok_or_else(|| format!("{}: completed row requires a floor", row.id))?;
                let floor = evidence
                    .get(floor_id)
                    .ok_or_else(|| format!("{}: unknown floor evidence {floor_id}", row.id))?;
                if floor.status != EvidenceStatus::Executed
                    || !floor.supported_journeys.contains(&row.id)
                {
                    return Err(format!(
                        "{}: floor evidence is not executed for this journey",
                        row.id
                    ));
                }
                if row.observed_facts.is_empty() {
                    return Err(format!("{}: completed row requires observed facts", row.id));
                }
                if row.status == ParityStatus::Matched && !row.proposed_differences.is_empty() {
                    return Err(format!("{}: matched row cannot contain differences", row.id));
                }
            }
            ParityStatus::Deferred | ParityStatus::Unsupported | ParityStatus::Unevidenced => {
                if row.floor_evidence_id.is_some() {
                    return Err(format!("{}: incomplete row cannot select a floor", row.id));
                }
            }
        }
    }
    if row_ids != required_rows {
        return Err("parity rows must exactly cover the required P0 journeys".into());
    }
    Ok(())
}

#[test]
fn mobile_application_parity_corpus_is_complete_and_non_vacuous() {
    validate(&read_corpus()).unwrap_or_else(|error| panic!("invalid parity corpus: {error}"));
}

#[cfg(test)]
mod mutation_tests {
    use super::*;
    use serde_json::Value;

    fn corpus_value() -> Value {
        serde_json::from_slice(&std::fs::read(corpus_path()).expect("read parity corpus"))
            .expect("parse parity corpus")
    }

    fn validation_error(mutate: impl FnOnce(&mut Value)) -> String {
        let mut value = corpus_value();
        mutate(&mut value);
        let corpus: Corpus =
            serde_json::from_value(value).expect("mutation must preserve the Serde schema");
        validate(&corpus).expect_err("mutation must be rejected")
    }

    #[test]
    fn closed_schema_rejects_unknown_fields() {
        let mut value = corpus_value();
        value["evidence"][0]["runtime_observed"] = Value::Bool(true);
        let error = serde_json::from_value::<Corpus>(value).expect_err("unknown field must fail");
        assert!(error.to_string().contains("unknown field `runtime_observed`"));
    }

    #[test]
    fn missing_p0_row_and_dangling_candidate_are_rejected() {
        let missing = validation_error(|value| {
            value["parity_rows"].as_array_mut().expect("rows").pop();
        });
        assert!(missing.contains("exactly cover"));

        let dangling = validation_error(|value| {
            value["parity_rows"][0]["candidate_evidence_ids"][0] =
                Value::String("missing-evidence".into());
        });
        assert!(dangling.contains("unknown candidate evidence"));
    }

    #[test]
    fn static_candidate_cannot_be_promoted_to_floor() {
        let error = validation_error(|value| {
            value["parity_rows"][0]["status"] = Value::String("intentionally_different".into());
            value["parity_rows"][0]["floor_evidence_id"] =
                Value::String("sideband-2.1.0-static-artifact-inspection".into());
            value["parity_rows"][0]["observed_facts"] =
                Value::Array(vec![Value::String("fabricated observation".into())]);
        });
        assert!(error.contains("floor evidence is not executed"));
    }

    #[test]
    fn current_source_requires_full_revision_and_conflict_cannot_pick_version() {
        let revision = validation_error(|value| {
            value["references"][1]["provenance"]["revision"] = Value::String("45f89a8".into());
        });
        assert!(revision.contains("full commit"));

        let conflict = validation_error(|value| {
            value["references"][4]["version"] = Value::String("1.1.1".into());
        });
        assert!(conflict.contains("cannot select a version"));
    }

    #[test]
    fn unknown_integration_case_and_parent_traversal_are_rejected() {
        let case = validation_error(|value| {
            value["parity_rows"][0]["integration_cases"][0] =
                Value::String("mobile.identity.missing".into());
        });
        assert!(case.contains("unknown integration case"));

        let path = validation_error(|value| {
            value["parity_rows"][0]["spec_references"][0] = Value::String("../outside.md".into());
        });
        assert!(path.contains("must be relative"));
    }
}
