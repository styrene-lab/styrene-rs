use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use styrene_interop_runner::{
    CancellationHandle, LiveScenario, PINNED_SCENARIOS, PinnedScenarioId, RunEvidence, RunStatus,
    python_lxmf_scenario,
};
use tokio::sync::{Mutex, watch};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioProfile {
    Fixture,
    LiveRunner,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioDefinition {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub profile: ScenarioProfile,
    pub revision: &'static str,
    pub controls: &'static [&'static str],
    pub runner_id: Option<PinnedScenarioId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioStatus {
    Running,
    Cancelling,
    Passed,
    Failed,
    TimedOut,
    Cancelled,
}

impl ScenarioStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Passed | Self::Failed | Self::TimedOut | Self::Cancelled)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ScenarioRun {
    pub run_id: String,
    pub scenario_id: &'static str,
    pub status: ScenarioStatus,
    pub milestones: Vec<String>,
    pub evidence: Vec<String>,
    pub runner_evidence: Option<RunEvidence>,
}

#[async_trait]
pub trait ScenarioBackend: Send + Sync {
    fn catalog(&self) -> &'static [ScenarioDefinition];
    fn availability(&self, scenario_id: &str) -> Result<(), String>;
    async fn start(&self, scenario_id: &str) -> Result<ScenarioRun, String>;
    async fn cancel(&self, run_id: &str) -> Result<ScenarioRun, String>;
    async fn wait(&self, run_id: &str) -> Result<ScenarioRun, String>;
    async fn export(&self, run_id: &str) -> Result<ScenarioRun, String>;
}

pub(crate) trait LiveScenarioExecutor: Send + Sync {
    fn availability(&self) -> Result<(), String> {
        Ok(())
    }

    fn run(
        &self,
        scenario: LiveScenario,
        cancellation: CancellationHandle,
    ) -> Result<RunEvidence, String>;
}

pub struct FixtureScenarioBackend {
    next_run: AtomicU64,
    active: Arc<Mutex<Option<ActiveScenario>>>,
    executor: Arc<dyn LiveScenarioExecutor>,
    live_enabled: bool,
}

struct ActiveScenario {
    run: ScenarioRun,
    cancellation: Option<CancellationHandle>,
    updates: watch::Sender<ScenarioRun>,
}

impl Default for FixtureScenarioBackend {
    fn default() -> Self {
        Self {
            next_run: AtomicU64::new(1),
            active: Arc::new(Mutex::new(None)),
            executor: Arc::new(crate::scenario_process::ProcessRunnerExecutor::new()),
            live_enabled: std::env::var("STYRENE_DX_LIVE_INTEROP").as_deref() == Ok("1"),
        }
    }
}

