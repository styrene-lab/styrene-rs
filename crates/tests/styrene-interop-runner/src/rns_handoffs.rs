use std::collections::HashSet;
use std::path::Path;

use serde::Deserialize;

use crate::rns_fixtures::{load_rns_index, repository_file, rns_vector};

const RNS_1_5_1_REVISION: &str = "149e4151095adf098b8f53eab0c03b37169e8559";

#[derive(Debug, Deserialize)]
pub struct RnsLiveHandoff {
    pub schema_version: u32,
    pub runner: String,
    pub authority_id: String,
    pub authority_revision: String,
    pub registered: bool,
    pub enabled: bool,
    pub claim_status: String,
    pub scenarios: Vec<RnsLiveHandoffScenario>,
}

#[derive(Debug, Deserialize)]
pub struct RnsLiveHandoffScenario {
    pub id: String,
    pub state: String,
    pub owner_tasks: Vec<String>,
    pub fixture_vectors: Vec<String>,
    pub topology: String,
    pub required_milestones: Vec<String>,
    pub required_assertions: Vec<String>,
    pub cancellation: String,
    pub cleanup: String,
    pub timeout_secs: u64,
    pub max_log_bytes: u64,
    pub max_artifacts: usize,
    pub max_artifact_bytes: u64,
    pub artifact_sha256_required: bool,
    pub revision_attestation_required: bool,
}

pub fn load_rns_live_handoff(root: &Path) -> Result<RnsLiveHandoff, Vec<String>> {
    let root = std::fs::canonicalize(root)
        .map_err(|error| vec![format!("failed to resolve {}: {error}", root.display())])?;
    let path =
        repository_file(&root, &root.join("tests/interop/handoffs/reticulum-1.5.1-live.json"))
            .map_err(|error| vec![error])?;
    let data = std::fs::read_to_string(&path)
        .map_err(|error| vec![format!("failed to read {}: {error}", path.display())])?;
    let handoff: RnsLiveHandoff = serde_json::from_str(&data)
        .map_err(|error| vec![format!("failed to parse {}: {error}", path.display())])?;
    let index = load_rns_index(&root)?;
    let mut errors = Vec::new();
    if handoff.schema_version != 1
        || handoff.runner != "styrene-interop-runner"
        || handoff.authority_id != "rns-1.5.1"
        || handoff.authority_revision != RNS_1_5_1_REVISION
    {
        errors.push("RNS live handoff authority contract is invalid".to_string());
    }
    if handoff.registered || handoff.enabled || handoff.claim_status != "unevidenced" {
        errors.push("RNS live handoff must remain unregistered and unevidenced".to_string());
    }
    if handoff.scenarios.len() != 3 {
        errors.push("RNS live handoff must contain exactly three scenarios".to_string());
    }
    let mut ids = HashSet::new();
    let mut owner_tasks = HashSet::new();
    for scenario in &handoff.scenarios {
        if scenario.id.is_empty() || !ids.insert(scenario.id.as_str()) {
            errors.push(format!("missing or duplicate handoff scenario id: {}", scenario.id));
        }
        owner_tasks.extend(scenario.owner_tasks.iter().map(String::as_str));
        if scenario.state != "handoff_only"
            || scenario.fixture_vectors.is_empty()
            || scenario.topology.is_empty()
            || scenario.required_milestones.is_empty()
            || scenario.required_assertions.is_empty()
            || scenario.cancellation.is_empty()
            || scenario.cleanup.is_empty()
            || scenario.timeout_secs == 0
            || scenario.max_log_bytes == 0
            || scenario.max_artifacts == 0
            || scenario.max_artifact_bytes == 0
            || !scenario.artifact_sha256_required
            || !scenario.revision_attestation_required
        {
            errors.push(format!("{}: incomplete or unbounded handoff contract", scenario.id));
        }
        for vector_id in &scenario.fixture_vectors {
            match rns_vector(&index, vector_id) {
                Ok(vector) if vector.authority_id == handoff.authority_id => {}
                Ok(vector) => errors.push(format!(
                    "{}: vector {vector_id} uses authority {}",
                    scenario.id, vector.authority_id
                )),
                Err(error) => errors.push(format!("{}: {error}", scenario.id)),
            }
        }
    }
    for required in [
        "reticulum-lxmf-nomadnet-parity:4.7",
        "reticulum-lxmf-nomadnet-parity:5.7",
        "reticulum-lxmf-nomadnet-parity:8.8",
        "reticulum-lxmf-nomadnet-parity:12.6",
    ] {
        if !owner_tasks.contains(required) {
            errors.push(format!("RNS live handoff is missing owner task {required}"));
        }
    }
    if errors.is_empty() { Ok(handoff) } else { Err(errors) }
}
