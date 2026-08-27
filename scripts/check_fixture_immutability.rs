use std::env;
use std::fs;
use std::path::Path;

fn snapshot(paths: &[String]) -> Result<Vec<u8>, String> {
    let mut output = b"styrene-fixture-snapshot-v1\0".to_vec();
    for path in paths {
        let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
        let path_bytes = path.as_bytes();
        output.extend_from_slice(&(path_bytes.len() as u64).to_le_bytes());
        output.extend_from_slice(path_bytes);
        output.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        output.extend_from_slice(&bytes);
    }
    Ok(output)
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mode = args.next().ok_or("expected snapshot or verify mode")?;
    let snapshot_path = args.next().ok_or("expected snapshot path")?;
    if !Path::new(&snapshot_path).starts_with("target") {
        return Err("snapshot path must be owned under target".into());
    }
    let paths = args.collect::<Vec<_>>();
    if paths.is_empty() {
        return Err("expected at least one fixture path".into());
    }
    let current = snapshot(&paths)?;
    match mode.as_str() {
        "snapshot" => fs::write(&snapshot_path, current)
            .map_err(|error| format!("write {snapshot_path}: {error}")),
        "verify" => {
            let expected = fs::read(&snapshot_path)
                .map_err(|error| format!("read {snapshot_path}: {error}"))?;
            if current == expected {
                Ok(())
            } else {
                Err("committed fixture changed while the test suite ran".into())
            }
        }
        _ => Err(format!("unsupported mode: {mode}")),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("fixture immutability check failed: {error}");
        std::process::exit(1);
    }
}
