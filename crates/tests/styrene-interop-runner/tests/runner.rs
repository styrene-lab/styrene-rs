use std::collections::BTreeMap;
use std::fs;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use styrene_interop_runner::{
    CancellationHandle, LiveScenario, PINNED_SCENARIOS, PinnedScenarioId, RevisionProbe, RunStatus,
    pinned_scenario, python_lxmf_scenario, reserve_topology, run_live_scenario,
    run_live_scenario_cancellable, topology_start_candidate, topology_start_ports,
};
use tempfile::TempDir;

fn fake_probe(expected: &str, actual: &str) -> RevisionProbe {
    RevisionProbe {
        name: "fake-peer".to_string(),
        expected: Some(expected.to_string()),
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_string(), format!("printf %s {actual}")],
        env: BTreeMap::new(),
        worktree: None,
        timeout: Duration::from_millis(500),
    }
}

fn fake_scenario(script: &str, evidence_dir: &Path) -> LiveScenario {
    let correlation_id = format!("test-correlation-{}", evidence_dir.display());
    let event_prefix = format!("STYRENE_EVENT {{\"correlation_id\":\"{correlation_id}\",");
    let script = format!(
        "{}\nprintf '%s\\n' 'STYRENE_EVENT {{\"correlation_id\":\"{}\",\"kind\":\"milestone\",\"name\":\"child-cleanup-complete\"}}'\n",
        script.replace("STYRENE_EVENT {", &event_prefix),
        correlation_id
    );
    LiveScenario {
        id: "fake-direct".to_string(),
        correlation_id,
        revision_probes: vec![fake_probe("revision-1", "revision-1")],
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_string(), script],
        env: BTreeMap::new(),
        timeout: Duration::from_secs(2),
        required_milestones: vec![
            "peer-ready".to_string(),
            "message-received".to_string(),
            "child-cleanup-complete".to_string(),
        ],
        required_assertions: vec!["content-matched".to_string()],
        required_artifacts: vec!["message".to_string()],
        max_log_bytes: 128,
        max_artifact_bytes: 1024,
        max_artifacts: 8,
        max_artifact_total_bytes: 4096,
        evidence_dir: evidence_dir.to_path_buf(),
    }
}

