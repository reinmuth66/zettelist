mod common;

use assert_cmd::Command;

/// サーバーなしで `zl` を動かす。状態は一時ディレクトリ、worker は起動しない。
fn zl(dir: &std::path::Path) -> Command {
    let config = dir.join("config.toml");
    std::fs::write(&config, "[omnigraph]\nserver = \"http://127.0.0.1:1\"\n").unwrap();
    let mut cmd = Command::cargo_bin("zl").unwrap();
    cmd.env("ZETTELIST_CONFIG", &config)
        .env("ZETTELIST_STATE_DIR", dir.join("state"))
        .env("ZETTELIST_NO_SPAWN", "1")
        .env("OMNIGRAPH_HOME", dir.join("og-home"));
    cmd
}

/// `state/queue/pending` の項目を JSON で読む。
fn pending(dir: &std::path::Path) -> Vec<serde_json::Value> {
    let mut items: Vec<_> = std::fs::read_dir(dir.join("state/queue/pending"))
        .map(|rd| rd.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    items.sort();
    items.iter().map(|p| serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()).collect()
}

#[test]
fn json_usage_error_is_json_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--json", "--inbox", "--review"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
    assert!(err["error"].as_str().is_some_and(|m| !m.is_empty()), "{err}");
    assert!(output.stdout.is_empty());
}

#[test]
fn usage_error_without_json_keeps_clap_message() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--inbox", "--review"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&stderr).is_err(), "stderr: {stderr}");
    assert!(stderr.contains("--inbox"), "stderr: {stderr}");
}

#[test]
fn body_and_action_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--inbox", "メモ"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(pending(dir.path()).is_empty());
}

#[test]
fn actions_are_mutually_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--backup", "--reembed"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn dash_dash_inbox_is_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--", "--inbox"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let items = pending(dir.path());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["job"]["WriteNote"]["body"], "--inbox");
}

#[test]
fn json_failure_is_json_on_stderr_with_exit_one() {
    // backup.repo が未設定 (Permanent)
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--json", "--backup"]).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "permanent");
    assert!(err["error"].as_str().unwrap().contains("backup.repo が設定されていません"), "{err}");
}

#[test]
fn json_invalid_input_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--json", "  "]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
}

#[test]
fn json_write_queued_prints_one_json_value() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--json", "メモ"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let out: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout は JSON のはず");
    assert_eq!(out["queued"], true);
}

#[test]
fn version_with_json_is_still_plain() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--json", "--version"]).output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().starts_with("zettelist "));
}

#[test]
fn source_with_action_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    for args in [vec!["--inbox", "-s", "foo"], vec!["--backup", "-s", "x", "-p", "3"], vec!["--worker", "-s", "x"]] {
        let output = zl(dir.path()).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
}

#[test]
fn json_source_with_action_is_json_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--json", "--inbox", "-s", "foo"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
}

#[test]
fn json_before_double_dash_makes_usage_error_json() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--json", "-p", "x", "--", "body"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
}

#[test]
fn json_only_after_double_dash_keeps_clap_output() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["-p", "x", "--", "body", "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&stderr).is_err(), "stderr: {stderr}");
}
