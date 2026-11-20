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

// ---- 読む操作 (--inbox、--show、--similar) ----

use chrono::{Duration, Local};
use common::{unit, TestServer};
use zettelist::notes::{promote, set_embedding, write_note, NewNote, NoteKind};

/// テスト用のサーバーに向けて `zl` を動かす。`inbox_similarity` は設定ファイルで渡す。
fn zl_on(server: &TestServer, dir: &std::path::Path, inbox_similarity: f32) -> Command {
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[omnigraph]\nserver = \"{}\"\n[link]\ninbox_similarity = {inbox_similarity}\n",
            server.config().omnigraph.server
        ),
    )
    .unwrap();
    let mut cmd = Command::cargo_bin("zl").unwrap();
    cmd.env("ZETTELIST_CONFIG", &config)
        .env("ZETTELIST_STATE_DIR", dir.join("state"))
        .env("ZETTELIST_NO_SPAWN", "1")
        .env("OMNIGRAPH_HOME", dir.join("og-home"));
    cmd
}

fn new_note(body: &str, source: Option<&str>, created: chrono::DateTime<Local>) -> NewNote {
    NewNote { body: body.to_string(), source: source.map(String::from), locator: None, created }
}

fn json_out(output: &std::process::Output) -> serde_json::Value {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("stdout は JSON のはず")
}

#[test]
fn inbox_json_lists_notes() {
    let server = TestServer::start();
    let og = server.og();
    let now = Local::now();
    let older = write_note(&og, &new_note("古いほう", None, now - Duration::hours(2))).unwrap();
    let newer = write_note(&og, &new_note("新しいほう", None, now - Duration::hours(1))).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--inbox", "--json"]).output().unwrap());
    let items = out.as_array().unwrap();
    assert_eq!(items.len(), 2, "{out}");
    assert_eq!(items[0]["slug"], older.slug);
    assert_eq!(items[1]["slug"], newer.slug);
    assert_eq!(items[0]["kind"], "fleeting");
    assert_eq!(items[0]["body"], "古いほう");
    assert_eq!(items[0]["old"], false);
    assert!(items[0]["created"].as_str().unwrap().ends_with('Z'), "{out}");
    assert!(chrono::DateTime::parse_from_rfc3339(items[0]["created"].as_str().unwrap()).is_ok(), "{out}");
    assert!(items[0]["source_id"].is_null() && items[0]["source_slug"].is_null() && items[0]["locator"].is_null());
}

