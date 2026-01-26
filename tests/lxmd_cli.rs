#[test]
fn lxmd_help_has_expected_flags() {
    let output = std::process::Command::new("cargo")
        .args(["run", "--bin", "lxmd", "--", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--config"));
    assert!(stdout.contains("--propagation-node"));
}
