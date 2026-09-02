use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub mod rns_fixtures;
pub mod rns_handoffs;

const EVENT_PREFIX: &str = "STYRENE_EVENT ";
const MAX_EVENT_LINE_BYTES: usize = 16 * 1024;
const MAX_PROBE_BYTES: usize = 4096;
const MAX_PROCESS_SNAPSHOT_BYTES: usize = 1024 * 1024;
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(20);
const TERMINATION_GRACE: Duration = Duration::from_secs(2);
const FINAL_PIPE_DRAIN_GRACE: Duration = Duration::from_secs(1);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const TOPOLOGY_CANDIDATES: u16 = 5_000;
const TOPOLOGY_HANDOFF_RETRIES: usize = 8;
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinnedScenarioId {
    Direct,
    DirectResource,
    Opportunistic,
    PropagatedResourceLxm,
}

impl PinnedScenarioId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::DirectResource => "direct_resource",
            Self::Opportunistic => "opportunistic",
            Self::PropagatedResourceLxm => "propagated_resource_lxm",
        }
    }

    /// Direct, resource-backed Direct, and Opportunistic gates exercise both
    /// endpoints: Python sends to Rust, then Rust sends to the same Python
    /// peer. The propagated scenario proves Python-to-Rust propagation intake
    /// only.
    pub const fn is_bidirectional(self) -> bool {
        matches!(self, Self::Direct | Self::DirectResource | Self::Opportunistic)
    }

    /// Wire representation the Rust outbound route must record for the
    /// Rust-to-Python leg of a bidirectional scenario.
    pub const fn expected_outbound_representation(self) -> &'static str {
        match self {
            Self::DirectResource => "resource",
            Self::Direct | Self::Opportunistic | Self::PropagatedResourceLxm => "packet",
        }
    }

    /// LXMF representation code the Python sender must report for the
    /// Python-to-Rust leg: 1 is PACKET, 2 is RESOURCE.
    pub const fn expected_python_representation(self) -> &'static str {
        match self {
            Self::DirectResource | Self::PropagatedResourceLxm => "2",
            Self::Direct | Self::Opportunistic => "1",
        }
    }
}

impl std::fmt::Display for PinnedScenarioId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PinnedScenarioId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        PINNED_SCENARIOS
            .iter()
            .find(|scenario| scenario.id.as_str() == value)
            .map(|scenario| scenario.id)
            .ok_or_else(|| format!("unsupported pinned interoperability scenario '{value}'"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinnedScenarioDefinition {
    pub id: PinnedScenarioId,
    pub title: &'static str,
    pub description: &'static str,
    pub controls: &'static [&'static str],
}

pub const PINNED_SCENARIOS: &[PinnedScenarioDefinition] = &[
    PinnedScenarioDefinition {
        id: PinnedScenarioId::Direct,
        title: "Pinned Direct Interop",
        description: "Runs the shared direct-delivery harness through the supervised runner.",
        controls: &["announce", "send-message", "cancel"],
    },
    PinnedScenarioDefinition {
        id: PinnedScenarioId::DirectResource,
        title: "Pinned Direct Resource Interop",
        description: "Runs the shared direct harness with resource-backed messages in both directions.",
        controls: &["announce", "send-resource", "cancel"],
    },
    PinnedScenarioDefinition {
        id: PinnedScenarioId::Opportunistic,
        title: "Pinned Opportunistic Interop",
        description: "Runs the shared opportunistic-delivery harness through the supervised runner.",
        controls: &["announce", "send-message", "cancel"],
    },
    PinnedScenarioDefinition {
        id: PinnedScenarioId::PropagatedResourceLxm,
        title: "Pinned Propagated Resource Interop",
        description: "Runs the shared propagated resource-backed LXMF harness.",
        controls: &["announce", "send-resource", "cancel"],
    },
];

pub fn pinned_scenario(value: &str) -> Option<&'static PinnedScenarioDefinition> {
    PINNED_SCENARIOS.iter().find(|scenario| scenario.id.as_str() == value)
}