#[async_trait]
impl ScenarioBackend for FixtureScenarioBackend {
    fn catalog(&self) -> &'static [ScenarioDefinition] {
        &CATALOG
    }

    fn availability(&self, scenario_id: &str) -> Result<(), String> {
        let definition = CATALOG
            .iter()
            .find(|scenario| scenario.id == scenario_id)
            .ok_or_else(|| format!("unknown scenario '{scenario_id}'"))?;
        if definition.profile == ScenarioProfile::LiveRunner && !self.live_enabled {
            return Err("live interoperability runner is not explicitly enabled".into());
        }
        if definition.profile == ScenarioProfile::LiveRunner {
            self.executor.availability()?;
        }
        Ok(())
    }

    async fn start(&self, scenario_id: &str) -> Result<ScenarioRun, String> {
        self.availability(scenario_id)?;
        let definition = CATALOG
            .iter()
            .find(|scenario| scenario.id == scenario_id)
            .ok_or_else(|| format!("unknown scenario '{scenario_id}'"))?;
        if definition.profile == ScenarioProfile::LiveRunner {
            let runner_id = definition.runner_id.ok_or("live scenario has no runner ID")?;
            let timeout = std::env::var("STYRENE_INTEROP_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(90);
            let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
            let python_bin = std::env::var("LXMF_PYTHON_BIN").unwrap_or_else(|_| "python3".into());
            let mut scenario = python_lxmf_scenario(
                &repo_root,
                runner_id,
                Duration::from_secs(timeout),
                &python_bin,
            );
            scenario.evidence_dir = repo_root.join("target/interop/lab");
            let cancellation = CancellationHandle::default();
            let run = ScenarioRun {
                run_id: scenario.correlation_id.clone(),
                scenario_id: definition.id,
                status: ScenarioStatus::Running,
                milestones: vec!["registered with supervised runner".to_string()],
                evidence: vec![format!("correlation:{}", scenario.correlation_id)],
                runner_evidence: None,
            };
            let (updates, _) = watch::channel(run.clone());
            let mut active = self.active.lock().await;
            if active.as_ref().is_some_and(|item| !item.run.status.is_terminal()) {
                return Err("another scenario run is still active".into());
            }
            *active = Some(ActiveScenario {
                run: run.clone(),
                cancellation: Some(cancellation.clone()),
                updates,
            });
            drop(active);
            let active = self.active.clone();
            let executor = self.executor.clone();
            let run_id = run.run_id.clone();
            tokio::spawn(async move {
                let result =
                    tokio::task::spawn_blocking(move || executor.run(scenario, cancellation)).await;
                let mut current = active.lock().await;
                let Some(current) = current.as_mut().filter(|item| item.run.run_id == run_id)
                else {
                    return;
                };
                current.cancellation = None;
                current.run = match result {
                    Ok(Ok(evidence)) => scenario_run_from_evidence(definition.id, evidence),
                    Ok(Err(error)) => failed_scenario_run(definition.id, &run_id, error),
                    Err(error) => failed_scenario_run(definition.id, &run_id, error.to_string()),
                };
                current.updates.send_replace(current.run.clone());
            });
            return Ok(run);
        }
        let run = ScenarioRun {
            run_id: format!("fixture-run-{}", self.next_run.fetch_add(1, Ordering::Relaxed)),
            scenario_id: definition.id,
            status: ScenarioStatus::Running,
            milestones: vec!["fixture loaded".into(), "deterministic event stream ready".into()],
            evidence: vec![format!("fixture:{}", definition.revision)],
            runner_evidence: None,
        };
        let (updates, _) = watch::channel(run.clone());
        let mut active = self.active.lock().await;
        if active.as_ref().is_some_and(|item| !item.run.status.is_terminal()) {
            return Err("another scenario run is still active".into());
        }
        *active = Some(ActiveScenario { run: run.clone(), cancellation: None, updates });
        drop(active);
        let active = self.active.clone();
        let run_id = run.run_id.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            let evidence = fixture_run_evidence(definition, &run_id);
            let terminal = scenario_run_from_evidence(definition.id, evidence);
            let mut current = active.lock().await;
            let Some(current) = current.as_mut().filter(|item| {
                item.run.run_id == run_id && item.run.status == ScenarioStatus::Running
            }) else {
                return;
            };
            current.run = terminal;
            current.updates.send_replace(current.run.clone());
        });
        Ok(run)
    }

    async fn cancel(&self, run_id: &str) -> Result<ScenarioRun, String> {
        let mut active = self.active.lock().await;
        let active = active.as_mut().ok_or_else(|| "no active scenario".to_string())?;
        if active.run.run_id != run_id {
            return Err("scenario run is no longer active".into());
        }
        if active.run.status.is_terminal() {
            return Err("scenario run is already terminal".into());
        }
        if let Some(cancellation) = &active.cancellation {
            cancellation.cancel();
            active.run.milestones.push("cancellation requested".into());
            active.run.status = ScenarioStatus::Cancelling;
        } else {
            active.run.milestones.push("cancelled and fixture state released".into());
            active.run.status = ScenarioStatus::Cancelled;
        }
        active.updates.send_replace(active.run.clone());
        Ok(active.run.clone())
    }

    async fn wait(&self, run_id: &str) -> Result<ScenarioRun, String> {
        let mut updates = {
            let active = self.active.lock().await;
            let active = active.as_ref().ok_or_else(|| "no active scenario".to_string())?;
            if active.run.run_id != run_id {
                return Err("scenario run is no longer active".into());
            }
            active.updates.subscribe()
        };
        loop {
            let run = updates.borrow().clone();
            if run.status.is_terminal() {
                return Ok(run);
            }
            updates.changed().await.map_err(|_| {
                "scenario runner exited before publishing a terminal result".to_string()
            })?;
        }
    }

    async fn export(&self, run_id: &str) -> Result<ScenarioRun, String> {
        let run = {
            let active = self.active.lock().await;
            let active = active.as_ref().ok_or_else(|| "no active scenario".to_string())?;
            if active.run.run_id != run_id {
                return Err("scenario run is no longer active".into());
            }
            if !active.run.status.is_terminal() {
                return Err("scenario evidence is not terminal".into());
            }
            active.run.clone()
        };
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let export_dir = repo_root.join("target/interop/lab/exports");
        let safe_run_id = run
            .run_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let export_path = export_dir.join(format!("{safe_run_id}.json"));
        let encoded = serde_json::to_vec_pretty(&run).map_err(|error| error.to_string())?;
        if encoded.len() > 2 * 1024 * 1024 {
            return Err("scenario evidence exceeds the 2 MiB export limit".into());
        }
        let write_path = export_path.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            std::fs::create_dir_all(&export_dir).map_err(|error| error.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&export_dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(|error| error.to_string())?;
            }
            let temporary = write_path.with_extension("json.tmp");
            write_private_file(&temporary, &encoded)?;
            std::fs::rename(&temporary, &write_path).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())??;

        let mut active = self.active.lock().await;
        let active = active
            .as_mut()
            .filter(|item| item.run.run_id == run_id)
            .ok_or_else(|| "scenario run is no longer active".to_string())?;
        active.run.evidence.retain(|item| !item.starts_with("export:"));
        active.run.evidence.push(format!("export:target/interop/lab/exports/{safe_run_id}.json"));
        active.updates.send_replace(active.run.clone());
        Ok(active.run.clone())
    }
}