#[test]
fn pinned_catalog_validates_ids_and_propagates_python_to_the_harness() {
    assert_eq!(PINNED_SCENARIOS.len(), 5);
    assert_eq!("direct".parse(), Ok(PinnedScenarioId::Direct));
    assert_eq!("propagated_retrieval".parse(), Ok(PinnedScenarioId::PropagatedRetrieval));
    assert_eq!("direct_resource".parse(), Ok(PinnedScenarioId::DirectResource));
    assert!("unknown".parse::<PinnedScenarioId>().is_err());
    assert_eq!(pinned_scenario("opportunistic").unwrap().id, PinnedScenarioId::Opportunistic);

    let scenario = python_lxmf_scenario(
        Path::new("/repo"),
        PinnedScenarioId::PropagatedResourceLxm,
        Duration::from_secs(9),
        "/pinned/python",
    );
    assert_eq!(scenario.id, "propagated_resource_lxm");
    assert_eq!(scenario.required_assertions, ["python-to-rust-propagation-item"]);
    assert_eq!(scenario.env["PYTHON_BIN"], "/pinned/python");
    assert_eq!(scenario.env["TIMEOUT_SECS"], "1");
    assert_eq!(scenario.env["SENDER_WAIT_SECS"], "1");
    assert_eq!(scenario.env["REMOTE_STATUS_TIMEOUT_SECS"], "1");
    assert!(scenario.required_artifacts.iter().any(|name| name == "datastore-proof"));
    assert_eq!(
        scenario.required_milestones.last().map(String::as_str),
        Some("child-cleanup-complete")
    );
    assert_eq!(scenario.revision_probes[0].program, PathBuf::from("/pinned/python"));
    assert!(!PinnedScenarioId::PropagatedResourceLxm.is_bidirectional());
    assert!(PinnedScenarioId::PropagatedRetrieval.is_retrieval());
    assert!(!PinnedScenarioId::PropagatedRetrieval.is_bidirectional());
    let retrieval = python_lxmf_scenario(
        Path::new("/repo"),
        PinnedScenarioId::PropagatedRetrieval,
        Duration::from_secs(300),
        "/pinned/python",
    );
    assert_eq!(
        retrieval.required_assertions,
        ["python-to-rust-propagation-item", "rust-to-python-retrieval"]
    );
    assert!(retrieval.required_milestones.iter().any(|name| name == "rust-restarted"));
    assert!(retrieval.required_artifacts.iter().any(|name| name == "rust-retrieval-proof"));
    assert!(retrieval.required_artifacts.iter().any(|name| name == "rust-daemon-restart-log"));
    assert_eq!(PinnedScenarioId::PropagatedRetrieval.expected_python_representation(), "1");
    assert!(!scenario.required_artifacts.iter().any(|name| name == "rust-outbound-proof"));

    assert_eq!(PinnedScenarioId::Direct.expected_outbound_representation(), "packet");
    assert_eq!(PinnedScenarioId::Opportunistic.expected_outbound_representation(), "packet");
    assert_eq!(PinnedScenarioId::DirectResource.expected_outbound_representation(), "resource");
    assert_eq!(PinnedScenarioId::Direct.expected_python_representation(), "1");
    assert_eq!(PinnedScenarioId::DirectResource.expected_python_representation(), "2");
    assert_eq!(PinnedScenarioId::PropagatedResourceLxm.expected_python_representation(), "2");
    for bidirectional in [
        PinnedScenarioId::Direct,
        PinnedScenarioId::DirectResource,
        PinnedScenarioId::Opportunistic,
    ] {
        assert!(bidirectional.is_bidirectional());
        let scenario = python_lxmf_scenario(
            Path::new("/repo"),
            bidirectional,
            Duration::from_secs(300),
            "/pinned/python",
        );
        assert_eq!(
            scenario.required_assertions,
            ["python-to-rust-content", "rust-to-python-content"]
        );
        let milestones: Vec<&str> =
            scenario.required_milestones.iter().map(String::as_str).collect();
        assert_eq!(
            milestones,
            [
                "topology-configured",
                "rust-ready",
                "python-ready",
                "python-message-sent",
                "rust-message-persisted",
                "rust-message-sent",
                "python-message-received",
                "child-cleanup-complete",
            ]
        );
        assert!(scenario.required_artifacts.iter().any(|name| name == "rust-outbound-proof"));
        assert!(scenario.required_artifacts.iter().any(|name| name == "datastore-proof"));
    }
}

const SUCCESS_SCRIPT: &str = r#"
printf '%s\n' 'STYRENE_EVENT {"kind":"milestone","name":"peer-ready"}'
mkdir "$STYRENE_RUN_ROOT/artifacts"
printf 'canonical payload' > "$STYRENE_RUN_ROOT/artifacts/message.bin"
printf '%s\n' 'STYRENE_EVENT {"kind":"milestone","name":"message-received"}'
printf '%s\n' 'STYRENE_EVENT {"kind":"assertion","name":"content-matched","passed":true}'
printf '%s\n' 'STYRENE_EVENT {"kind":"artifact","name":"message","path":"artifacts/message.bin"}'
"#;

#[test]
fn structured_fake_process_retains_complete_evidence_and_cleans_up() {
    let temp = TempDir::new().expect("temporary directory");
    let evidence = run_live_scenario(&fake_scenario(SUCCESS_SCRIPT, temp.path()))
        .expect("fake scenario should run");

    assert_eq!(evidence.status, RunStatus::Passed);
    assert!(evidence.event_rejections.is_empty());
    assert_eq!(evidence.revisions[0].actual.as_deref(), Some("revision-1"));
    assert_eq!(evidence.artifacts[0].bytes, 17);
    assert_eq!(
        evidence.artifacts[0].sha256,
        "4a64e29359c7d3f9be9aa5118f928b72226ff181b3123da6cea94b4ef8a1d993"
    );
    assert!(Path::new(&evidence.artifacts[0].retained_path).is_file());
    assert_eq!(
        fs::read(&evidence.artifacts[0].retained_path).expect("retained artifact"),
        b"canonical payload"
    );
    assert!(evidence.cleanup.direct_process_reaped);
    assert!(evidence.cleanup.process_group_gone);
    assert!(evidence.cleanup.temp_resources_removed);
    assert!(evidence.cleanup.topology_reservation_released);
}