#[derive(Clone, Debug)]
pub struct RevisionProbe {
    pub name: String,
    pub expected: Option<String>,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub worktree: Option<PathBuf>,
    pub timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct LiveScenario {
    pub id: String,
    pub correlation_id: String,
    pub revision_probes: Vec<RevisionProbe>,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub timeout: Duration,
    pub required_milestones: Vec<String>,
    pub required_assertions: Vec<String>,
    pub required_artifacts: Vec<String>,
    pub max_log_bytes: usize,
    pub max_artifact_bytes: u64,
    pub max_artifacts: usize,
    pub max_artifact_total_bytes: u64,
    pub evidence_dir: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopologyEvidence {
    pub allocation_key: String,
    pub candidate: u16,
    pub host: String,
    pub ports: BTreeMap<String, u16>,
    pub reservation_invariant: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevisionEvidence {
    pub name: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub matches: bool,
    pub worktree_dirty: Option<bool>,
    pub error: Option<String>,
    pub cleanup_complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimedEvidence {
    pub name: String,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssertionEvidence {
    pub name: String,
    pub passed: bool,
    pub detail: Option<String>,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactEvidence {
    pub name: String,
    pub source_path: String,
    pub retained_path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRejection {
    pub name: String,
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogEvidence {
    pub stream: String,
    pub text: String,
    pub bytes_seen: u64,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupEvidence {
    pub termination_requested: bool,
    pub kill_escalated: bool,
    pub direct_process_reaped: bool,
    pub process_group_gone: bool,
    pub descendant_discovery_supported: bool,
    pub owned_descendants_gone: Option<bool>,
    pub remaining_owned_pids: Vec<u32>,
    pub pipes_drained: bool,
    pub reader_threads_joined: bool,
    pub temp_resources_removed: bool,
    pub topology_reservation_released: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Passed,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunEvidence {
    pub schema_version: u16,
    pub scenario_id: String,
    pub correlation_id: String,
    pub status: RunStatus,
    pub topology: TopologyEvidence,
    pub revisions: Vec<RevisionEvidence>,
    pub milestones: Vec<TimedEvidence>,
    pub assertions: Vec<AssertionEvidence>,
    pub artifacts: Vec<ArtifactEvidence>,
    pub artifact_rejections: Vec<ArtifactRejection>,
    pub event_rejections: Vec<String>,
    pub timings_ms: BTreeMap<String, u64>,
    pub logs: Vec<LogEvidence>,
    pub cleanup: CleanupEvidence,
    pub process_exit: Option<i32>,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CancellationHandle {
    cancelled: Arc<AtomicBool>,
}

impl CancellationHandle {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

pub struct TopologyReservation {
    evidence: TopologyEvidence,
    listeners: Vec<TcpListener>,
    lock_path: PathBuf,
    released: bool,
}

impl TopologyReservation {
    pub fn evidence(&self) -> &TopologyEvidence {
        &self.evidence
    }

    /// Hands the validated sockets to a child that binds by port number.
    ///
    /// The cooperative allocation lock remains held until the reservation is
    /// finalized, preventing another runner from selecting this block. An
    /// unrelated process can still race the child after this handoff.
    pub fn handoff_to_child(&mut self) -> io::Result<()> {
        self.listeners.clear();
        let base = self.evidence.ports["rust_rpc"];
        self.listeners = bind_topology(base)?;
        self.listeners.clear();
        Ok(())
    }

    fn release(mut self) -> bool {
        self.listeners.clear();
        let released = fs::remove_dir(&self.lock_path).is_ok() && !self.lock_path.exists();
        self.released = released;
        released
    }
}

impl Drop for TopologyReservation {
    fn drop(&mut self) {
        self.listeners.clear();
        if !self.released {
            let _ = fs::remove_dir(&self.lock_path);
        }
    }
}

#[derive(Deserialize)]
struct RunnerEvent {
    kind: String,
    name: String,
    #[serde(default)]
    passed: Option<bool>,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    correlation_id: Option<String>,
}

#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

enum StreamMessage {
    Chunk(StreamKind, Vec<u8>),
    Closed,
}

struct BoundedLog {
    stream: &'static str,
    bytes: Vec<u8>,
    bytes_seen: u64,
}

impl BoundedLog {
    fn new(stream: &'static str) -> Self {
        Self { stream, bytes: Vec::new(), bytes_seen: 0 }
    }

    fn push(&mut self, chunk: &[u8], limit: usize) {
        self.bytes_seen = self.bytes_seen.saturating_add(chunk.len() as u64);
        let remaining = limit.saturating_sub(self.bytes.len());
        self.bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    fn finish(self) -> LogEvidence {
        LogEvidence {
            stream: self.stream.to_string(),
            text: String::from_utf8_lossy(&self.bytes).into_owned(),
            bytes_seen: self.bytes_seen,
            truncated: self.bytes_seen > self.bytes.len() as u64,
        }
    }
}

struct EventLines {
    pending: Vec<u8>,
    oversized: bool,
}

impl EventLines {
    fn new() -> Self {
        Self { pending: Vec::new(), oversized: false }
    }

    fn push<F>(&mut self, chunk: &[u8], mut handle: F)
    where
        F: FnMut(&[u8]),
    {
        for byte in chunk {
            if *byte == b'\n' {
                if !self.oversized {
                    handle(&self.pending);
                }
                self.pending.clear();
                self.oversized = false;
            } else if self.pending.len() < MAX_EVENT_LINE_BYTES {
                self.pending.push(*byte);
            } else {
                self.oversized = true;
            }
        }
    }
}

pub fn reserve_topology(allocation_key: &str) -> io::Result<TopologyReservation> {
    let hash = stable_hash(allocation_key);
    let lock_root = std::env::temp_dir().join("styrene-interop-topology-v1");
    fs::create_dir_all(&lock_root)?;
    for attempt in 0..TOPOLOGY_CANDIDATES {
        let candidate =
            (topology_start_candidate(allocation_key).wrapping_add(attempt)) % TOPOLOGY_CANDIDATES;
        let base = 30_000 + candidate * 4;
        let lock_path = lock_root.join(format!("{base}"));
        match fs::create_dir(&lock_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
        match bind_topology(base) {
            Ok(listeners) => {
                let ports = [
                    ("rust_rpc".to_string(), base),
                    ("rust_transport".to_string(), base + 1),
                    ("python_shared_instance".to_string(), base + 2),
                    ("python_instance_control".to_string(), base + 3),
                ]
                .into_iter()
                .collect();
                return Ok(TopologyReservation {
                    evidence: TopologyEvidence {
                        allocation_key: format!("fnv1a64:{hash:016x}"),
                        candidate,
                        host: "127.0.0.1".to_string(),
                        ports,
                        reservation_invariant: "cooperative lock held through readiness and finalization; bound sockets validate allocation before handoff but cannot exclude arbitrary external binders during handoff".to_string(),
                    },
                    listeners,
                    lock_path,
                    released: false,
                });
            }
            Err(_) => {
                let _ = fs::remove_dir(&lock_path);
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::AddrInUse, "no interop topology block available"))
}

pub fn topology_start_candidate(allocation_key: &str) -> u16 {
    (stable_hash(allocation_key) as u16) % TOPOLOGY_CANDIDATES
}

pub fn topology_start_ports(allocation_key: &str) -> [u16; 4] {
    let base = 30_000 + topology_start_candidate(allocation_key) * 4;
    [base, base + 1, base + 2, base + 3]
}

fn bind_topology(base: u16) -> io::Result<Vec<TcpListener>> {
    let mut listeners = Vec::with_capacity(4);
    for port in base..=base + 3 {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))?;
        listener.set_nonblocking(true)?;
        listeners.push(listener);
    }
    Ok(listeners)
}

fn handoff_topology_with_retry(
    mut topology: TopologyReservation,
    allocation_key: &str,
) -> io::Result<TopologyReservation> {
    for attempt in 0..TOPOLOGY_HANDOFF_RETRIES {
        match topology.handoff_to_child() {
            Ok(()) => return Ok(topology),
            Err(error) if attempt + 1 == TOPOLOGY_HANDOFF_RETRIES => {
                let _ = topology.release();
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("topology handoff remained unavailable after bounded retries: {error}"),
                ));
            }
            Err(_) => {
                let _ = topology.release();
                topology = reserve_topology(allocation_key)?;
            }
        }
    }
    unreachable!("topology handoff loop always returns")
}

pub fn python_lxmf_scenario(
    repo_root: &Path,
    scenario_id: PinnedScenarioId,
    timeout: Duration,
    python_bin: &str,
) -> LiveScenario {
    let bidirectional = scenario_id.is_bidirectional();
    let scenario_id = scenario_id.as_str();
    let correlation_id = format!(
        "interop-{scenario_id}-{}-{}",
        std::process::id(),
        RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let python_probe = r#"import pathlib,subprocess,sys
module=__import__(sys.argv[1]); path=pathlib.Path(module.__file__).resolve()
root=next((p for p in (path.parent,*path.parents) if (p/'.git').exists()),None)
if root is None: raise SystemExit('module is not from a Git checkout')
print(subprocess.check_output(['git','-C',str(root),'rev-parse','HEAD'],text=True).strip())"#;
    let revision_probes = vec![
        RevisionProbe {
            name: "python-rns".to_string(),
            expected: Some("b48b96e61676504e0a4e527b33b9a0b4495c6872".to_string()),
            program: PathBuf::from(python_bin),
            args: vec!["-c".to_string(), python_probe.to_string(), "RNS".to_string()],
            env: BTreeMap::new(),
            worktree: None,
            timeout: PROBE_TIMEOUT,
        },
        RevisionProbe {
            name: "python-lxmf".to_string(),
            expected: Some("795fdaa2b0777c13033787d933d1afc94a2377cb".to_string()),
            program: PathBuf::from(python_bin),
            args: vec!["-c".to_string(), python_probe.to_string(), "LXMF".to_string()],
            env: BTreeMap::new(),
            worktree: None,
            timeout: PROBE_TIMEOUT,
        },
        RevisionProbe {
            name: "styrene-rs".to_string(),
            expected: std::env::var("GITHUB_SHA").ok(),
            program: PathBuf::from("git"),
            args: vec![
                "-C".to_string(),
                repo_root.display().to_string(),
                "rev-parse".to_string(),
                "HEAD".to_string(),
            ],
            env: BTreeMap::new(),
            worktree: Some(repo_root.to_path_buf()),
            timeout: PROBE_TIMEOUT,
        },
    ];
    let runner_timeout_secs = timeout.as_secs();
    let phase_budget_secs =
        runner_timeout_secs.saturating_sub(10).checked_div(5).unwrap_or(0).max(1);
    let remote_status_timeout_secs = phase_budget_secs.min(10);
    LiveScenario {
        id: scenario_id.to_string(),
        correlation_id: correlation_id.clone(),
        revision_probes,
        program: PathBuf::from("bash"),
        args: vec![
            repo_root.join("scripts/python-lxmf-smoke.sh").display().to_string(),
            "--scenario".to_string(),
            scenario_id.to_string(),
        ],
        env: BTreeMap::from([
            ("PYTHON_BIN".to_string(), python_bin.to_string()),
            ("TIMEOUT_SECS".to_string(), phase_budget_secs.to_string()),
            ("SENDER_WAIT_SECS".to_string(), phase_budget_secs.to_string()),
            ("REMOTE_STATUS_TIMEOUT_SECS".to_string(), remote_status_timeout_secs.to_string()),
        ]),
        timeout,
        required_milestones: if bidirectional {
            vec![
                "topology-configured".to_string(),
                "rust-ready".to_string(),
                "python-ready".to_string(),
                "python-message-sent".to_string(),
                "rust-message-persisted".to_string(),
                "rust-message-sent".to_string(),
                "python-message-received".to_string(),
                "child-cleanup-complete".to_string(),
            ]
        } else {
            vec![
                "topology-configured".to_string(),
                "rust-ready".to_string(),
                "python-ready".to_string(),
                "python-message-sent".to_string(),
                "rust-message-persisted".to_string(),
                "child-cleanup-complete".to_string(),
            ]
        },
        required_assertions: if bidirectional {
            vec!["python-to-rust-content".to_string(), "rust-to-python-content".to_string()]
        } else {
            vec!["python-to-rust-propagation-item".to_string()]
        },
        required_artifacts: if bidirectional {
            vec![
                "scenario-report".to_string(),
                "datastore-proof".to_string(),
                "rust-outbound-proof".to_string(),
                "rust-daemon-log".to_string(),
                "python-daemon-log".to_string(),
            ]
        } else {
            vec![
                "scenario-report".to_string(),
                "datastore-proof".to_string(),
                "rust-daemon-log".to_string(),
                "python-daemon-log".to_string(),
            ]
        },
        max_log_bytes: 64 * 1024,
        max_artifact_bytes: 2 * 1024 * 1024,
        max_artifacts: 16,
        max_artifact_total_bytes: 8 * 1024 * 1024,
        evidence_dir: repo_root.join("target/interop/runs"),
    }
}

pub fn run_live_scenario(scenario: &LiveScenario) -> io::Result<RunEvidence> {
    run_live_scenario_cancellable(scenario, &CancellationHandle::default())
}

pub fn run_live_scenario_cancellable(
    scenario: &LiveScenario,
    cancellation: &CancellationHandle,
) -> io::Result<RunEvidence> {
    let started = Instant::now();
    let run_root = create_run_root()?;
    let retained_dir = scenario.evidence_dir.join(format!(
        "{}-{:016x}",
        safe_name(&scenario.correlation_id),
        stable_hash(&scenario.correlation_id)
    ));
    if let Err(error) = fs::create_dir_all(&retained_dir) {
        let _ = fs::remove_dir_all(&run_root);
        return Err(error);
    }
    let mut topology = match reserve_topology(&scenario.correlation_id) {
        Ok(topology) => topology,
        Err(error) => {
            let _ = fs::remove_dir_all(&run_root);
            return Err(error);
        }
    };
    let mut topology_evidence = topology.evidence().clone();
    let revisions = match attest_revisions(&scenario.revision_probes, cancellation) {
        Ok(revisions) => revisions,
        Err(error) => {
            let _ = topology.release();
            let _ = fs::remove_dir_all(&run_root);
            return Err(error);
        }
    };
    let revision_failure = revisions.iter().find(|revision| !revision.matches).map(|revision| {
        revision.error.clone().unwrap_or_else(|| {
            format!(
                "revision '{}' mismatch: expected {}, actual {}",
                revision.name,
                revision.expected.as_deref().unwrap_or("any"),
                revision.actual.as_deref().unwrap_or("unavailable")
            )
        })
    });
    if revision_failure.is_some() || cancellation.is_cancelled() {
        let revision_cleanup_complete = revisions.iter().all(|revision| revision.cleanup_complete);
        let topology_released = topology.release();
        let temp_removed = fs::remove_dir_all(&run_root).is_ok() && !run_root.exists();
        return Ok(empty_evidence(
            scenario,
            topology_evidence,
            revisions,
            if cancellation.is_cancelled() { RunStatus::Cancelled } else { RunStatus::Failed },
            revision_failure.or_else(|| Some("cancelled before process start".to_string())),
            CleanupEvidence {
                topology_reservation_released: topology_released,
                temp_resources_removed: temp_removed,
                process_group_gone: revision_cleanup_complete,
                owned_descendants_gone: Some(revision_cleanup_complete),
                pipes_drained: revision_cleanup_complete,
                reader_threads_joined: revision_cleanup_complete,
                ..CleanupEvidence::default()
            },
            started,
        ));
    }

    topology = match handoff_topology_with_retry(topology, &scenario.correlation_id) {
        Ok(reservation) => reservation,
        Err(error) => {
            let _ = fs::remove_dir_all(&run_root);
            return Err(error);
        }
    };
    topology_evidence = topology.evidence().clone();
    let mut command = Command::new(&scenario.program);
    command
        .args(&scenario.args)
        .envs(&scenario.env)
        .env("STYRENE_INTEROP_CORRELATION_ID", &scenario.correlation_id)
        .env("STYRENE_RUN_ROOT", &run_root)
        .env("LOG_DIR", run_root.join("logs"))
        .env("REPORT_PATH", run_root.join("report.json"))
        .env("RUST_RPC_PORT", topology_evidence.ports["rust_rpc"].to_string())
        .env("RUST_TRANSPORT_PORT", topology_evidence.ports["rust_transport"].to_string())
        .env(
            "PY_SHARED_INSTANCE_PORT",
            topology_evidence.ports["python_shared_instance"].to_string(),
        )
        .env(
            "PY_INSTANCE_CONTROL_PORT",
            topology_evidence.ports["python_instance_control"].to_string(),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = topology.release();
            let _ = fs::remove_dir_all(&run_root);
            return Err(error);
        }
    };
    let process_group = child.id();
    let mut owned_processes = OwnedProcesses::new(process_group);
    owned_processes.refresh();
    let (sender, receiver) = mpsc::sync_channel(32);
    let stdout_thread = spawn_reader(
        child.stdout.take().ok_or_else(|| io::Error::other("missing child stdout"))?,
        StreamKind::Stdout,
        sender.clone(),
    );
    let stderr_thread = spawn_reader(
        child.stderr.take().ok_or_else(|| io::Error::other("missing child stderr"))?,
        StreamKind::Stderr,
        sender,
    );
    let mut milestones = Vec::new();
    let mut assertions = Vec::new();
    let mut artifacts = Vec::new();
    let mut artifact_rejections = Vec::new();
    let mut event_rejections = Vec::new();
    let mut artifact_events_seen = 0_usize;
    let mut stdout_log = BoundedLog::new("stdout");
    let mut stderr_log = BoundedLog::new("stderr");
    let mut stdout_lines = EventLines::new();
    let mut stderr_lines = EventLines::new();
    let mut open_streams = 2_u8;
    let mut exit_status = None;
    let mut timed_out = false;
    let mut cancelled = false;
    let mut cleanup = CleanupEvidence::default();
    let mut final_drain_deadline = None;

    while exit_status.is_none() || open_streams > 0 {
        owned_processes.refresh();
        if !cleanup.termination_requested
            && (started.elapsed() >= scenario.timeout || cancellation.is_cancelled())
        {
            timed_out = started.elapsed() >= scenario.timeout;
            cancelled = cancellation.is_cancelled() && !timed_out;
            cleanup.termination_requested = true;
            let termination = terminate_process_group(
                process_group,
                &mut child,
                &mut exit_status,
                &mut owned_processes,
            )?;
            cleanup.kill_escalated = termination.kill_escalated;
            cleanup.process_group_gone = termination.group_gone;
            cleanup.descendant_discovery_supported = owned_processes.discovery_supported;
            cleanup.owned_descendants_gone = termination.descendants_gone;
            cleanup.remaining_owned_pids = termination.remaining_owned_pids;
            final_drain_deadline = Some(Instant::now() + FINAL_PIPE_DRAIN_GRACE);
        }
        if open_streams > 0
            && final_drain_deadline.is_some_and(|deadline| Instant::now() >= deadline)
        {
            break;
        }
        let remaining = scenario.timeout.saturating_sub(started.elapsed());
        let wait = PROCESS_POLL_INTERVAL.min(remaining.max(Duration::from_millis(1)));
        match receiver.recv_timeout(wait) {
            Ok(StreamMessage::Chunk(kind, chunk)) => match kind {
                StreamKind::Stdout => {
                    stdout_log.push(&chunk, scenario.max_log_bytes);
                    stdout_lines.push(&chunk, |line| {
                        record_event(
                            line,
                            started,
                            &run_root,
                            &retained_dir,
                            scenario,
                            &mut milestones,
                            &mut assertions,
                            &mut artifacts,
                            &mut artifact_rejections,
                            &mut artifact_events_seen,
                            &mut event_rejections,
                        );
                    });
                }
                StreamKind::Stderr => {
                    stderr_log.push(&chunk, scenario.max_log_bytes);
                    stderr_lines.push(&chunk, |line| {
                        record_event(
                            line,
                            started,
                            &run_root,
                            &retained_dir,
                            scenario,
                            &mut milestones,
                            &mut assertions,
                            &mut artifacts,
                            &mut artifact_rejections,
                            &mut artifact_events_seen,
                            &mut event_rejections,
                        );
                    });
                }
            },
            Ok(StreamMessage::Closed) => open_streams = open_streams.saturating_sub(1),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => open_streams = 0,
        }
        if exit_status.is_none() {
            exit_status = child.try_wait()?;
        }
    }
    cleanup.pipes_drained = open_streams == 0;
    let reader_join_deadline = Instant::now() + Duration::from_millis(100);
    while (!stdout_thread.is_finished() || !stderr_thread.is_finished())
        && Instant::now() < reader_join_deadline
    {
        thread::sleep(Duration::from_millis(1));
    }
    cleanup.reader_threads_joined = stdout_thread.is_finished() && stderr_thread.is_finished();
    if cleanup.reader_threads_joined {
        stdout_thread.join().map_err(|_| io::Error::other("stdout reader panicked"))?;
        stderr_thread.join().map_err(|_| io::Error::other("stderr reader panicked"))?;
    }
    if exit_status.is_none() {
        exit_status = child.try_wait()?;
    }
    cleanup.direct_process_reaped = exit_status.is_some();
    owned_processes.refresh();
    if !cleanup.process_group_gone || !owned_processes.remaining().is_empty() {
        if process_group_exists(process_group)? || !owned_processes.remaining().is_empty() {
            cleanup.termination_requested = true;
            let termination = terminate_process_group(
                process_group,
                &mut child,
                &mut exit_status,
                &mut owned_processes,
            )?;
            cleanup.kill_escalated |= termination.kill_escalated;
            cleanup.process_group_gone = termination.group_gone;
            cleanup.owned_descendants_gone = termination.descendants_gone;
            cleanup.remaining_owned_pids = termination.remaining_owned_pids;
        } else {
            cleanup.process_group_gone = true;
        }
    }
    cleanup.descendant_discovery_supported = owned_processes.discovery_supported;
    if cleanup.owned_descendants_gone.is_none() && owned_processes.discovery_supported {
        let remaining = owned_processes.remaining();
        cleanup.owned_descendants_gone = Some(remaining.is_empty());
        cleanup.remaining_owned_pids = remaining;
    }

    let missing_milestones = missing_names(
        &scenario.required_milestones,
        milestones.iter().map(|item| item.name.as_str()),
    );
    let milestone_protocol_error = if missing_milestones.is_empty() {
        required_milestone_protocol_error(&scenario.required_milestones, &milestones)
    } else {
        None
    };
    let missing_assertions = missing_names(
        &scenario.required_assertions,
        assertions.iter().filter(|item| item.passed).map(|item| item.name.as_str()),
    );
    let missing_artifacts = missing_names(
        &scenario.required_artifacts,
        artifacts.iter().map(|item| item.name.as_str()),
    );
    let artifact_semantic_error = if missing_artifacts.is_empty() {
        validate_protocol_artifacts(scenario, &artifacts)
    } else {
        None
    };
    let process_exit = exit_status.and_then(|status| status.code());
    let mut failure = if timed_out {
        Some(format!(
            "deadline exceeded after {} ms; last milestone: {}",
            scenario.timeout.as_millis(),
            milestones.last().map_or("none", |item| item.name.as_str())
        ))
    } else if cancelled {
        Some("run cancelled".to_string())
    } else if let Some(status) = exit_status.filter(|status| !status.success()) {
        Some(format!("process exited with {}", exit_description(status)))
    } else if exit_status.is_none() {
        Some("direct child could not be reaped within cleanup deadline".to_string())
    } else if !event_rejections.is_empty() {
        Some(format!("{} structured event(s) rejected", event_rejections.len()))
    } else if !artifact_rejections.is_empty() {
        Some(format!("{} artifact(s) rejected", artifact_rejections.len()))
    } else if !missing_milestones.is_empty() {
        Some(format!("missing protocol milestones: {}", missing_milestones.join(", ")))
    } else if let Some(error) = milestone_protocol_error {
        Some(error)
    } else if !missing_assertions.is_empty() {
        Some(format!("missing passing assertions: {}", missing_assertions.join(", ")))
    } else if !missing_artifacts.is_empty() {
        Some(format!("missing retained artifacts: {}", missing_artifacts.join(", ")))
    } else if let Some(error) = artifact_semantic_error {
        Some(error)
    } else if let Some(item) = assertions.iter().find(|item| !item.passed) {
        Some(format!("assertion '{}' failed", item.name))
    } else if !cleanup.process_group_gone {
        Some("owned process group remains alive".to_string())
    } else if cleanup.owned_descendants_gone == Some(false) {
        Some(format!("owned descendant processes remain alive: {:?}", cleanup.remaining_owned_pids))
    } else if !cleanup.pipes_drained || !cleanup.reader_threads_joined {
        Some("owned output pipes did not drain within cleanup deadline".to_string())
    } else {
        None
    };
    let mut run_status = if timed_out {
        RunStatus::TimedOut
    } else if cancelled {
        RunStatus::Cancelled
    } else if failure.is_some() {
        RunStatus::Failed
    } else {
        RunStatus::Passed
    };
    cleanup.topology_reservation_released = topology.release();
    cleanup.temp_resources_removed = fs::remove_dir_all(&run_root).is_ok() && !run_root.exists();
    if (!cleanup.temp_resources_removed || !cleanup.topology_reservation_released)
        && run_status == RunStatus::Passed
    {
        run_status = RunStatus::Failed;
        failure = Some("owned resource cleanup was incomplete".to_string());
    }
    let mut timings_ms = BTreeMap::new();
    timings_ms.insert("total".to_string(), elapsed_ms(started));
    Ok(RunEvidence {
        schema_version: 3,
        scenario_id: scenario.id.clone(),
        correlation_id: scenario.correlation_id.clone(),
        status: run_status,
        topology: topology_evidence,
        revisions,
        milestones,
        assertions,
        artifacts,
        artifact_rejections,
        event_rejections,
        timings_ms,
        logs: vec![stdout_log.finish(), stderr_log.finish()],
        cleanup,
        process_exit,
        failure,
    })
}

fn validate_protocol_artifacts(
    scenario: &LiveScenario,
    artifacts: &[ArtifactEvidence],
) -> Option<String> {
    pinned_scenario(&scenario.id)?;
    let load = |name: &str| -> Result<serde_json::Value, String> {
        let artifact = artifacts
            .iter()
            .find(|artifact| artifact.name == name)
            .ok_or_else(|| format!("missing retained {name}"))?;
        let bytes = fs::read(&artifact.retained_path)
            .map_err(|error| format!("cannot read retained {name}: {error}"))?;
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid retained {name}: {error}"))
    };
    let report = match load("scenario-report") {
        Ok(report) => report,
        Err(error) => return Some(error),
    };
    if json_string_field(&report, "status") != "pass"
        || json_string_field(&report, "scenario") != scenario.id
        || json_string_field(&report, "correlation_id") != scenario.correlation_id
    {
        return Some("retained scenario report does not match this passing run".to_string());
    }
    if let Some(scenario_id) = pinned_scenario(&scenario.id).map(|definition| definition.id)
        && json_string_field(&report["proof"], "python_message_representation")
            != scenario_id.expected_python_representation()
    {
        return Some(
            "retained scenario report records an unexpected Python representation".to_string(),
        );
    }

    let proof = match load("datastore-proof") {
        Ok(proof) => proof,
        Err(error) => return Some(error),
    };
    if json_string_field(&proof, "scenario") != scenario.id
        || json_string_field(&proof, "correlation_id") != scenario.correlation_id
    {
        return Some("retained datastore proof does not match this run".to_string());
    }
    let expected = &proof["expected_hashes"];
    let selected = &proof["selected_row"];
    let report_proof = &report["proof"];
    let matches = if scenario.id == PinnedScenarioId::PropagatedResourceLxm.as_str() {
        json_string_field(expected, "destination") == json_string_field(selected, "destination")
            && json_string_field(expected, "transient_id")
                == json_string_field(selected, "transient_id")
            && json_string_field(expected, "transient_id")
                == json_string_field(report_proof, "python_message_transient_id")
            && json_string_field(selected, "state") == "queued"
            && selected
                .get("stored_size")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|size| size > 32)
    } else {
        json_string_field(expected, "destination") == json_string_field(selected, "destination")
            && json_string_field(expected, "source") == json_string_field(selected, "source")
            && json_string_field(expected, "message_id") == json_string_field(selected, "id")
            && json_string_field(expected, "message_id")
                == json_string_field(report_proof, "python_message_id")
            && json_string_field(&proof, "expected_content")
                == json_string_field(selected, "content")
            && json_string_field(&proof, "expected_content")
                == json_string_field(report_proof, "python_to_rust_inbound_content")
            && json_string_field(selected, "direction") == "in"
    };
    if !matches {
        return Some("retained datastore proof row does not match expected values".to_string());
    }
    let scenario_id = pinned_scenario(&scenario.id).map(|definition| definition.id)?;
    if !scenario_id.is_bidirectional() {
        return None;
    }
    let outbound = match load("rust-outbound-proof") {
        Ok(outbound) => outbound,
        Err(error) => return Some(error),
    };
    if json_string_field(&outbound, "scenario") != scenario.id
        || json_string_field(&outbound, "correlation_id") != scenario.correlation_id
    {
        return Some("retained Rust outbound proof does not match this run".to_string());
    }
    let expected = &outbound["expected_hashes"];
    let selected = &outbound["selected_row"];
    let receipt = &outbound["python_receipt"];
    let route = &outbound["route"];
    let message_id = json_string_field(expected, "message_id");
    let content = json_string_field(&outbound, "expected_content");
    let route_state = json_string_field(route, "state");
    // The Python peer proves both direct and opportunistic deliveries, so the
    // Rust route must be delivered for every bidirectional scenario, and it
    // must have used the representation the scenario exists to prove.
    let route_accepted = route_state == "delivered"
        && json_string_field(route, "representation")
            == scenario_id.expected_outbound_representation();
    let matches = !message_id.is_empty()
        && !content.is_empty()
        && json_string_field(expected, "destination") == json_string_field(selected, "destination")
        && json_string_field(expected, "source") == json_string_field(selected, "source")
        && message_id == json_string_field(selected, "id")
        && content == json_string_field(selected, "content")
        && json_string_field(selected, "direction") == "out"
        && json_string_field(receipt, "content") == content
        && json_string_field(receipt, "message_id") == message_id
        && json_string_field(receipt, "source") == json_string_field(expected, "source")
        && json_string_field(receipt, "destination") == json_string_field(expected, "destination")
        && receipt.get("signature_validated").and_then(serde_json::Value::as_bool) == Some(true)
        && json_string_field(report_proof, "rust_message_id") == message_id
        && json_string_field(report_proof, "rust_to_python_outbound_content") == content
        && json_string_field(report_proof, "rust_outbound_state") == route_state
        && route_accepted;
    (!matches).then(|| "retained Rust outbound proof does not match expected values".to_string())
}

fn json_string_field<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value.get(key).and_then(serde_json::Value::as_str).unwrap_or_default()
}

fn empty_evidence(
    scenario: &LiveScenario,
    topology: TopologyEvidence,
    revisions: Vec<RevisionEvidence>,
    status: RunStatus,
    failure: Option<String>,
    cleanup: CleanupEvidence,
    started: Instant,
) -> RunEvidence {
    RunEvidence {
        schema_version: 3,
        scenario_id: scenario.id.clone(),
        correlation_id: scenario.correlation_id.clone(),
        status,
        topology,
        revisions,
        milestones: Vec::new(),
        assertions: Vec::new(),
        artifacts: Vec::new(),
        artifact_rejections: Vec::new(),
        event_rejections: Vec::new(),
        timings_ms: BTreeMap::from([("total".to_string(), elapsed_ms(started))]),
        logs: vec![BoundedLog::new("stdout").finish(), BoundedLog::new("stderr").finish()],
        cleanup,
        process_exit: None,
        failure,
    }
}

fn attest_revisions(
    probes: &[RevisionProbe],
    cancellation: &CancellationHandle,
) -> io::Result<Vec<RevisionEvidence>> {
    let mut evidence = Vec::with_capacity(probes.len());
    for probe in probes {
        if cancellation.is_cancelled() {
            break;
        }
        let result = run_bounded_command(&probe.program, &probe.args, &probe.env, probe.timeout);
        let (actual, mut error) = match result {
            Ok(output) => (Some(output.trim().to_string()), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let worktree_dirty = match &probe.worktree {
            Some(path) => {
                let args = vec![
                    "-C".to_string(),
                    path.display().to_string(),
                    "status".to_string(),
                    "--porcelain".to_string(),
                    "--untracked-files=normal".to_string(),
                ];
                match run_bounded_command(Path::new("git"), &args, &BTreeMap::new(), PROBE_TIMEOUT)
                {
                    Ok(output) => Some(!output.trim().is_empty()),
                    Err(probe_error)
                        if probe_error.kind() == io::ErrorKind::InvalidData
                            && probe_error.to_string() == "revision probe output too large" =>
                    {
                        Some(true)
                    }
                    Err(probe_error) => {
                        error.get_or_insert_with(|| probe_error.to_string());
                        None
                    }
                }
            }
            None => None,
        };
        let matches = error.is_none()
            && probe.expected.as_deref().is_none_or(|expected| actual.as_deref() == Some(expected));
        let cleanup_complete =
            !error.as_deref().is_some_and(|message| message.starts_with("cleanup incomplete:"));
        evidence.push(RevisionEvidence {
            name: probe.name.clone(),
            expected: probe.expected.clone(),
            actual,
            matches,
            worktree_dirty,
            error,
            cleanup_complete,
        });
    }
    Ok(evidence)
}

fn run_bounded_command(
    program: &Path,
    args: &[String],
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> io::Result<String> {
    let mut command = Command::new(program);
    command.args(args).envs(env).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let process_group = child.id();
    let mut owned_processes = OwnedProcesses::new(process_group);
    owned_processes.refresh();
    let stdout = child.stdout.take().ok_or_else(|| io::Error::other("missing probe stdout"))?;
    let (sender, receiver) = mpsc::sync_channel(8);
    let reader = spawn_reader(stdout, StreamKind::Stdout, sender);
    let started = Instant::now();
    let mut output = Vec::new();
    let mut status = None;
    let mut stream_open = true;
    let mut drain_deadline = None;
    while status.is_none() || stream_open {
        owned_processes.refresh();
        if drain_deadline.is_none() && started.elapsed() >= timeout {
            let _ = terminate_process_group(
                process_group,
                &mut child,
                &mut status,
                &mut owned_processes,
            );
            drain_deadline = Some(Instant::now() + FINAL_PIPE_DRAIN_GRACE);
        }
        if stream_open && drain_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        match receiver.recv_timeout(PROCESS_POLL_INTERVAL) {
            Ok(StreamMessage::Chunk(_, chunk)) => {
                let remaining = MAX_PROBE_BYTES.saturating_sub(output.len());
                output.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            }
            Ok(StreamMessage::Closed) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                stream_open = false;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if status.is_none() {
            status = child.try_wait()?;
        }
    }
    let reader_join_deadline = Instant::now() + Duration::from_millis(100);
    while !reader.is_finished() && Instant::now() < reader_join_deadline {
        thread::sleep(Duration::from_millis(1));
    }
    if stream_open || !reader.is_finished() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "cleanup incomplete: revision probe retained its output pipe past the drain deadline",
        ));
    }
    reader.join().map_err(|_| io::Error::other("revision probe reader panicked"))?;
    if status.is_none() {
        status = child.try_wait()?;
    }
    let remaining_pids = owned_processes.remaining();
    if !remaining_pids.is_empty() || process_group_exists(process_group)? {
        return Err(io::Error::other(format!(
            "cleanup incomplete: revision probe left processes alive: {remaining_pids:?}"
        )));
    }
    let status = status.ok_or_else(|| {
        io::Error::other("cleanup incomplete: revision probe child was not reaped")
    })?;
    if started.elapsed() >= timeout {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "revision probe timed out"));
    }
    if !status.success() {
        return Err(io::Error::other(format!(
            "revision probe exited with {}",
            exit_description(status)
        )));
    }
    if output.len() == MAX_PROBE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "revision probe output too large"));
    }
    String::from_utf8(output).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "revision probe output was not UTF-8")
    })
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    kind: StreamKind,
    sender: SyncSender<StreamMessage>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if sender.send(StreamMessage::Chunk(kind, buffer[..count].to_vec())).is_err() {
                        return;
                    }
                }
            }
        }
        let _ = sender.send(StreamMessage::Closed);
    })
}

