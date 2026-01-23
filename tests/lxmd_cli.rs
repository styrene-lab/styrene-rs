use assert_cmd::Command;

#[test]
fn lxmd_help_runs() {
    Command::cargo_bin("lxmd")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
}