#[test]
fn mismatched_event_correlation_is_rejected() {
    let temp = TempDir::new().expect("temporary directory");
    let mut scenario = fake_scenario(SUCCESS_SCRIPT, temp.path());
    scenario.args[1] = scenario.args[1].replacen(&scenario.correlation_id, "wrong-correlation", 1);

    let evidence = run_live_scenario(&scenario).expect("correlation rejection evidence");

    assert_eq!(evidence.status, RunStatus::Failed);
    assert_eq!(evidence.event_rejections.len(), 1);
    assert!(evidence.failure.as_deref().unwrap_or_default().contains("structured event"));
}

#[test]
fn required_milestones_must_be_unique_and_ordered() {
    let temp = TempDir::new().expect("temporary directory");
    let mut out_of_order = fake_scenario(SUCCESS_SCRIPT, &temp.path().join("order"));
    out_of_order.args[1] = out_of_order.args[1]
        .replacen("peer-ready", "temporary-name", 1)
        .replacen("message-received", "peer-ready", 1)
        .replacen("temporary-name", "message-received", 1);
    let evidence = run_live_scenario(&out_of_order).expect("ordering evidence");
    assert_eq!(evidence.status, RunStatus::Failed);
    assert!(evidence.failure.as_deref().unwrap_or_default().contains("out of order"));

    let mut duplicate = fake_scenario(SUCCESS_SCRIPT, &temp.path().join("duplicate"));
    let duplicate_event = format!(
        "printf '%s\\n' 'STYRENE_EVENT {{\"correlation_id\":\"{}\",\"kind\":\"milestone\",\"name\":\"peer-ready\"}}'\n",
        duplicate.correlation_id
    );
    duplicate.args[1] = format!("{duplicate_event}{}", duplicate.args[1]);
    let evidence = run_live_scenario(&duplicate).expect("uniqueness evidence");
    assert_eq!(evidence.status, RunStatus::Failed);
    assert!(evidence.failure.as_deref().unwrap_or_default().contains("observed 2 times"));
}

#[test]
fn required_datastore_proof_artifact_is_retained() {
    let temp = TempDir::new().expect("temporary directory");
    let script = SUCCESS_SCRIPT
        .replace("artifacts/message.bin", "artifacts/datastore-proof.json")
        .replace("\"name\":\"message\"", "\"name\":\"datastore-proof\"");
    let mut scenario = fake_scenario(&script, temp.path());
    scenario.required_artifacts = vec!["datastore-proof".to_string()];

    let evidence = run_live_scenario(&scenario).expect("datastore proof evidence");

    assert_eq!(evidence.status, RunStatus::Passed);
    assert_eq!(evidence.artifacts[0].name, "datastore-proof");
    assert!(Path::new(&evidence.artifacts[0].retained_path).is_file());
}

#[test]
fn successful_process_exit_is_not_protocol_success() {
    let temp = TempDir::new().expect("temporary directory");
    let evidence =
        run_live_scenario(&fake_scenario("exit 0", temp.path())).expect("fake scenario should run");
    assert_eq!(evidence.status, RunStatus::Failed);
    assert!(
        evidence.failure.as_deref().unwrap_or_default().contains("missing protocol milestones")
    );
}