#[allow(clippy::too_many_arguments)]
fn record_event(
    line: &[u8],
    started: Instant,
    run_root: &Path,
    retained_dir: &Path,
    scenario: &LiveScenario,
    milestones: &mut Vec<TimedEvidence>,
    assertions: &mut Vec<AssertionEvidence>,
    artifacts: &mut Vec<ArtifactEvidence>,
    artifact_rejections: &mut Vec<ArtifactRejection>,
    artifact_events_seen: &mut usize,
    event_rejections: &mut Vec<String>,
) {
    let Ok(line) = std::str::from_utf8(line) else { return };
    let Some(payload) = line.strip_prefix(EVENT_PREFIX) else { return };
    let Ok(event) = serde_json::from_str::<RunnerEvent>(payload) else { return };
    if event.correlation_id.as_deref() != Some(scenario.correlation_id.as_str()) {
        event_rejections.push(format!(
            "event '{}' correlation mismatch: expected '{}', observed '{}'",
            event.name,
            scenario.correlation_id,
            event.correlation_id.as_deref().unwrap_or("missing")
        ));
        return;
    }
    match event.kind.as_str() {
        "milestone" => {
            milestones.push(TimedEvidence { name: event.name, elapsed_ms: elapsed_ms(started) })
        }
        "assertion" => assertions.push(AssertionEvidence {
            name: event.name,
            passed: event.passed.unwrap_or(false),
            detail: event.detail,
            elapsed_ms: elapsed_ms(started),
        }),
        "artifact" => {
            let path = event.path.unwrap_or_default();
            if *artifact_events_seen >= scenario.max_artifacts {
                if !artifact_rejections
                    .iter()
                    .any(|item| item.reason == "aggregate artifact count limit exceeded")
                {
                    artifact_rejections.push(ArtifactRejection {
                        name: event.name,
                        path,
                        reason: "aggregate artifact count limit exceeded".to_string(),
                    });
                }
                return;
            }
            *artifact_events_seen += 1;
            let retained_bytes = artifacts.iter().map(|item| item.bytes).sum::<u64>();
            let aggregate_remaining =
                scenario.max_artifact_total_bytes.saturating_sub(retained_bytes);
            match retain_artifact(
                run_root,
                retained_dir,
                &event.name,
                &path,
                scenario.max_artifact_bytes,
                aggregate_remaining,
                artifacts.len(),
            ) {
                Ok(artifact) => artifacts.push(artifact),
                Err(reason) => {
                    artifact_rejections.push(ArtifactRejection { name: event.name, path, reason })
                }
            }
        }
        _ => {}
    }
}