fn fixture_run_evidence(definition: &'static ScenarioDefinition, run_id: &str) -> RunEvidence {
    RunEvidence {
        schema_version: 2,
        scenario_id: definition.id.into(),
        correlation_id: run_id.into(),
        status: RunStatus::Passed,
        topology: styrene_interop_runner::TopologyEvidence {
            allocation_key: format!("fixture:{}", definition.id),
            candidate: 0,
            host: "fixture-only".into(),
            ports: std::collections::BTreeMap::new(),
            reservation_invariant: "fixture playback allocates no sockets or processes".into(),
        },
        revisions: vec![styrene_interop_runner::RevisionEvidence {
            name: "fixture-corpus".into(),
            expected: Some(definition.revision.into()),
            actual: Some(definition.revision.into()),
            matches: true,
            worktree_dirty: None,
            error: None,
            cleanup_complete: true,
        }],
        milestones: vec![
            styrene_interop_runner::TimedEvidence { name: "fixture-loaded".into(), elapsed_ms: 0 },
            styrene_interop_runner::TimedEvidence {
                name: "deterministic-events-replayed".into(),
                elapsed_ms: 1,
            },
        ],
        assertions: vec![styrene_interop_runner::AssertionEvidence {
            name: "fixture-playback-complete".into(),
            passed: true,
            detail: Some("no external implementation invoked".into()),
            elapsed_ms: 1,
        }],
        artifacts: Vec::new(),
        artifact_rejections: Vec::new(),
        event_rejections: Vec::new(),
        timings_ms: std::collections::BTreeMap::from([("total".into(), 1)]),
        logs: Vec::new(),
        cleanup: styrene_interop_runner::CleanupEvidence {
            direct_process_reaped: true,
            process_group_gone: true,
            pipes_drained: true,
            reader_threads_joined: true,
            temp_resources_removed: true,
            topology_reservation_released: true,
            ..styrene_interop_runner::CleanupEvidence::default()
        },
        process_exit: None,
        failure: None,
    }
}