#[test]
fn deadline_applies_after_parent_exit_and_removes_descendants() {
    let temp = TempDir::new().expect("temporary directory");
    let marker = temp.path().join("descendant.pid");
    let mut scenario = fake_scenario(
        "(trap '' TERM; while :; do sleep 1; done) & printf %s $! > \"$MARKER\"; exit 0",
        temp.path(),
    );
    scenario.env.insert("MARKER".to_string(), marker.display().to_string());
    scenario.timeout = Duration::from_millis(150);

    let started = Instant::now();
    let evidence = run_live_scenario(&scenario).expect("timed out process tree should return");

    assert_eq!(evidence.status, RunStatus::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(evidence.cleanup.termination_requested);
    assert!(evidence.cleanup.kill_escalated);
    assert!(evidence.cleanup.direct_process_reaped);
    assert!(evidence.cleanup.process_group_gone);
    assert!(evidence.cleanup.pipes_drained);
    assert!(evidence.cleanup.reader_threads_joined);
}

#[test]
fn escaped_reparented_descendant_is_discovered_and_cannot_retain_pipes() {
    let temp = TempDir::new().expect("temporary directory");
    let mut scenario = fake_scenario(
        "perl -MPOSIX -e 'POSIX::setsid(); sleep 30' & sleep 0.2; exit 0",
        temp.path(),
    );
    scenario.timeout = Duration::from_millis(350);

    let started = Instant::now();
    let evidence = run_live_scenario(&scenario).expect("escaped descendant evidence");

    assert_eq!(evidence.status, RunStatus::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(evidence.cleanup.descendant_discovery_supported);
    assert_eq!(evidence.cleanup.owned_descendants_gone, Some(true));
    assert!(evidence.cleanup.remaining_owned_pids.is_empty());
    assert!(evidence.cleanup.pipes_drained);
    assert!(evidence.cleanup.reader_threads_joined);
}

#[test]
fn blocked_symlink_fifo_and_oversized_artifacts_are_rejected_without_blocking() {
    let temp = TempDir::new().expect("temporary directory");
    let script = r#"
printf '%s\n' 'STYRENE_EVENT {"kind":"milestone","name":"peer-ready"}'
printf '%s\n' 'STYRENE_EVENT {"kind":"milestone","name":"message-received"}'
printf '%s\n' 'STYRENE_EVENT {"kind":"assertion","name":"content-matched","passed":true}'
printf x > "$STYRENE_RUN_ROOT/real"
ln -s real "$STYRENE_RUN_ROOT/link"
mkfifo "$STYRENE_RUN_ROOT/fifo"
dd if=/dev/zero of="$STYRENE_RUN_ROOT/large" bs=2048 count=1 2>/dev/null
printf '%s\n' 'STYRENE_EVENT {"kind":"artifact","name":"message","path":"link"}'
printf '%s\n' 'STYRENE_EVENT {"kind":"artifact","name":"fifo","path":"fifo"}'
printf '%s\n' 'STYRENE_EVENT {"kind":"artifact","name":"large","path":"large"}'
"#;
    let started = Instant::now();
    let evidence = run_live_scenario(&fake_scenario(script, temp.path()))
        .expect("invalid artifacts should produce evidence");

    assert_eq!(evidence.status, RunStatus::Failed);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(evidence.artifact_rejections.len(), 3);
    assert!(evidence.artifact_rejections.iter().any(|item| item.reason.contains("symbolic")));
    assert!(evidence.artifact_rejections.iter().any(|item| item.reason.contains("regular")));
    assert!(evidence.artifact_rejections.iter().any(|item| item.reason.contains("exceeds")));
}

#[test]
fn topology_reservations_avoid_collisions_and_reuse_deterministically() {
    let key = format!("same-allocation-{}", std::process::id());
    assert_eq!(topology_start_candidate(&key), topology_start_candidate(&key));
    let first = reserve_topology(&key).expect("first reservation");
    let first_topology = first.evidence().clone();
    let second = reserve_topology(&key).expect("colliding reservation");
    assert_ne!(first_topology.ports, second.evidence().ports);
    assert!(first_topology.reservation_invariant.contains("cannot exclude arbitrary external"));
    drop(first);
    drop(second);
}

#[test]
fn topology_allocation_retries_when_initial_block_is_bound() {
    let key = format!("occupied-topology-{}", std::process::id());
    let listeners: Vec<_> = topology_start_ports(&key)
        .into_iter()
        .map(|port| {
            TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
                .expect("occupy initial topology port")
        })
        .collect();

    let reservation = reserve_topology(&key).expect("allocator should retry bounded candidates");

    assert_ne!(reservation.evidence().ports["rust_rpc"], topology_start_ports(&key)[0]);
    drop(listeners);
}

#[test]
fn revision_mismatch_fails_before_scenario_process_starts() {
    let temp = TempDir::new().expect("temporary directory");
    let marker = temp.path().join("started");
    let mut scenario = fake_scenario("touch \"$MARKER\"", temp.path());
    scenario.env.insert("MARKER".to_string(), marker.display().to_string());
    scenario.revision_probes = vec![fake_probe("expected", "actual")];

    let evidence = run_live_scenario(&scenario).expect("mismatch should produce evidence");

    assert_eq!(evidence.status, RunStatus::Failed);
    assert!(!marker.exists());
    assert!(!evidence.revisions[0].matches);
    assert!(evidence.failure.as_deref().unwrap_or_default().contains("mismatch"));
    assert!(!evidence.cleanup.direct_process_reaped);
    assert!(evidence.cleanup.process_group_gone);
    assert!(evidence.cleanup.temp_resources_removed);
    assert!(evidence.cleanup.topology_reservation_released);
}

#[test]
fn revision_attestation_records_actual_commit_and_dirty_worktree() {
    let temp = TempDir::new().expect("temporary directory");
    let repository = temp.path().join("repository");
    fs::create_dir(&repository).expect("repository directory");
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ],
    ] {
        let status =
            Command::new("git").args(&args).current_dir(&repository).status().expect("git command");
        assert!(status.success());
    }
    let commit = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repository)
            .output()
            .expect("git revision")
            .stdout,
    )
    .expect("UTF-8 revision")
    .trim()
    .to_string();
    fs::write(repository.join("dirty.txt"), "dirty").expect("dirty marker");
    let mut scenario = fake_scenario(SUCCESS_SCRIPT, &temp.path().join("evidence"));
    scenario.revision_probes = vec![RevisionProbe {
        name: "rust-worktree".to_string(),
        expected: Some(commit.clone()),
        program: PathBuf::from("git"),
        args: vec![
            "-C".to_string(),
            repository.display().to_string(),
            "rev-parse".to_string(),
            "HEAD".to_string(),
        ],
        env: BTreeMap::new(),
        worktree: Some(repository),
        timeout: Duration::from_secs(2),
    }];

    let evidence = run_live_scenario(&scenario).expect("revision-attested scenario");

    assert_eq!(evidence.status, RunStatus::Passed);
    assert_eq!(evidence.revisions[0].actual.as_deref(), Some(commit.as_str()));
    assert_eq!(evidence.revisions[0].worktree_dirty, Some(true));
}