fn retain_artifact(
    run_root: &Path,
    evidence_dir: &Path,
    name: &str,
    path: &str,
    max_bytes: u64,
    aggregate_remaining: u64,
    index: usize,
) -> Result<ArtifactEvidence, String> {
    let candidate = Path::new(path);
    let candidate =
        if candidate.is_absolute() { candidate.to_path_buf() } else { run_root.join(candidate) };
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("symbolic links are not accepted".to_string());
    }
    if !metadata.file_type().is_file() {
        return Err("artifact is not a regular file".to_string());
    }
    let canonical_root = run_root.canonicalize().map_err(|error| error.to_string())?;
    let canonical = candidate.canonicalize().map_err(|error| error.to_string())?;
    if !canonical.starts_with(&canonical_root) {
        return Err("artifact is outside the owned run root".to_string());
    }
    if metadata.len() > max_bytes {
        return Err(format!("artifact exceeds {max_bytes} byte limit"));
    }
    if metadata.len() > aggregate_remaining {
        return Err(format!(
            "artifact exceeds remaining aggregate byte limit of {aggregate_remaining}"
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(&canonical).map_err(|error| error.to_string())?;
    let opened_metadata = file.metadata().map_err(|error| error.to_string())?;
    if !opened_metadata.file_type().is_file()
        || opened_metadata.len() > max_bytes
        || opened_metadata.len() > aggregate_remaining
    {
        return Err("artifact changed while being retained".to_string());
    }
    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    let read_limit = max_bytes.min(aggregate_remaining);
    file.take(read_limit + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("artifact exceeds {max_bytes} byte limit"));
    }
    if bytes.len() as u64 > aggregate_remaining {
        return Err("aggregate artifact byte limit exceeded".to_string());
    }
    let artifact_dir = evidence_dir.join("artifacts");
    fs::create_dir_all(&artifact_dir).map_err(|error| error.to_string())?;
    let retained = artifact_dir.join(format!("{index:03}-{}", safe_name(name)));
    fs::write(&retained, &bytes).map_err(|error| error.to_string())?;
    Ok(ArtifactEvidence {
        name: name.to_string(),
        source_path: canonical
            .strip_prefix(&canonical_root)
            .unwrap_or(&canonical)
            .display()
            .to_string(),
        retained_path: retained.display().to_string(),
        bytes: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(&bytes)),
    })
}

