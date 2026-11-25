mod common;

use assert_cmd::Command;

#[test]
fn version_prints_package_name() {
    let output = Command::cargo_bin("zl").unwrap().arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("zettelist "), "stdout: {stdout}");
}

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
fn blank_body_exits_nonzero() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).arg("  ").output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("本文が空です"), "stderr: {stderr}");
}

#[test]
fn stopped_server_queues_the_note() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).arg("メモ").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("待ち行列に保存しました"), "stdout: {stdout}");
    let items = pending(dir.path());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["job"]["WriteNote"]["body"], "メモ");
}

#[test]
fn dash_body_after_double_dash() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--", "-x"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let items = pending(dir.path());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["job"]["WriteNote"]["body"], "-x");
}

#[test]
fn no_body_without_terminal_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("TUI は端末で起動します"), "stderr: {stderr}");
}

#[test]
fn server_rejection_still_queues_the_note() {
    // サーバーは生きているが、グラフ名が違うので拒否される (Permanent)。メモは失わず待ち行列に残る
    let server = common::TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = zl(dir.path());
    std::fs::write(
        dir.path().join("config.toml"),
        format!("[omnigraph]\nserver = \"{}\"\ngraph = \"nosuchgraph\"\n", server.config().omnigraph.server),
    )
    .unwrap();
    let output = cmd.arg("メモ").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("書き込めなかったので待ち行列に保存しました: "), "stdout: {stdout}");
    assert_eq!(pending(dir.path()).len(), 1);
}

#[test]
fn invalid_source_is_not_queued() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["-s", "", "メモ"]).output().unwrap();
    assert!(!output.status.success());
    assert!(pending(dir.path()).is_empty());
}

#[test]
fn worker_writes_queued_note_when_server_is_back() {
    use zettelist::notes::NewNote;
    use zettelist::queue::{Job, Queue};

    let server = common::TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = zl(dir.path());
    // サーバーは生きているが、Ollama には届かない
    std::fs::write(
        dir.path().join("config.toml"),
        format!(
            "[omnigraph]\nserver = \"{}\"\n[embed]\nollama = \"http://127.0.0.1:1\"\n",
            server.config().omnigraph.server
        ),
    )
    .unwrap();
    let note = NewNote { body: "あとで書く".to_string(), source: None, locator: None, created: chrono::Local::now() };
    Queue::open(&dir.path().join("state")).unwrap().push(Job::WriteNote(note)).unwrap();

    let output = cmd.args(["--worker", "--once"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let inbox = zettelist::notes::inbox(&server.og()).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].body, "あとで書く");
    // 埋め込みは Ollama に届かず、待ち行列に残って 1 回失敗した扱いになる
    let items = pending(dir.path());
    assert_eq!(items.len(), 1);
    assert!(items[0]["job"]["Embed"].is_object(), "{}", items[0]);
    assert_eq!(items[0]["attempts"], 1);
}

#[test]
fn reembed_queues_one_embed_per_note() {
    let server = common::TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = zl(dir.path());
    std::fs::write(
        dir.path().join("config.toml"),
        format!("[omnigraph]\nserver = \"{}\"\n", server.config().omnigraph.server),
    )
    .unwrap();
    let note = zettelist::notes::NewNote {
        body: "埋め込み直し".to_string(),
        source: None,
        locator: None,
        created: chrono::Local::now(),
    };
    zettelist::notes::write_note(&server.og(), &note).unwrap();

    let output = cmd.arg("--reembed").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("1 件の埋め込みを積みました"), "stdout: {stdout}");
    let items = pending(dir.path());
    assert_eq!(items.len(), 1);
    assert!(items[0]["job"]["Embed"].is_object(), "{}", items[0]);
}

#[test]
fn backup_without_repo_exits_nonzero() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).arg("--backup").output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("backup.repo が設定されていません"), "stderr: {stderr}");
}

#[test]
fn locator_without_source_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["-p", "p.42", "x"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--source"), "stderr: {stderr}");
    assert!(pending(dir.path()).is_empty());
}