fn scenario_run_from_evidence(scenario_id: &'static str, evidence: RunEvidence) -> ScenarioRun {
    let mut retained_evidence = evidence;
    redact_run_evidence(scenario_id, &mut retained_evidence);
    let mut summaries = vec![
        format!("correlation:{}", retained_evidence.correlation_id),
        format!(
            "topology:{}:{:?}",
            retained_evidence.topology.allocation_key, retained_evidence.topology.ports
        ),
        format!("timings:{:?}", retained_evidence.timings_ms),
        format!("cleanup:{:?}", retained_evidence.cleanup),
    ];
    if let Some(failure) = retained_evidence.failure.as_ref() {
        summaries.push(format!("failure:{failure}"));
    }
    summaries.extend(retained_evidence.revisions.iter().map(|revision| {
        format!(
            "revision:{}:actual={}:expected={}:dirty={:?}:cleanup={}",
            revision.name,
            revision.actual.as_deref().unwrap_or("unavailable"),
            revision.expected.as_deref().unwrap_or("any"),
            revision.worktree_dirty,
            revision.cleanup_complete
        )
    }));
    summaries.extend(
        retained_evidence
            .assertions
            .iter()
            .map(|assertion| format!("assertion:{}:{}", assertion.name, assertion.passed)),
    );
    summaries.extend(retained_evidence.artifacts.iter().map(|artifact| {
        format!(
            "artifact:{}:sha256:{}:retained={}",
            artifact.name, artifact.sha256, artifact.retained_path
        )
    }));
    summaries.extend(
        retained_evidence
            .artifact_rejections
            .iter()
            .map(|rejection| format!("artifact-rejected:{}:{}", rejection.name, rejection.reason)),
    );
    summaries.extend(retained_evidence.logs.iter().map(|log| {
        format!("log:{}:{} bytes:truncated={}", log.stream, log.bytes_seen, log.truncated)
    }));
    ScenarioRun {
        run_id: retained_evidence.correlation_id.clone(),
        scenario_id,
        status: match &retained_evidence.status {
            RunStatus::Passed => ScenarioStatus::Passed,
            RunStatus::Cancelled => ScenarioStatus::Cancelled,
            RunStatus::Failed => ScenarioStatus::Failed,
            RunStatus::TimedOut => ScenarioStatus::TimedOut,
        },
        milestones: retained_evidence
            .milestones
            .iter()
            .map(|milestone| format!("{} ({} ms)", milestone.name, milestone.elapsed_ms))
            .collect(),
        evidence: summaries,
        runner_evidence: Some(retained_evidence),
    }
}

fn failed_scenario_run(scenario_id: &'static str, run_id: &str, error: String) -> ScenarioRun {
    tracing::warn!(target: "dx::lab", error_bytes = error.len(), "runner failed before typed evidence finalization");
    ScenarioRun {
        run_id: run_id.to_string(),
        scenario_id,
        status: ScenarioStatus::Failed,
        milestones: vec!["runner failed before evidence finalization".to_string()],
        evidence: vec!["failure:runner failed before typed evidence finalization".into()],
        runner_evidence: None,
    }
}

fn redact_run_evidence(scenario_id: &str, evidence: &mut RunEvidence) {
    evidence.scenario_id = scenario_id.into();
    evidence.correlation_id = safe_evidence_label(&evidence.correlation_id);
    evidence.topology.allocation_key = "[REDACTED]".into();
    evidence.topology.host = if evidence.topology.host == "fixture-only" {
        "fixture-only".into()
    } else {
        "runner-managed loopback".into()
    };
    evidence.topology.reservation_invariant = "runner-managed reservation".into();
    evidence.topology.ports = evidence
        .topology
        .ports
        .iter()
        .enumerate()
        .map(|(index, (_, port))| (format!("endpoint-{}", index + 1), *port))
        .collect();
    for revision in &mut evidence.revisions {
        revision.name = safe_evidence_label(&revision.name);
        revision.expected = revision.expected.take().map(|value| safe_revision(&value));
        revision.actual = revision.actual.take().map(|value| safe_revision(&value));
        if revision.error.is_some() {
            revision.error = Some("revision probe failed".into());
        }
    }
    for milestone in &mut evidence.milestones {
        milestone.name = safe_evidence_label(&milestone.name);
    }
    for assertion in &mut evidence.assertions {
        assertion.name = safe_evidence_label(&assertion.name);
        if assertion.detail.is_some() {
            assertion.detail = Some("[REDACTED]".into());
        }
    }
    for artifact in &mut evidence.artifacts {
        artifact.name = safe_evidence_label(&artifact.name);
        artifact.sha256 = safe_sha256(&artifact.sha256);
        artifact.source_path = "[REDACTED]".into();
        artifact.retained_path = "[REDACTED]".into();
    }
    for rejection in &mut evidence.artifact_rejections {
        rejection.name = safe_evidence_label(&rejection.name);
        rejection.path = "[REDACTED]".into();
        rejection.reason = "artifact rejected by runner policy".into();
    }
    for log in &mut evidence.logs {
        log.stream = match log.stream.as_str() {
            "stdout" => "stdout".into(),
            "stderr" => "stderr".into(),
            _ => "runner".into(),
        };
        if !log.text.is_empty() {
            log.text = "[REDACTED]".into();
        }
    }
    evidence.timings_ms = evidence
        .timings_ms
        .iter()
        .enumerate()
        .map(|(index, (_, duration))| (format!("stage-{}", index + 1), *duration))
        .collect();
    if evidence.failure.is_some() {
        evidence.failure = Some("runner reported failure".into());
    }
}