#[test]
fn inbox_json_marks_old_and_literature() {
    let server = TestServer::start();
    let og = server.og();
    let old = write_note(&og, &new_note("昔のメモ", None, Local::now() - Duration::days(40))).unwrap();
    let lit = write_note(&og, &new_note("引用", Some("9784480068998"), Local::now())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--inbox", "--json"]).output().unwrap());
    let items = out.as_array().unwrap();
    assert_eq!(items[0]["slug"], old.slug);
    assert_eq!(items[0]["old"], true);
    assert_eq!(items[1]["slug"], lit.slug);
    assert_eq!(items[1]["kind"], "literature");
    assert!(items[1]["source_id"].as_str().is_some_and(|s| !s.is_empty()), "{out}");
    assert_eq!(items[1]["old"], false);
}

#[test]
fn inbox_human_lists_slug_and_body() {
    let server = TestServer::start();
    let og = server.og();
    let w = write_note(&og, &new_note("人向けの一覧", None, Local::now())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let output = zl_on(&server, dir.path(), 0.0).arg("--inbox").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(&w.slug) && stdout.contains("人向けの一覧"), "{stdout}");
}

#[test]
fn show_returns_body() {
    let server = TestServer::start();
    let og = server.og();
    let w = write_note(&og, &new_note("本文です", None, Local::now())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--show", &w.slug, "--json"]).output().unwrap());
    assert_eq!(out["kind"], "fleeting");
    assert_eq!(out["slug"], w.slug);
    assert_eq!(out["body"], "本文です");
    assert!(out["title"].is_null());
    assert!(chrono::DateTime::parse_from_rfc3339(out["created"].as_str().unwrap()).is_ok(), "{out}");
}

#[test]
fn show_permanent_has_title() {
    let server = TestServer::start();
    let og = server.og();
    let f = write_note(&og, &new_note("元", None, Local::now())).unwrap();
    let p = promote(&og, &[(NoteKind::Fleeting, f.slug.clone())], "題名", "永久の本文", Local::now() + Duration::hours(1))
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--show", &p, "--json"]).output().unwrap());
    assert_eq!(out["kind"], "permanent");
    assert_eq!(out["title"], "題名");
    assert_eq!(out["body"], "永久の本文");
}

#[test]
fn show_unknown_is_invalid() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let output = zl_on(&server, dir.path(), 0.0).args(["--show", "nosuch", "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
    assert!(err["error"].as_str().unwrap().contains("ノート 'nosuch' はありません"), "{err}");
}

#[test]
fn show_conflicts_with_other_actions() {
    let dir = tempfile::tempdir().unwrap();
    for args in [vec!["--show", "a", "--inbox"], vec!["--similar", "a", "--show", "a"], vec!["--show", "a", "本文"]] {
        let output = zl(dir.path()).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
}

#[test]
fn similar_lists_near_inbox_and_permanent() {
    let server = TestServer::start();
    let og = server.og();
    let now = Local::now();
    let me = write_note(&og, &new_note("基準", None, now - Duration::hours(4))).unwrap();
    let near = write_note(&og, &new_note("近いメモ", None, now - Duration::hours(3))).unwrap();
    let far = write_note(&og, &new_note("遠いメモ", None, now - Duration::hours(2))).unwrap();
    let src = write_note(&og, &new_note("元", None, now - Duration::hours(1))).unwrap();
    let perm_near =
        promote(&og, &[(NoteKind::Fleeting, src.slug.clone())], "近い永久", "近い永久の本文", now).unwrap();
    let perm_far =
        promote(&og, &[(NoteKind::Fleeting, src.slug.clone())], "遠い永久", "遠い永久の本文", now + Duration::hours(1))
            .unwrap();
    let x = unit(&[(0, 1.0)]);
    let close = unit(&[(0, 1.0), (1, 0.2)]);
    let orth = unit(&[(5, 1.0)]);
    set_embedding(&og, NoteKind::Fleeting, &me.slug, &x).unwrap();
    set_embedding(&og, NoteKind::Fleeting, &near.slug, &close).unwrap();
    set_embedding(&og, NoteKind::Fleeting, &far.slug, &orth).unwrap();
    set_embedding(&og, NoteKind::Permanent, &perm_near, &close).unwrap();
    set_embedding(&og, NoteKind::Permanent, &perm_far, &orth).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = json_out(&zl_on(&server, dir.path(), 0.5).args(["--similar", &me.slug, "--json"]).output().unwrap());
    let inbox = out["inbox"].as_array().unwrap();
    let slugs: Vec<_> = inbox.iter().map(|n| n["slug"].as_str().unwrap().to_string()).collect();
    assert_eq!(slugs, vec![near.slug.clone()], "{out}");
    assert_eq!(inbox[0]["kind"], "fleeting");
    assert_eq!(inbox[0]["body"], "近いメモ");
    assert!(inbox[0]["similarity"].as_f64().unwrap() > 0.9, "{out}");
    let perm = out["permanent"].as_array().unwrap();
    assert_eq!(perm.len(), 1, "{out}");
    assert_eq!(perm[0]["slug"], perm_near);
    assert_eq!(perm[0]["title"], "近い永久");
    assert_eq!(perm[0]["body"], "近い永久の本文");
}

#[test]
fn similar_without_embedding_is_empty() {
    let server = TestServer::start();
    let og = server.og();
    let w = write_note(&og, &new_note("埋め込みなし", None, Local::now())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--similar", &w.slug, "--json"]).output().unwrap());
    assert_eq!(out, serde_json::json!({"inbox": [], "permanent": []}));
}

#[test]
fn similar_unknown_is_invalid() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let output = zl_on(&server, dir.path(), 0.0).args(["--similar", "nosuch", "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
}

#[test]
fn similar_inbox_excludes_promoted_notes() {
    let server = TestServer::start();
    let og = server.og();
    let now = Local::now();
    let me = write_note(&og, &new_note("基準", None, now - Duration::hours(4))).unwrap();
    let promoted = write_note(&og, &new_note("昇格済み", None, now - Duration::hours(3))).unwrap();
    let live = write_note(&og, &new_note("まだ inbox", None, now - Duration::hours(2))).unwrap();
    promote(&og, &[(NoteKind::Fleeting, promoted.slug.clone())], "題名", "本文", now).unwrap();
    // 昇格済みのほうが me に近い
    set_embedding(&og, NoteKind::Fleeting, &me.slug, &unit(&[(0, 1.0)])).unwrap();
    set_embedding(&og, NoteKind::Fleeting, &promoted.slug, &unit(&[(0, 1.0), (1, 0.01)])).unwrap();
    set_embedding(&og, NoteKind::Fleeting, &live.slug, &unit(&[(0, 1.0), (1, 0.5)])).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = json_out(&zl_on(&server, dir.path(), 0.5).args(["--similar", &me.slug, "--json"]).output().unwrap());
    let slugs: Vec<_> = out["inbox"].as_array().unwrap().iter().map(|n| n["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, vec![live.slug.as_str()], "{out}");
}

#[test]
fn similar_permanent_excludes_self_and_caps_at_three() {
    let server = TestServer::start();
    let og = server.og();
    let now = Local::now();
    let src = write_note(&og, &new_note("元", None, now - Duration::hours(10))).unwrap();
    let mut perms = Vec::new();
    for i in 0..5 {
        let p = promote(
            &og,
            &[(NoteKind::Fleeting, src.slug.clone())],
            &format!("題名{i}"),
            &format!("本文{i}"),
            now + Duration::hours(i),
        )
        .unwrap();
        set_embedding(&og, NoteKind::Permanent, &p, &unit(&[(0, 1.0), (1, 0.1 * (i + 1) as f32)])).unwrap();
        perms.push(p);
    }
    let me = perms[0].clone();
    let dir = tempfile::tempdir().unwrap();
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--similar", &me, "--json"]).output().unwrap());
    let perm = out["permanent"].as_array().unwrap();
    assert_eq!(perm.len(), 3, "{out}");
    assert!(perm.iter().all(|n| n["slug"] != me.as_str()), "{out}");
}