fn safe_name(name: &str) -> String {
    let value: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || "-_.".contains(character) {
                character
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    if value.is_empty() { "artifact".to_string() } else { value }
}

struct TerminationResult {
    kill_escalated: bool,
    group_gone: bool,
    descendants_gone: Option<bool>,
    remaining_owned_pids: Vec<u32>,
}

struct OwnedProcesses {
    root: u32,
    descendants: BTreeSet<u32>,
    discovery_supported: bool,
}

impl OwnedProcesses {
    fn new(root: u32) -> Self {
        Self { root, descendants: BTreeSet::new(), discovery_supported: true }
    }

    fn refresh(&mut self) {
        let Ok(processes) = process_snapshot() else {
            self.discovery_supported = false;
            return;
        };
        let mut owned = self.descendants.clone();
        owned.insert(self.root);
        loop {
            let before = owned.len();
            for process in &processes {
                if process.group == self.root || owned.contains(&process.parent) {
                    owned.insert(process.pid);
                }
            }
            if owned.len() == before {
                break;
            }
        }
        owned.remove(&self.root);
        self.descendants.extend(owned);
    }

    fn signal_descendants(&self, signal: &str) {
        for pid in &self.descendants {
            let _ = signal_pid(*pid, signal);
        }
    }

    fn remaining(&self) -> Vec<u32> {
        self.descendants.iter().copied().filter(|pid| pid_exists(*pid)).collect()
    }
}

struct ProcessRecord {
    pid: u32,
    parent: u32,
    group: u32,
}

fn terminate_process_group(
    process_group: u32,
    child: &mut Child,
    exit_status: &mut Option<ExitStatus>,
    owned: &mut OwnedProcesses,
) -> io::Result<TerminationResult> {
    owned.refresh();
    let _ = signal_process_group(process_group, "-TERM");
    owned.signal_descendants("-TERM");
    let graceful_deadline = Instant::now() + TERMINATION_GRACE;
    while Instant::now() < graceful_deadline {
        if exit_status.is_none() {
            *exit_status = child.try_wait()?;
        }
        owned.refresh();
        let remaining = owned.remaining();
        if !process_group_exists(process_group)? && remaining.is_empty() {
            return Ok(TerminationResult {
                kill_escalated: false,
                group_gone: true,
                descendants_gone: owned.discovery_supported.then_some(true),
                remaining_owned_pids: remaining,
            });
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    }
    let _ = signal_process_group(process_group, "-KILL");
    owned.signal_descendants("-KILL");
    let kill_deadline = Instant::now() + TERMINATION_GRACE;
    while Instant::now() < kill_deadline {
        if exit_status.is_none() {
            *exit_status = child.try_wait()?;
        }
        owned.refresh();
        owned.signal_descendants("-KILL");
        let remaining = owned.remaining();
        if !process_group_exists(process_group)? && remaining.is_empty() {
            return Ok(TerminationResult {
                kill_escalated: true,
                group_gone: true,
                descendants_gone: owned.discovery_supported.then_some(true),
                remaining_owned_pids: remaining,
            });
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    }
    let remaining = owned.remaining();
    Ok(TerminationResult {
        kill_escalated: true,
        group_gone: !process_group_exists(process_group)?,
        descendants_gone: owned.discovery_supported.then_some(remaining.is_empty()),
        remaining_owned_pids: remaining,
    })
}

fn process_snapshot() -> io::Result<Vec<ProcessRecord>> {
    let mut child = Command::new("ps")
        .args(["-axo", "pid=,ppid=,pgid="])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| io::Error::other("missing ps stdout"))?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(MAX_PROCESS_SNAPSHOT_BYTES as u64 + 1).read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait()?;
            return Err(io::Error::new(io::ErrorKind::TimedOut, "process snapshot timed out"));
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    };
    let reader_join_deadline = Instant::now() + Duration::from_millis(100);
    while !reader.is_finished() && Instant::now() < reader_join_deadline {
        thread::sleep(Duration::from_millis(1));
    }
    if !status.success() || !reader.is_finished() {
        return Err(io::Error::other("process snapshot failed"));
    }
    let bytes =
        reader.join().map_err(|_| io::Error::other("process snapshot reader panicked"))??;
    if bytes.len() > MAX_PROCESS_SNAPSHOT_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "process snapshot too large"));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "process snapshot was not UTF-8")
    })?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some(ProcessRecord {
                pid: fields.next()?.parse().ok()?,
                parent: fields.next()?.parse().ok()?,
                group: fields.next()?.parse().ok()?,
            })
        })
        .collect())
}

