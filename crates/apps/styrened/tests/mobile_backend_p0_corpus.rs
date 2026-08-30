use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

const CORPUS_ID: &str = "styrene-mobile-backend-p0-v1";
const REQUIRED_ROWS: &[&str] = &[
    "mobile.backend.p0.propagation.persist-before-ack",
    "mobile.backend.p0.messaging.unicode-preview",
    "mobile.backend.p0.identity.custody-fail-closed",
    "mobile.backend.p0.identity.custody-status",
    "mobile.backend.p0.identity.durable-edit",
    "mobile.backend.p0.runtime.offline-ready",
    "mobile.backend.p0.runtime.typed-boot-retry",
    "mobile.backend.p0.network.interface-observations",
    "mobile.backend.p0.settings.capability-generation",
    "mobile.backend.p0.people.conversation-create",
    "mobile.backend.p0.people.alias-projection",
    "mobile.backend.p0.delivery.route-bearer-correlation",
    "mobile.backend.p0.diagnostics.redacted-export",
    "mobile.backend.p0.persistence.forced-termination",
    "mobile.backend.p0.messaging.canonical-retry",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    corpus: String,
    description: String,
    created_on: String,
    authority: Authority,
    evidence_boundary: String,
    rows: Vec<Row>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Authority {
    backend_base_revision: String,
    integration_corpus: String,
    application_parity_corpus: String,
    openspec_change: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    id: String,
    domain: Domain,
    integration_cases: Vec<String>,
    parity_journeys: Vec<String>,
    current_contract_state: ContractState,
    delivery_state: DeliveryState,
    summary: String,
    owner_paths: Vec<String>,
    assertions: Vec<String>,
    forbidden_outcomes: Vec<String>,
    required_tests: Vec<String>,
    limits: Limits,
    frontend_handoff: FrontendHandoff,
    evidence: Vec<String>,
    exclusions: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Domain {
    Runtime,
    Identity,
    Messaging,
    People,
    Network,
    Delivery,
    Propagation,
    Settings,
    Diagnostics,
    Persistence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ContractState {
    Available,
    Partial,
    Defective,
    Missing,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum DeliveryState {
    Planned,
    Implementing,
    Verified,
    Blocked,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Limits {
    max_seconds: u32,
    max_items: u32,
    max_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrontendHandoff {
    ready: bool,
    contracts: Vec<String>,
    blocked_on: Vec<String>,
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

#[derive(Deserialize)]
struct ApplicationParityCorpus {
    parity_rows: Vec<ApplicationParityRow>,
}

#[derive(Deserialize)]
struct ApplicationParityRow {
    id: String,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn corpus_path() -> PathBuf {
    workspace_root().join("tests/fixtures/mobile-backend-p0-v1/corpus.json")
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

fn nonblank(value: &str) -> bool {
    !value.trim().is_empty()
}

fn is_date(value: &str) -> bool {
    value.len() == 10
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7) { byte == b'-' } else { byte.is_ascii_digit() }
        })
}

fn is_full_revision(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_stable_id(value: &str) -> bool {
    nonblank(value)
        && value.split(['.', '-']).all(|segment| {
            !segment.is_empty()
                && segment.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn repository_path(root: &Path, owner: &str, value: &str, must_exist: bool) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute()
        || value.contains('\\')
        || path.components().any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("{owner}: path must be a repository-relative normal path: {value}"));
    }
    if must_exist && !root.join(path).exists() {
        return Err(format!("{owner}: repository path does not exist: {value}"));
    }
    Ok(())
}

fn test_reference(root: &Path, owner: &str, value: &str) -> Result<(), String> {
    if !nonblank(value) {
        return Err(format!("{owner}: test reference cannot be blank"));
    }
    let (path, symbol) =
        value.split_once('#').map_or((value, None), |(path, symbol)| (path, Some(symbol)));
    repository_path(root, owner, path, true)?;
    if !path.ends_with(".rs") {
        return Err(format!("{owner}: test reference must name a Rust source file: {value}"));
    }
    if let Some(symbol) = symbol {
        if !nonblank(symbol) {
            return Err(format!("{owner}: test symbol cannot be blank: {value}"));
        }
        let source = std::fs::read_to_string(root.join(path))
            .map_err(|error| format!("{owner}: failed to read test reference {path}: {error}"))?;
        let function = format!("fn {symbol}");
        if !source.contains(&function) {
            return Err(format!("{owner}: test symbol does not exist: {value}"));
        }
    }
    Ok(())
}

fn nonblank_list(owner: &str, field: &str, values: &[String]) -> Result<(), String> {
    if values.is_empty() || values.iter().any(|value| !nonblank(value)) {
        return Err(format!("{owner}: {field} must be a non-empty nonblank list"));
    }
    Ok(())
}

fn validate(corpus: &Corpus) -> Result<(), String> {
    if corpus.schema_version != 1 || corpus.corpus != CORPUS_ID {
        return Err("unsupported mobile backend P0 corpus identity".into());
    }
    if !nonblank(&corpus.description)
        || !is_date(&corpus.created_on)
        || corpus.evidence_boundary != "internal_runtime"
    {
        return Err("invalid corpus metadata or evidence boundary".into());
    }
    if !is_full_revision(&corpus.authority.backend_base_revision) {
        return Err("backend authority requires a full commit revision".into());
    }

    let root = workspace_root();
    for path in [
        &corpus.authority.integration_corpus,
        &corpus.authority.application_parity_corpus,
        &corpus.authority.openspec_change,
    ] {
        repository_path(&root, "authority", path, true)?;
    }

    let integration: IntegrationCorpus =
        read_json(&root.join(&corpus.authority.integration_corpus))?;
    let integration_cases: HashMap<_, _> =
        integration.cases.into_iter().map(|item| (item.id, item.priority)).collect();
    let application: ApplicationParityCorpus =
        read_json(&root.join(&corpus.authority.application_parity_corpus))?;
    let parity_rows: HashSet<_> = application.parity_rows.into_iter().map(|row| row.id).collect();
    let required_rows: HashSet<_> = REQUIRED_ROWS.iter().copied().collect();
    let mut row_ids = HashSet::new();
    let mut test_owners = HashMap::<&str, &str>::new();

    for row in &corpus.rows {
        if !is_stable_id(&row.id)
            || !row.id.starts_with("mobile.backend.p0.")
            || !required_rows.contains(row.id.as_str())
            || !row_ids.insert(row.id.as_str())
        {
            return Err(format!("invalid, unexpected, or duplicate row ID: {}", row.id));
        }
        let _ = row.domain;
        nonblank_list(&row.id, "integration cases", &row.integration_cases)?;
        for case in &row.integration_cases {
            match integration_cases.get(case) {
                Some(priority) if priority == "p0" => {}
                Some(_) => return Err(format!("{}: integration case is not P0: {case}", row.id)),
                None => return Err(format!("{}: unknown integration case: {case}", row.id)),
            }
        }
        for journey in &row.parity_journeys {
            if !parity_rows.contains(journey) {
                return Err(format!("{}: unknown parity journey: {journey}", row.id));
            }
        }
        if !nonblank(&row.summary) {
            return Err(format!("{}: summary is required", row.id));
        }
        nonblank_list(&row.id, "owner paths", &row.owner_paths)?;
        for path in &row.owner_paths {
            repository_path(&root, &row.id, path, true)?;
            if path.contains("styrene-ui") || path.contains("/platform/") {
                return Err(format!("{}: frontend-owned path cannot own backend work", row.id));
            }
        }
        nonblank_list(&row.id, "assertions", &row.assertions)?;
        nonblank_list(&row.id, "forbidden outcomes", &row.forbidden_outcomes)?;
        nonblank_list(&row.id, "required tests", &row.required_tests)?;
        nonblank_list(&row.id, "exclusions", &row.exclusions)?;
        for test in &row.required_tests {
            test_reference(&root, &row.id, test)?;
            if let Some(previous) = test_owners.insert(test, &row.id) {
                return Err(format!(
                    "{}: required test is also owned by {previous}: {test}",
                    row.id
                ));
            }
        }
        if row.limits.max_seconds == 0
            || row.limits.max_seconds > 300
            || row.limits.max_items == 0
            || row.limits.max_items > 100_000
            || row.limits.max_bytes == 0
            || row.limits.max_bytes > 64 * 1024 * 1024
        {
            return Err(format!("{}: limits are absent or unbounded", row.id));
        }
        if row.frontend_handoff.contracts.iter().any(|value| !nonblank(value))
            || row.frontend_handoff.blocked_on.iter().any(|value| !nonblank(value))
        {
            return Err(format!("{}: frontend handoff contains a blank value", row.id));
        }
        if row.frontend_handoff.ready
            && (row.current_contract_state != ContractState::Available
                || row.delivery_state != DeliveryState::Verified
                || row.frontend_handoff.contracts.is_empty()
                || !row.frontend_handoff.blocked_on.is_empty())
        {
            return Err(format!("{}: frontend readiness is not supported by row state", row.id));
        }
        if row.delivery_state == DeliveryState::Verified {
            nonblank_list(&row.id, "evidence", &row.evidence)?;
            for evidence in &row.evidence {
                test_reference(&root, &row.id, evidence)?;
            }
        } else if !row.evidence.is_empty() {
            return Err(format!("{}: unverified row cannot retain pass evidence", row.id));
        }
        if row.current_contract_state == ContractState::Defective
            && row.forbidden_outcomes.is_empty()
        {
            return Err(format!("{}: defective row requires forbidden outcomes", row.id));
        }
    }

    if row_ids != required_rows {
        return Err("backend P0 rows must exactly cover the required set".into());
    }
    Ok(())
}

#[test]
fn mobile_backend_p0_corpus_is_complete_and_non_vacuous() {
    validate(&read_corpus()).unwrap_or_else(|error| panic!("invalid backend P0 corpus: {error}"));
}

#[cfg(test)]
mod mutation_tests {
    use super::*;
    use serde_json::Value;

    fn corpus_value() -> Value {
        serde_json::from_slice(&std::fs::read(corpus_path()).expect("read backend P0 corpus"))
            .expect("parse backend P0 corpus")
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
        value["rows"][0]["packaged_passed"] = Value::Bool(true);
        let error = serde_json::from_value::<Corpus>(value).expect_err("unknown field must fail");
        assert!(error.to_string().contains("unknown field `packaged_passed`"));
    }

    #[test]
    fn unknown_non_p0_and_unknown_parity_references_are_rejected() {
        let unknown = validation_error(|value| {
            value["rows"][0]["integration_cases"][0] =
                Value::String("mobile.propagation.missing".into());
        });
        assert!(unknown.contains("unknown integration case"));

        let non_p0 = validation_error(|value| {
            value["rows"][0]["integration_cases"][0] =
                Value::String("mobile.messaging.history-order-and-pagination".into());
        });
        assert!(non_p0.contains("is not P0"));

        let parity = validation_error(|value| {
            value["rows"][0]["parity_journeys"][0] = Value::String("mobile.journey.missing".into());
        });
        assert!(parity.contains("unknown parity journey"));
    }

    #[test]
    fn duplicate_rows_tests_and_unsafe_paths_are_rejected() {
        let duplicate = validation_error(|value| {
            value["rows"][1]["id"] = value["rows"][0]["id"].clone();
        });
        assert!(duplicate.contains("duplicate row ID"));

        let test = validation_error(|value| {
            value["rows"][1]["required_tests"][0] = value["rows"][0]["required_tests"][0].clone();
        });
        assert!(test.contains("also owned"));

        let path = validation_error(|value| {
            value["rows"][0]["owner_paths"][0] = Value::String("../outside.rs".into());
        });
        assert!(path.contains("repository-relative normal path"));

        let symbol = validation_error(|value| {
            value["rows"][0]["required_tests"][0] = Value::String(
                "crates/apps/styrened/tests/mobile_p0_backend.rs#missing_test".into(),
            );
        });
        assert!(symbol.contains("test symbol does not exist"));
    }

    #[test]
    fn blank_assertions_empty_tests_and_unbounded_limits_are_rejected() {
        let assertion = validation_error(|value| {
            value["rows"][0]["assertions"][0] = Value::String(" ".into());
        });
        assert!(assertion.contains("assertions"));

        let tests = validation_error(|value| {
            value["rows"][0]["required_tests"] = Value::Array(Vec::new());
        });
        assert!(tests.contains("required tests"));

        let limits = validation_error(|value| {
            value["rows"][0]["limits"]["max_seconds"] = Value::from(0);
        });
        assert!(limits.contains("limits are absent or unbounded"));
    }

    #[test]
    fn false_frontend_readiness_and_unverified_evidence_are_rejected() {
        let readiness = validation_error(|value| {
            value["rows"][0]["frontend_handoff"]["contracts"] = Value::Array(Vec::new());
        });
        assert!(readiness.contains("frontend readiness"));

        let evidence = validation_error(|value| {
            value["rows"][0]["delivery_state"] = Value::String("planned".into());
            value["rows"][0]["frontend_handoff"]["ready"] = Value::Bool(false);
        });
        assert!(evidence.contains("unverified row"));
    }
}
