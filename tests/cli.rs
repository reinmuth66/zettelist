use assert_cmd::Command;

#[test]
fn version_prints_package_name() {
    let output = Command::cargo_bin("zl").unwrap().arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("zettelist "), "stdout: {stdout}");
}