fn signal_pid(pid: u32, signal: &str) -> io::Result<()> {
    let status = Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() || !pid_exists(pid) {
        Ok(())
    } else {
        Err(io::Error::other(format!("failed to signal process {pid}")))
    }
}

fn pid_exists(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Signal every process in a group.
///
/// The negative group id follows `--`: BSD `kill` accepts it either way, but
/// procps `kill` on Linux misparses a bare `-PGID` as an option, so the plain
/// form silently fails to signal and misreports liveness.
fn signal_process_group(process_group: u32, signal: &str) -> io::Result<()> {
    let status = Command::new("kill")
        .arg(signal)
        .arg("--")
        .arg(format!("-{process_group}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() || !process_group_exists(process_group)? {
        Ok(())
    } else {
        Err(io::Error::other(format!("failed to signal process group {process_group}")))
    }
}

fn process_group_exists(process_group: u32) -> io::Result<bool> {
    Ok(Command::new("kill")
        .arg("-0")
        .arg("--")
        .arg(format!("-{process_group}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success())
}

fn missing_names<'a>(
    required: &[String],
    observed: impl Iterator<Item = &'a str> + Clone,
) -> Vec<String> {
    required
        .iter()
        .filter(|required| !observed.clone().any(|observed| observed == required.as_str()))
        .cloned()
        .collect()
}

fn required_milestone_protocol_error(
    required: &[String],
    observed: &[TimedEvidence],
) -> Option<String> {
    for name in required {
        let count = observed.iter().filter(|item| item.name == *name).count();
        if count != 1 {
            return Some(format!("required milestone '{name}' observed {count} times"));
        }
    }
    let observed_required: Vec<_> = observed
        .iter()
        .filter(|item| required.contains(&item.name))
        .map(|item| item.name.as_str())
        .collect();
    if observed_required.iter().copied().eq(required.iter().map(String::as_str)) {
        None
    } else {
        Some("required protocol milestones were emitted out of order".to_string())
    }
}

fn create_run_root() -> io::Result<PathBuf> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    for attempt in 0..100_u8 {
        let path = std::env::temp_dir().join(format!(
            "styrene-interop-{}-{now}-{}-{attempt}",
            std::process::id(),
            RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "unable to allocate interop run root"))
}

fn stable_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn exit_description(status: ExitStatus) -> String {
    status.code().map_or_else(|| "signal".to_string(), |code| code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Guards the process-group primitives against platform `kill` parsing
    /// differences: a live group must read as alive, a terminated group and a
    /// nonexistent group as gone, on every supported host.
    #[cfg(unix)]
    #[test]
    fn process_group_liveness_matches_reality_on_this_platform() {
        use std::os::unix::process::CommandExt;

        let mut child = Command::new("sleep")
            .arg("30")
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep in its own process group");
        let group = child.id();
        assert!(process_group_exists(group).unwrap(), "live process group reported missing");
        assert!(!process_group_exists(999_999_999).unwrap(), "nonexistent group reported alive");

        signal_process_group(group, "-KILL").expect("signal process group");
        let _ = child.wait();
        let deadline = Instant::now() + Duration::from_secs(2);
        while process_group_exists(group).unwrap() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!process_group_exists(group).unwrap(), "terminated group reported alive");
    }

    fn direct_scenario() -> LiveScenario {
        LiveScenario {
            id: PinnedScenarioId::Direct.as_str().to_string(),
            correlation_id: "correlation-1".to_string(),
            revision_probes: Vec::new(),
            program: PathBuf::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            timeout: Duration::from_secs(1),
            required_milestones: Vec::new(),
            required_assertions: Vec::new(),
            required_artifacts: Vec::new(),
            max_log_bytes: 1,
            max_artifact_bytes: 1,
            max_artifacts: 2,
            max_artifact_total_bytes: 2,
            evidence_dir: PathBuf::new(),
        }
    }

    fn semantic_artifacts(
        directory: &Path,
        report: &serde_json::Value,
        proof: &serde_json::Value,
    ) -> Vec<ArtifactEvidence> {
        semantic_artifacts_with_outbound(directory, report, proof, &direct_outbound_proof())
    }

    fn direct_report() -> serde_json::Value {
        json!({
            "status": "pass",
            "scenario": "direct",
            "correlation_id": "correlation-1",
            "proof": {
                "python_message_id": "message-1",
                "python_message_representation": "1",
                "python_to_rust_inbound_content": "content-1",
                "rust_message_id": "message-2",
                "rust_to_python_outbound_content": "content-2",
                "rust_outbound_state": "delivered"
            }
        })
    }

    fn direct_outbound_proof() -> serde_json::Value {
        json!({
            "scenario": "direct",
            "correlation_id": "correlation-1",
            "expected_content": "content-2",
            "expected_hashes": {
                "destination": "source-1",
                "source": "destination-1",
                "message_id": "message-2"
            },
            "expected_method": "direct",
            "python_receipt": {
                "content": "content-2",
                "destination": "source-1",
                "message_id": "message-2",
                "method": 1,
                "signature_validated": true,
                "source": "destination-1",
                "title": ""
            },
            "route": {
                "actual_method": "direct",
                "representation": "packet",
                "requested_method": "direct",
                "state": "delivered"
            },
            "selected_row": {
                "content": "content-2",
                "destination": "source-1",
                "direction": "out",
                "id": "message-2",
                "source": "destination-1"
            }
        })
    }

    fn semantic_artifacts_with_outbound(
        directory: &Path,
        report: &serde_json::Value,
        proof: &serde_json::Value,
        outbound: &serde_json::Value,
    ) -> Vec<ArtifactEvidence> {
        let report_path = directory.join("report.json");
        let proof_path = directory.join("proof.json");
        let outbound_path = directory.join("outbound.json");
        fs::write(&report_path, serde_json::to_vec(report).expect("serialize report"))
            .expect("write report");
        fs::write(&proof_path, serde_json::to_vec(proof).expect("serialize proof"))
            .expect("write proof");
        fs::write(&outbound_path, serde_json::to_vec(outbound).expect("serialize outbound"))
            .expect("write outbound proof");
        vec![
            ArtifactEvidence {
                name: "rust-outbound-proof".to_string(),
                source_path: "outbound.json".to_string(),
                retained_path: outbound_path.to_string_lossy().into_owned(),
                bytes: 0,
                sha256: String::new(),
            },
            ArtifactEvidence {
                name: "scenario-report".to_string(),
                source_path: "report.json".to_string(),
                retained_path: report_path.to_string_lossy().into_owned(),
                bytes: 0,
                sha256: String::new(),
            },
            ArtifactEvidence {
                name: "datastore-proof".to_string(),
                source_path: "proof.json".to_string(),
                retained_path: proof_path.to_string_lossy().into_owned(),
                bytes: 0,
                sha256: String::new(),
            },
        ]
    }

    #[test]
    fn semantic_validation_cross_checks_direct_message_id() {
        let directory = tempfile::tempdir().expect("temp directory");
        let report = direct_report();
        let proof = json!({
            "scenario": "direct",
            "correlation_id": "correlation-1",
            "expected_content": "content-1",
            "expected_hashes": {
                "destination": "destination-1",
                "source": "source-1",
                "message_id": "message-1"
            },
            "selected_row": {
                "content": "content-1",
                "destination": "destination-1",
                "direction": "in",
                "id": "message-1",
                "source": "source-1"
            }
        });
        let artifacts = semantic_artifacts(directory.path(), &report, &proof);
        assert_eq!(validate_protocol_artifacts(&direct_scenario(), &artifacts), None);

        let mut mismatched_report = report;
        mismatched_report["proof"]["python_message_id"] = json!("message-2");
        let artifacts = semantic_artifacts(directory.path(), &mismatched_report, &proof);
        assert!(validate_protocol_artifacts(&direct_scenario(), &artifacts).is_some());
    }

    #[test]
    fn semantic_validation_rejects_malformed_artifact_json() {
        let directory = tempfile::tempdir().expect("temp directory");
        let report_path = directory.path().join("report.json");
        fs::write(&report_path, b"not-json").expect("write malformed report");
        let artifacts = vec![ArtifactEvidence {
            name: "scenario-report".to_string(),
            source_path: "report.json".to_string(),
            retained_path: report_path.to_string_lossy().into_owned(),
            bytes: 8,
            sha256: String::new(),
        }];
        assert!(validate_protocol_artifacts(&direct_scenario(), &artifacts).is_some());
    }
}