#[test]
fn oversized_dirty_worktree_status_is_recorded_without_failing_attestation() {
    let temp = TempDir::new().expect("temporary directory");
    let repository = temp.path().join("repository");
    fs::create_dir(&repository).expect("repository directory");
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ],
    ] {
        let status =
            Command::new("git").args(&args).current_dir(&repository).status().expect("git command");
        assert!(status.success());
    }
    let commit = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repository)
            .output()
            .expect("git revision")
            .stdout,
    )
    .expect("UTF-8 revision")
    .trim()
    .to_string();
    for index in 0..2_000 {
        fs::write(repository.join(format!("dirty-file-{index:04}-with-a-long-name.txt")), "dirty")
            .expect("dirty marker");
    }
    let mut scenario = fake_scenario(SUCCESS_SCRIPT, &temp.path().join("evidence"));
    scenario.revision_probes = vec![RevisionProbe {
        name: "rust-worktree".to_string(),
        expected: Some(commit),
        program: PathBuf::from("git"),
        args: vec![
            "-C".to_string(),
            repository.display().to_string(),
            "rev-parse".to_string(),
            "HEAD".to_string(),
        ],
        env: BTreeMap::new(),
        worktree: Some(repository),
        timeout: Duration::from_secs(2),
    }];

    let evidence = run_live_scenario(&scenario).expect("revision-attested scenario");

    assert_eq!(evidence.status, RunStatus::Passed);
    assert_eq!(evidence.revisions[0].worktree_dirty, Some(true));
    assert_eq!(evidence.revisions[0].error, None);
}