fn safe_evidence_label(value: &str) -> String {
    let sensitive = ["secret", "token", "password", "private", "profile", "key="];
    let lowered = value.to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 96
        || value.contains('/')
        || value.contains('\\')
        || value.contains('=')
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || character == ' '
                || matches!(character, '-' | '_' | '.')
        })
        || sensitive.iter().any(|marker| lowered.contains(marker))
    {
        "[REDACTED]".into()
    } else {
        value.into()
    }
}

fn safe_sha256(value: &str) -> String {
    if value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()) {
        value.into()
    } else {
        "[REDACTED]".into()
    }
}

fn safe_revision(value: &str) -> String {
    let known_fixture = value.starts_with("fixture-")
        && value.len() <= 32
        && value.chars().all(|character| character.is_ascii_alphanumeric() || character == '-');
    let git_revision = (7..=64).contains(&value.len())
        && value.chars().all(|character| character.is_ascii_hexdigit());
    if known_fixture || git_revision { value.into() } else { "[REDACTED]".into() }
}

fn write_private_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;

    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

const CATALOG: [ScenarioDefinition; 7] = [
    ScenarioDefinition {
        id: "fixture-discovery",
        title: "Discovery Baseline",
        description: "Replays accepted announces without inventing routes or links.",
        profile: ScenarioProfile::Fixture,
        revision: "fixture-v1",
        controls: &["reset", "replay"],
        runner_id: None,
    },
    ScenarioDefinition {
        id: "fixture-routed",
        title: "Routed Delivery",
        description: "Replays a multi-hop route, link lifecycle, and delivery evidence.",
        profile: ScenarioProfile::Fixture,
        revision: "fixture-v1",
        controls: &["announce", "request-path", "send-message"],
        runner_id: None,
    },
    ScenarioDefinition {
        id: "fixture-degraded",
        title: "Degraded Runtime",
        description: "Exercises stale observations and unavailable transport state.",
        profile: ScenarioProfile::Fixture,
        revision: "fixture-v1",
        controls: &["reset", "replay"],
        runner_id: None,
    },
    ScenarioDefinition {
        id: "fixture-high-cardinality",
        title: "500 Peer Load",
        description: "Loads the deterministic high-cardinality network fixture.",
        profile: ScenarioProfile::Fixture,
        revision: "fixture-v1",
        controls: &["reset", "replay"],
        runner_id: None,
    },
    ScenarioDefinition {
        id: PINNED_SCENARIOS[0].id.as_str(),
        title: PINNED_SCENARIOS[0].title,
        description: PINNED_SCENARIOS[0].description,
        profile: ScenarioProfile::LiveRunner,
        revision: "pinned-harness",
        controls: PINNED_SCENARIOS[0].controls,
        runner_id: Some(PINNED_SCENARIOS[0].id),
    },
    ScenarioDefinition {
        id: PINNED_SCENARIOS[1].id.as_str(),
        title: PINNED_SCENARIOS[1].title,
        description: PINNED_SCENARIOS[1].description,
        profile: ScenarioProfile::LiveRunner,
        revision: "pinned-harness",
        controls: PINNED_SCENARIOS[1].controls,
        runner_id: Some(PINNED_SCENARIOS[1].id),
    },
    ScenarioDefinition {
        id: PINNED_SCENARIOS[2].id.as_str(),
        title: PINNED_SCENARIOS[2].title,
        description: PINNED_SCENARIOS[2].description,
        profile: ScenarioProfile::LiveRunner,
        revision: "pinned-harness",
        controls: PINNED_SCENARIOS[2].controls,
        runner_id: Some(PINNED_SCENARIOS[2].id),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct FakeExecutor {
        status: RunStatus,
        delay: Duration,
    }

    struct PanicExecutor;

    impl LiveScenarioExecutor for PanicExecutor {
        fn run(
            &self,
            _scenario: LiveScenario,
            _cancellation: CancellationHandle,
        ) -> Result<RunEvidence, String> {
            panic!("fixture scenario attempted external runner execution")
        }
    }

    impl LiveScenarioExecutor for FakeExecutor {
        fn run(
            &self,
            scenario: LiveScenario,
            _cancellation: CancellationHandle,
        ) -> Result<RunEvidence, String> {
            std::thread::sleep(self.delay);
            Ok(RunEvidence {
                schema_version: 2,
                scenario_id: scenario.id,
                correlation_id: scenario.correlation_id,
                status: self.status.clone(),
                topology: styrene_interop_runner::TopologyEvidence {
                    allocation_key: "token=structured-secret".into(),
                    candidate: 1,
                    host: "127.0.0.1".into(),
                    ports: BTreeMap::new(),
                    reservation_invariant: "password=structured-secret".into(),
                },
                revisions: vec![styrene_interop_runner::RevisionEvidence {
                    name: "secret-revision".into(),
                    expected: Some("token=structured-secret".into()),
                    actual: Some("password=structured-secret".into()),
                    matches: false,
                    worktree_dirty: None,
                    error: Some("/private/structured-secret".into()),
                    cleanup_complete: true,
                }],
                milestones: vec![styrene_interop_runner::TimedEvidence {
                    name: "token=structured-secret".into(),
                    elapsed_ms: 1,
                }],
                assertions: vec![styrene_interop_runner::AssertionEvidence {
                    name: "password=structured-secret".into(),
                    passed: false,
                    detail: Some("/private/structured-secret".into()),
                    elapsed_ms: 1,
                }],
                artifacts: vec![styrene_interop_runner::ArtifactEvidence {
                    name: "secret-report".into(),
                    source_path: "/private/fixture-secret".into(),
                    retained_path: "target/interop/retained/report.json".into(),
                    bytes: 32,
                    sha256: "abc123".into(),
                }],
                artifact_rejections: vec![styrene_interop_runner::ArtifactRejection {
                    name: "token=rejected".into(),
                    path: "/private/structured-secret".into(),
                    reason: "password=structured-secret".into(),
                }],
                event_rejections: Vec::new(),
                timings_ms: BTreeMap::from([("secret-stage".into(), 1)]),
                logs: vec![styrene_interop_runner::LogEvidence {
                    stream: "stdout".into(),
                    text: "fixture-secret".into(),
                    bytes_seen: 14,
                    truncated: false,
                }],
                cleanup: styrene_interop_runner::CleanupEvidence::default(),
                process_exit: None,
                failure: Some("token=structured-secret /private/operator".into()),
            })
        }
    }

    fn backend_with(status: RunStatus, live_enabled: bool) -> FixtureScenarioBackend {
        FixtureScenarioBackend {
            next_run: AtomicU64::new(1),
            active: Arc::new(Mutex::new(None)),
            executor: Arc::new(FakeExecutor { status, delay: Duration::ZERO }),
            live_enabled,
        }
    }

    #[tokio::test]
    async fn fixture_scenarios_complete_through_typed_runner_contract() {
        let backend = FixtureScenarioBackend {
            next_run: AtomicU64::new(1),
            active: Arc::new(Mutex::new(None)),
            executor: Arc::new(PanicExecutor),
            live_enabled: false,
        };
        assert_eq!(backend.catalog()[0].id, "fixture-discovery");
        let run = backend.start("fixture-routed").await.unwrap();
        assert_eq!(run.status, ScenarioStatus::Running);
        assert!(!run.evidence.is_empty());
        let terminal = backend.wait(&run.run_id).await.expect("fixture terminal result");
        assert_eq!(terminal.status, ScenarioStatus::Passed);
        let evidence = terminal.runner_evidence.expect("typed fixture evidence");
        assert_eq!(evidence.topology.host, "fixture-only");
        assert!(evidence.topology.ports.is_empty());
        assert_eq!(evidence.assertions[0].name, "fixture-playback-complete");
        assert!(backend.cancel(&run.run_id).await.unwrap_err().contains("already terminal"));
    }

    #[tokio::test]
    async fn live_catalog_adapts_every_canonical_runner_scenario() {
        let backend = backend_with(RunStatus::Passed, false);
        let live = backend
            .catalog()
            .iter()
            .filter(|scenario| scenario.profile == ScenarioProfile::LiveRunner)
            .collect::<Vec<_>>();
        assert_eq!(live.len(), PINNED_SCENARIOS.len());
        for definition in live {
            let runner_id = definition.runner_id.expect("live runner ID");
            assert_eq!(definition.id, runner_id.as_str());
            assert!(backend.availability(definition.id).unwrap_err().contains("not explicitly"));
        }
    }

    #[tokio::test]
    async fn fake_live_runner_publishes_typed_terminal_evidence_without_python() {
        let backend = backend_with(RunStatus::TimedOut, true);
        let running = backend.start("opportunistic").await.expect("start fake runner");
        assert_eq!(running.status, ScenarioStatus::Running);

        let terminal = backend.wait(&running.run_id).await.expect("terminal runner evidence");

        assert_eq!(terminal.status, ScenarioStatus::TimedOut);
        let encoded = serde_json::to_string(&terminal).expect("serialize retained evidence");
        assert!(!encoded.contains("structured-secret"));
        assert!(!encoded.contains("/private/"));
        assert!(!encoded.contains("target/interop/retained"));
        let evidence = terminal.runner_evidence.expect("typed runner evidence");
        assert_eq!(evidence.scenario_id, "opportunistic");
        assert_eq!(evidence.status, RunStatus::TimedOut);
        assert_eq!(evidence.logs[0].text, "[REDACTED]");
        assert_eq!(evidence.artifacts[0].source_path, "[REDACTED]");
        assert!(!terminal.evidence.iter().any(|item| item.contains("fixture-secret")));
    }

    #[tokio::test]
    async fn active_runs_block_replacement_then_terminal_runs_can_rerun_and_export() {
        let backend = FixtureScenarioBackend {
            next_run: AtomicU64::new(1),
            active: Arc::new(Mutex::new(None)),
            executor: Arc::new(FakeExecutor {
                status: RunStatus::Passed,
                delay: Duration::from_millis(25),
            }),
            live_enabled: true,
        };
        let running = backend.start("direct").await.expect("start first run");
        assert!(backend.start("opportunistic").await.unwrap_err().contains("still active"));
        let terminal = backend.wait(&running.run_id).await.expect("first terminal result");
        assert_eq!(terminal.status, ScenarioStatus::Passed);

        let exported = backend.export(&terminal.run_id).await.expect("export terminal evidence");
        let export_path = exported
            .evidence
            .iter()
            .find_map(|item| item.strip_prefix("export:"))
            .expect("export path");
        let export_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").join(export_path);
        let encoded = std::fs::read_to_string(&export_path).expect("read exported evidence");
        assert!(encoded.contains("[REDACTED]"));
        assert!(!encoded.contains("fixture-secret"));
        assert!(!encoded.contains("structured-secret"));
        assert!(!encoded.contains("/private/"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode =
                std::fs::metadata(&export_path).expect("export metadata").permissions().mode()
                    & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_file(export_path).expect("remove test export");

        let rerun = backend.start("direct").await.expect("rerun terminal scenario");
        assert_ne!(rerun.run_id, running.run_id);
        assert_eq!(rerun.status, ScenarioStatus::Running);
        assert_eq!(
            backend.wait(&rerun.run_id).await.expect("rerun terminal result").status,
            ScenarioStatus::Passed
        );
    }

    #[tokio::test]
    async fn live_cancellation_is_forwarded_to_registered_runner() {
        let backend = backend_with(RunStatus::Cancelled, true);
        let cancellation = CancellationHandle::default();
        let run = ScenarioRun {
            run_id: "live-test".to_string(),
            scenario_id: "direct",
            status: ScenarioStatus::Running,
            milestones: vec!["registered with supervised runner".to_string()],
            evidence: Vec::new(),
            runner_evidence: None,
        };
        let (updates, _) = watch::channel(run.clone());
        *backend.active.lock().await =
            Some(ActiveScenario { run, cancellation: Some(cancellation.clone()), updates });

        let cancelled = backend.cancel("live-test").await.expect("cancel registered run");

        assert!(cancellation.is_cancelled());
        assert_eq!(cancelled.status, ScenarioStatus::Cancelling);
        assert!(cancelled.milestones.iter().any(|item| item == "cancellation requested"));
    }
}
