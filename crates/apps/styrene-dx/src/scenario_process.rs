use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use styrene_interop_runner::{CancellationHandle, LiveScenario, RunEvidence};

use crate::scenario::LiveScenarioExecutor;

pub(crate) struct ProcessRunnerExecutor {
    runner: Option<PathBuf>,
}

impl ProcessRunnerExecutor {
    pub(crate) const fn new() -> Self {
        Self { runner: None }
    }

    fn runner_path(&self) -> Result<PathBuf, String> {
        if let Some(path) = &self.runner {
            return Ok(path.clone());
        }
        if let Some(path) = std::env::var_os("STYRENE_DX_INTEROP_RUNNER") {
            let path = PathBuf::from(path);
            return path
                .is_file()
                .then_some(path)
                .ok_or_else(|| "STYRENE_DX_INTEROP_RUNNER is not a runner executable".to_string());
        }
        let sibling = std::env::current_exe()
            .map_err(|error| error.to_string())?
            .with_file_name("styrene-interop");
        sibling.is_file().then_some(sibling).ok_or_else(|| {
            "live interoperability runner is unavailable; set STYRENE_DX_INTEROP_RUNNER".to_string()
        })
    }
}

impl LiveScenarioExecutor for ProcessRunnerExecutor {
    fn availability(&self) -> Result<(), String> {
        self.runner_path().map(|_| ())
    }

    fn run(
        &self,
        scenario: LiveScenario,
        cancellation: CancellationHandle,
    ) -> Result<RunEvidence, String> {
        let runner = self.runner_path()?;
        std::fs::create_dir_all(&scenario.evidence_dir).map_err(|error| error.to_string())?;
        let evidence_path = scenario.evidence_dir.join(format!("{}.json", scenario.correlation_id));
        let cancel_path = scenario.evidence_dir.join(format!("{}.cancel", scenario.correlation_id));
        let _ = std::fs::remove_file(&cancel_path);
        let mut child = Command::new(runner)
            .arg(&scenario.id)
            .arg("--timeout")
            .arg(scenario.timeout.as_secs().to_string())
            .arg("--evidence")
            .arg(&evidence_path)
            .arg("--correlation-id")
            .arg(&scenario.correlation_id)
            .arg("--cancel-file")
            .arg(&cancel_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("failed to start interoperability runner: {error}"))?;
        let status = loop {
            if cancellation.is_cancelled() && !cancel_path.exists() {
                if let Err(error) = std::fs::write(&cancel_path, b"cancel\n") {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("failed to signal runner cancellation: {error}"));
                }
            }
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let _ = std::fs::remove_file(&cancel_path);
        let encoded = std::fs::read(&evidence_path).map_err(|error| {
            format!("interoperability runner exited with {status} without evidence: {error}")
        })?;
        serde_json::from_slice(&encoded)
            .map_err(|error| format!("invalid interoperability runner evidence: {error}"))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use styrene_interop_runner::{python_lxmf_scenario, PinnedScenarioId};

    #[test]
    #[ignore = "executes an isolated local helper process; run explicitly for runner-boundary validation"]
    fn runner_process_is_separate_and_evidence_is_typed() {
        let root = std::env::temp_dir().join(format!(
            "styrene-dx-runner-boundary-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir(&root).expect("create test directory");
        let marker = root.join("runner.pid");
        let runner = root.join("runner.sh");
        std::fs::write(
            &runner,
            format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nprintf '{{}}' > \"$5\"\n",
                marker.display()
            ),
        )
        .expect("write helper runner");
        std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o700))
            .expect("make helper executable");
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut scenario = python_lxmf_scenario(
            &repo_root,
            PinnedScenarioId::Direct,
            Duration::from_secs(1),
            "python3",
        );
        scenario.evidence_dir = root.join("evidence");

        let error = ProcessRunnerExecutor { runner: Some(runner) }
            .run(scenario, CancellationHandle::default())
            .expect_err("untyped evidence must be rejected");
        assert!(error.contains("invalid interoperability runner evidence"));
        let runner_pid = std::fs::read_to_string(&marker).expect("runner process marker");
        assert_ne!(runner_pid, std::process::id().to_string());
        std::fs::remove_dir_all(root).expect("remove test directory");
    }
}