#[test]
fn revision_probe_with_retained_pipe_is_bounded_and_cleans_descendant() {
    let temp = TempDir::new().expect("temporary directory");
    let mut scenario = fake_scenario(SUCCESS_SCRIPT, temp.path());
    scenario.revision_probes = vec![RevisionProbe {
        name: "retained-pipe".to_string(),
        expected: Some("revision-1".to_string()),
        program: PathBuf::from("/bin/sh"),
        args: vec![
            "-c".to_string(),
            "printf revision-1; perl -MPOSIX -e 'POSIX::setsid(); sleep 30' & sleep 0.2; exit 0"
                .to_string(),
        ],
        env: BTreeMap::new(),
        worktree: None,
        timeout: Duration::from_millis(350),
    }];
    let started = Instant::now();

    let evidence = run_live_scenario(&scenario).expect("bounded revision evidence");

    assert_eq!(evidence.status, RunStatus::Failed);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(evidence.revisions[0].error.as_deref().unwrap_or_default().contains("timed out"));
}

#[test]
fn cancellation_stops_registered_process_group_and_retains_cleanup() {
    let temp = TempDir::new().expect("temporary directory");
    let marker = temp.path().join("started");
    let mut scenario = fake_scenario(
        "printf ready > \"$MARKER\"; trap 'exit 0' TERM; while :; do sleep 1; done",
        temp.path(),
    );
    scenario.env.insert("MARKER".to_string(), marker.display().to_string());
    scenario.timeout = Duration::from_secs(5);
    let cancellation = CancellationHandle::default();
    let runner_cancel = cancellation.clone();
    let runner = thread::spawn(move || run_live_scenario_cancellable(&scenario, &runner_cancel));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !marker.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(marker.exists(), "fake process did not register readiness");
    cancellation.cancel();
    let evidence = runner.join().expect("runner thread").expect("cancelled evidence");

    assert_eq!(evidence.status, RunStatus::Cancelled);
    assert!(evidence.cleanup.termination_requested);
    assert!(evidence.cleanup.direct_process_reaped);
    assert!(evidence.cleanup.process_group_gone);
    assert!(evidence.cleanup.temp_resources_removed);
}

#[test]
fn logs_are_bounded() {
    let temp = TempDir::new().expect("temporary directory");
    let mut scenario =
        fake_scenario(&format!("printf '%0200d\\n' 0\n{SUCCESS_SCRIPT}"), temp.path());
    scenario.max_log_bytes = 32;
    let evidence = run_live_scenario(&scenario).expect("fake scenario should run");
    assert_eq!(evidence.status, RunStatus::Passed);
    assert!(evidence.logs[0].truncated);
    assert_eq!(evidence.logs[0].text.len(), 32);
}

#[test]
fn retained_daemon_logs_have_checksums() {
    let temp = TempDir::new().expect("temporary directory");
    let mut scenario = fake_scenario(
        r#"
printf '%s\n' 'STYRENE_EVENT {"kind":"milestone","name":"peer-ready"}'
printf '%s\n' 'STYRENE_EVENT {"kind":"milestone","name":"message-received"}'
printf '%s\n' 'STYRENE_EVENT {"kind":"assertion","name":"content-matched","passed":true}'
printf daemon-log > "$STYRENE_RUN_ROOT/daemon.log"
printf '%s\n' 'STYRENE_EVENT {"kind":"artifact","name":"daemon-log","path":"daemon.log"}'
"#,
        temp.path(),
    );
    scenario.required_artifacts = vec!["daemon-log".to_string()];
    let evidence = run_live_scenario(&scenario).expect("daemon log scenario");
    assert_eq!(evidence.status, RunStatus::Passed);
    assert_eq!(evidence.artifacts[0].bytes, 10);
    assert!(Path::new(&evidence.artifacts[0].retained_path).is_file());
    assert_eq!(evidence.artifacts[0].sha256.len(), 64);
}

