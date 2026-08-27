use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use styrene_interop_runner::{
    python_lxmf_scenario, run_live_scenario_cancellable, CancellationHandle, PinnedScenarioId,
    RunStatus,
};

#[derive(Parser)]
#[command(about = "Run a supervised live interoperability scenario")]
struct Args {
    scenario: PinnedScenarioId,
    #[arg(long, default_value_t = 90)]
    timeout: u64,
    #[arg(long, default_value = "target/interop/evidence.json")]
    evidence: PathBuf,
    #[arg(long)]
    python: Option<PathBuf>,
    #[arg(long)]
    correlation_id: Option<String>,
    #[arg(long)]
    cancel_file: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let python = args
        .python
        .or_else(|| std::env::var_os("LXMF_PYTHON_BIN").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("python3"));
    let mut scenario = python_lxmf_scenario(
        &repo_root,
        args.scenario,
        Duration::from_secs(args.timeout),
        &python.display().to_string(),
    );
    if let Some(correlation_id) = args.correlation_id {
        scenario.correlation_id = correlation_id;
    }
    scenario.evidence_dir = args
        .evidence
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!("{}-artifacts", args.scenario));
    let cancellation = CancellationHandle::default();
    let monitor_stop = Arc::new(AtomicBool::new(false));
    let monitor = args.cancel_file.map(|cancel_file| {
        let cancellation = cancellation.clone();
        let monitor_stop = monitor_stop.clone();
        thread::spawn(move || {
            while !monitor_stop.load(Ordering::Acquire) {
                if cancel_file.exists() {
                    cancellation.cancel();
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        })
    });
    let result = run_live_scenario_cancellable(&scenario, &cancellation);
    monitor_stop.store(true, Ordering::Release);
    if let Some(monitor) = monitor {
        let _ = monitor.join();
    }
    let evidence = match result {
        Ok(evidence) => evidence,
        Err(error) => {
            eprintln!("failed to run interoperability scenario: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(parent) = args.evidence.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!("failed to create evidence directory: {error}");
            return ExitCode::FAILURE;
        }
    }
    let encoded = match serde_json::to_vec_pretty(&evidence) {
        Ok(encoded) => encoded,
        Err(error) => {
            eprintln!("failed to encode evidence: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = std::fs::write(&args.evidence, encoded) {
        eprintln!("failed to write evidence: {error}");
        return ExitCode::FAILURE;
    }
    println!("{}", args.evidence.display());
    if evidence.status == RunStatus::Passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
