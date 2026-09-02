use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use styrene_interop_runner::{
    PINNED_SCENARIOS, RunStatus, python_lxmf_scenario, run_live_scenario,
};

#[test]
fn compatibility_matrix_covers_first_slice() {
    assert_eq!(PINNED_SCENARIOS.len(), 13);
    assert_case_present("direct");
    assert_case_present("nomadnet_pages");
    assert_case_present("nomadnet_client");
    assert_case_present("propagated_retrieval");
    assert_case_present("propagated_capacity");
    assert_case_present("propagated_expiry");
    assert_case_present("routed_direct");
    assert_case_present("routed_direct_resource");
    assert_case_present("routed_nomadnet_pages");
    assert_case_present("routed_channel");
    assert_case_present("direct_resource");
    assert_case_present("opportunistic");
    assert_case_present("propagated_resource_lxm");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_direct() {
    run_case("direct");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_direct_resource() {
    run_case("direct_resource");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_opportunistic() {
    run_case("opportunistic");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_propagated_resource_lxm() {
    run_case("propagated_resource_lxm");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_propagated_retrieval() {
    run_case("propagated_retrieval");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_propagated_capacity() {
    run_case("propagated_capacity");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_propagated_expiry() {
    run_case("propagated_expiry");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_routed_direct() {
    run_case("routed_direct");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_routed_direct_resource() {
    run_case("routed_direct_resource");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_routed_nomadnet_pages() {
    run_case("routed_nomadnet_pages");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_routed_channel() {
    run_case("routed_channel");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_nomadnet_pages() {
    run_case("nomadnet_pages");
}

#[test]
#[ignore = "requires live Python compatibility harness environment"]
fn python_compat_nomadnet_client() {
    run_case("nomadnet_client");
}

fn run_case(case_id: &str) {
    let script = smoke_script_path();
    let python_bin = env::var("LXMF_PYTHON_BIN").unwrap_or_else(|_| "python3".to_string());
    let python_path = effective_python_path();
    ensure_environment(&script, &python_bin, python_path.as_deref()).unwrap_or_else(|reason| {
        panic!("python compatibility harness unavailable for '{case_id}': {reason}")
    });

    let timeout_secs =
        env::var("LXMF_PY_COMPAT_TIMEOUT").ok().and_then(|value| value.parse().ok()).unwrap_or(90);
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let scenario_id = case_id.parse().expect("catalog uses a canonical pinned scenario ID");
    let mut scenario = python_lxmf_scenario(
        &repo_root,
        scenario_id,
        Duration::from_secs(timeout_secs),
        &python_bin,
    );
    scenario.program = PathBuf::from("bash");
    if env::var_os("LXMF_PY_COMPAT_SMOKE").is_some() && !scenario_id_is_nomadnet(case_id) {
        scenario.args[0] = script.display().to_string();
    }
    scenario.env.insert("PYTHON_BIN".to_string(), python_bin);
    scenario.env.insert("TIMEOUT_SECS".to_string(), timeout_secs.to_string());
    if let Some(path) = python_path {
        scenario.env.insert("PYTHONPATH".to_string(), path.clone());
        for probe in &mut scenario.revision_probes {
            probe.env.insert("PYTHONPATH".to_string(), path.clone());
        }
    }
    scenario.evidence_dir = repo_root.join(format!("target/interop/ci/{case_id}-artifacts"));
    let evidence = run_live_scenario(&scenario).expect("failed to execute supervised smoke script");
    let evidence_path = repo_root.join(format!("target/interop/ci/{case_id}.json"));
    std::fs::create_dir_all(evidence_path.parent().expect("evidence path should have a parent"))
        .expect("failed to create interoperability evidence directory");
    std::fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&evidence).expect("failed to encode interoperability evidence"),
    )
    .expect("failed to retain interoperability evidence");
    assert_eq!(
        evidence.status,
        RunStatus::Passed,
        "python compatibility case '{case_id}' failed: {}\nevidence: {}\nstdout:\n{}\nstderr:\n{}",
        evidence.failure.as_deref().unwrap_or("unknown failure"),
        evidence_path.display(),
        evidence.logs[0].text,
        evidence.logs[1].text,
    );
}

fn scenario_id_is_nomadnet(case_id: &str) -> bool {
    matches!(
        case_id,
        "nomadnet_pages" | "nomadnet_client" | "routed_nomadnet_pages" | "routed_channel"
    )
}

fn smoke_script_path() -> PathBuf {
    env::var("LXMF_PY_COMPAT_SMOKE").map(PathBuf::from).unwrap_or_else(|_| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/python-lxmf-smoke.sh")
    })
}

fn effective_python_path() -> Option<String> {
    let from_env = env::var("PYTHONPATH").ok().filter(|v| !v.trim().is_empty());
    let repo_root = match Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize() {
        Ok(path) => path,
        Err(_) => return from_env,
    };
    let parent = match repo_root.parent() {
        Some(parent) => parent,
        None => return from_env,
    };
    let reticulum = parent.join("Reticulum");
    let lxmf = parent.join("LXMF");
    if !reticulum.exists() || !lxmf.exists() {
        return from_env;
    }
    Some(match from_env {
        Some(existing) => format!("{existing}:{}:{}", reticulum.display(), lxmf.display()),
        None => format!("{}:{}", reticulum.display(), lxmf.display()),
    })
}

fn ensure_environment(
    script: &Path,
    python_bin: &str,
    python_path: Option<&str>,
) -> Result<(), String> {
    if !script.exists() {
        return Err(format!("missing script at {}", script.display()));
    }

    let mut cmd = Command::new(python_bin);
    cmd.arg("-c")
        .arg("import importlib.util,sys;missing=[m for m in ('RNS','LXMF') if importlib.util.find_spec(m) is None];sys.exit(0 if not missing else 1)");
    if let Some(path) = python_path {
        cmd.env("PYTHONPATH", path);
    }
    let status = cmd
        .status()
        .map_err(|error| format!("unable to run python '{}': {}", python_bin, error))?;
    if !status.success() {
        return Err(
            "missing Python modules RNS/LXMF; set PYTHONPATH or install editable checkouts"
                .to_string(),
        );
    }
    Ok(())
}

fn assert_case_present(case_id: &str) {
    assert!(
        PINNED_SCENARIOS
            .iter()
            .any(|case| case.id.as_str() == case_id && !case.description.is_empty()),
        "missing compatibility case '{}'",
        case_id
    );
}
