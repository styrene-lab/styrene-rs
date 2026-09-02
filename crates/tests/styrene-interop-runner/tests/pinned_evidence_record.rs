//! The committed record of hosted pinned live runs.
//!
//! `tests/interop/handoffs/pinned-live-evidence.json` records every
//! `live-interop.yml` dispatch whose scenarios passed, at which `styrene-rs`
//! revision, against which pinned upstream revisions. Product parity claims
//! may promote a live gate only when this record carries a passing run for
//! every scenario the gate names, so the record must stay consistent with the
//! runner's pins, the workflow matrix, and the capability registry.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use styrene_interop_runner::{PINNED_SCENARIOS, PinnedScenarioId, python_lxmf_scenario};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(workspace_root().join(path))
        .unwrap_or_else(|error| panic!("read {path}: {error}"))
}

fn record() -> serde_json::Value {
    serde_json::from_str(&read("tests/interop/handoffs/pinned-live-evidence.json"))
        .expect("pinned live evidence record parses")
}

fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Scenario ids with at least one recorded passing hosted run.
fn passing_scenarios(record: &serde_json::Value) -> BTreeSet<String> {
    let mut passing = BTreeSet::new();
    for run in record["runs"].as_array().expect("runs") {
        if run["conclusion"] != "success" {
            continue;
        }
        for (scenario, conclusion) in run["scenarios"].as_object().expect("scenarios") {
            if conclusion == "success" {
                passing.insert(scenario.clone());
            }
        }
    }
    passing
}

#[test]
fn record_pins_match_the_runner_probes() {
    let record = record();
    assert_eq!(record["schema_version"], 1);
    assert_eq!(record["runner"], "styrene-interop-runner");
    assert_eq!(record["workflow"], ".github/workflows/live-interop.yml");
    assert_eq!(record["trigger"], "workflow_dispatch");
    let scenario = python_lxmf_scenario(
        &workspace_root(),
        PinnedScenarioId::Direct,
        Duration::from_secs(300),
        "python3",
    );
    let probes: BTreeMap<_, _> = scenario
        .revision_probes
        .iter()
        .filter_map(|probe| probe.expected.clone().map(|expected| (probe.name.clone(), expected)))
        .collect();
    let authorities = record["authorities"].as_object().expect("authorities");
    for name in ["python-rns", "python-lxmf", "python-nomadnet"] {
        let authority = &authorities[name];
        assert_eq!(
            authority["revision"].as_str(),
            probes.get(name).map(String::as_str),
            "{name} revision must match the runner's revision probe"
        );
        assert!(!authority["version"].as_str().unwrap_or_default().is_empty());
    }
    assert_eq!(authorities.len(), 3, "only the three pinned Python authorities are recorded");
}

#[test]
fn every_pinned_scenario_has_a_passing_hosted_run() {
    let record = record();
    let workflow = read(".github/workflows/live-interop.yml");
    let pinned: BTreeSet<&str> =
        PINNED_SCENARIOS.iter().map(|scenario| scenario.id.as_str()).collect();
    let mut run_ids = BTreeSet::new();
    for run in record["runs"].as_array().expect("runs") {
        let run_id = run["run_id"].as_u64().expect("run id");
        assert!(run_ids.insert(run_id), "duplicate run id {run_id}");
        let url = run["url"].as_str().expect("url");
        assert!(url.ends_with(&format!("/actions/runs/{run_id}")), "{url}");
        assert!(is_hex(run["styrene_rs_revision"].as_str().expect("revision"), 40));
        assert!(!run["recorded_at"].as_str().unwrap_or_default().is_empty());
        assert_eq!(run["conclusion"], "success", "only passing dispatches are evidence");
        let scenarios = run["scenarios"].as_object().expect("scenarios");
        assert!(!scenarios.is_empty());
        for (scenario, conclusion) in scenarios {
            assert!(pinned.contains(scenario.as_str()), "unknown scenario {scenario}");
            assert!(workflow.contains(&format!("- {scenario}")), "{scenario} missing from matrix");
            assert_eq!(conclusion, "success", "{scenario} in run {run_id}");
        }
    }
    let passing = passing_scenarios(&record);
    for scenario in pinned {
        assert!(passing.contains(scenario), "no passing hosted run recorded for {scenario}");
    }
}

/// Parse `[[parity_gates]]` blocks without a TOML dependency: the fields the
/// test needs are single-line scalars and one-line or multi-line arrays.
fn live_gate_blocks(registry: &str) -> Vec<&str> {
    registry
        .split("[[parity_gates]]")
        .skip(1)
        .filter(|block| block.contains("kind = \"live\""))
        .collect()
}

fn scalar<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    block.lines().map(str::trim).find_map(|line| {
        line.strip_prefix(key)
            .and_then(|rest| rest.trim_start().strip_prefix('='))
            .map(|value| value.trim().trim_matches('"'))
    })
}

#[test]
fn enabled_live_gates_name_only_scenarios_with_passing_evidence() {
    let record = record();
    let passing = passing_scenarios(&record);
    let registry = read("product/capabilities-v1.toml");
    let mut enabled_gates = 0;
    for block in live_gate_blocks(&registry) {
        let gate = scalar(block, "id").expect("gate id");
        let ignored = scalar(block, "ignored").expect("ignored flag");
        if ignored == "true" {
            continue;
        }
        enabled_gates += 1;
        assert!(
            block.contains("tests/interop/handoffs/pinned-live-evidence.json"),
            "{gate} must cite the hosted evidence record"
        );
        let command = scalar(block, "command").expect("command");
        let named: Vec<&str> = command
            .split("styrene-interop ")
            .skip(1)
            .map(|rest| rest.split_whitespace().next().expect("scenario id"))
            .collect();
        assert!(!named.is_empty(), "{gate} must run at least one pinned scenario");
        for scenario in named {
            assert!(
                PINNED_SCENARIOS.iter().any(|pinned| pinned.id.as_str() == scenario),
                "{gate} names unknown scenario {scenario}"
            );
            assert!(passing.contains(scenario), "{gate} names {scenario} without passing evidence");
        }
    }
    assert!(enabled_gates > 0, "at least one live gate must be enabled");
}
