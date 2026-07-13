use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-env-changed=STYRENE_BUILD_SHA");

    let sha = std::env::var("STYRENE_BUILD_SHA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(git_short_sha)
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=STYRENE_BUILD_SHA={sha}");
}

fn git_short_sha() -> Option<String> {
    let output = Command::new("git").args(["rev-parse", "--short=9", "HEAD"]).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