#[test]
fn aggregate_artifact_count_and_byte_limits_are_enforced() {
    let temp = TempDir::new().expect("temporary directory");
    let script = r#"
printf '%s\n' 'STYRENE_EVENT {"kind":"milestone","name":"peer-ready"}'
printf '%s\n' 'STYRENE_EVENT {"kind":"milestone","name":"message-received"}'
printf '%s\n' 'STYRENE_EVENT {"kind":"assertion","name":"content-matched","passed":true}'
printf aa > "$STYRENE_RUN_ROOT/a"
printf bb > "$STYRENE_RUN_ROOT/b"
printf cc > "$STYRENE_RUN_ROOT/c"
printf '%s\n' 'STYRENE_EVENT {"kind":"artifact","name":"a","path":"a"}'
printf '%s\n' 'STYRENE_EVENT {"kind":"artifact","name":"b","path":"b"}'
printf '%s\n' 'STYRENE_EVENT {"kind":"artifact","name":"c","path":"c"}'
"#;
    let mut count_limited = fake_scenario(script, &temp.path().join("count"));
    count_limited.required_artifacts = vec!["a".to_string()];
    count_limited.max_artifacts = 2;
    let count_evidence = run_live_scenario(&count_limited).expect("count-limited evidence");
    assert_eq!(count_evidence.status, RunStatus::Failed);
    assert_eq!(count_evidence.artifacts.len(), 2);
    assert!(count_evidence.artifact_rejections.iter().any(|item| item.reason.contains("count")));

    let mut byte_limited = fake_scenario(script, &temp.path().join("bytes"));
    byte_limited.required_artifacts = vec!["a".to_string()];
    byte_limited.max_artifact_total_bytes = 3;
    let byte_evidence = run_live_scenario(&byte_limited).expect("byte-limited evidence");
    assert_eq!(byte_evidence.status, RunStatus::Failed);
    assert_eq!(byte_evidence.artifacts.len(), 1);
    assert!(byte_evidence.artifact_rejections.iter().any(|item| item.reason.contains("aggregate")));
}

#[test]
fn repeated_and_concurrent_runs_use_correlation_specific_retained_directories() {
    let temp = TempDir::new().expect("temporary directory");
    let shared = temp.path().join("retained");
    let mut first = fake_scenario(SUCCESS_SCRIPT, &shared);
    first.args[1] = first.args[1].replace(&first.correlation_id, "concurrent-one");
    first.correlation_id = "concurrent-one".to_string();
    let mut second = fake_scenario(SUCCESS_SCRIPT, &shared);
    second.args[1] = second.args[1].replace(&second.correlation_id, "concurrent-two");
    second.correlation_id = "concurrent-two".to_string();

    let first_run = thread::spawn(move || run_live_scenario(&first));
    let second_run = thread::spawn(move || run_live_scenario(&second));
    let first_evidence = first_run.join().expect("first thread").expect("first evidence");
    let second_evidence = second_run.join().expect("second thread").expect("second evidence");

    assert_eq!(first_evidence.status, RunStatus::Passed);
    assert_eq!(second_evidence.status, RunStatus::Passed);
    assert_ne!(
        first_evidence.artifacts[0].retained_path,
        second_evidence.artifacts[0].retained_path
    );
    assert!(first_evidence.artifacts[0].retained_path.contains("concurrent-one"));
    assert!(second_evidence.artifacts[0].retained_path.contains("concurrent-two"));
    assert!(Path::new(&first_evidence.artifacts[0].retained_path).is_file());
    assert!(Path::new(&second_evidence.artifacts[0].retained_path).is_file());
}
