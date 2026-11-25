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
    let src2 = write_note(&og, &new_note("元二", None, now - Duration::minutes(30))).unwrap();
    let perm_near =
        promote(&og, &[(NoteKind::Fleeting, src.slug.clone())], "近い永久", "近い永久の本文", now).unwrap();
    let perm_far =
        promote(&og, &[(NoteKind::Fleeting, src2.slug.clone())], "遠い永久", "遠い永久の本文", now + Duration::hours(1))
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
    let mut perms = Vec::new();
    for i in 0..5 {
        let src = write_note(&og, &new_note(&format!("元{i}"), None, now - Duration::hours(10 - i))).unwrap();
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

// ---- 昇格と削除 (--promote、--delete) ----

fn inbox_slugs(server: &TestServer, dir: &std::path::Path) -> Vec<String> {
    let out = json_out(&zl_on(server, dir, 0.0).args(["--inbox", "--json"]).output().unwrap());
    out.as_array().unwrap().iter().map(|i| i["slug"].as_str().unwrap().to_string()).collect()
}

fn show_exists(server: &TestServer, dir: &std::path::Path, slug: &str) -> bool {
    zl_on(server, dir, 0.0).args(["--show", slug, "--json"]).output().unwrap().status.success()
}

#[test]
fn promote_creates_permanent_and_queues_job() {
    let server = TestServer::start();
    let og = server.og();
    let now = Local::now();
    let a = write_note(&og, &new_note("元 A", None, now - Duration::hours(2))).unwrap();
    let b = write_note(&og, &new_note("元 B", None, now - Duration::hours(1))).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let list = format!("{},{}", a.slug, b.slug);
    let out = json_out(
        &zl_on(&server, dir.path(), 0.0)
            .args(["--promote", &list, "--title", "T", "--body", "B", "--json"])
            .output()
            .unwrap(),
    );
    let slug = out["slug"].as_str().unwrap().to_string();
    assert!(inbox_slugs(&server, dir.path()).is_empty());
    let shown = json_out(&zl_on(&server, dir.path(), 0.0).args(["--show", &slug, "--json"]).output().unwrap());
    assert_eq!(shown["kind"], "permanent");
    assert_eq!(shown["title"], "T");
    assert_eq!(shown["body"], "B");
    let jobs = pending(dir.path());
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    assert_eq!(jobs[0]["job"]["Promote"]["slug"], slug);
}

#[test]
fn promote_reads_body_file() {
    let server = TestServer::start();
    let og = server.og();
    let a = write_note(&og, &new_note("元", None, Local::now())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("body.md");
    std::fs::write(&file, "ファイルの本文\n二行目").unwrap();
    let out = json_out(
        &zl_on(&server, dir.path(), 0.0)
            .args(["--promote", &a.slug, "--title", "題", "--body-file", file.to_str().unwrap(), "--json"])
            .output()
            .unwrap(),
    );
    let slug = out["slug"].as_str().unwrap();
    let shown = json_out(&zl_on(&server, dir.path(), 0.0).args(["--show", slug, "--json"]).output().unwrap());
    assert_eq!(shown["body"], "ファイルの本文\n二行目");
}

#[test]
fn promote_unreadable_body_file_is_invalid() {
    let server = TestServer::start();
    let og = server.og();
    let a = write_note(&og, &new_note("元", None, Local::now())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nosuch.md");
    let output = zl_on(&server, dir.path(), 0.0)
        .args(["--promote", &a.slug, "--title", "題", "--body-file", missing.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
    assert_eq!(inbox_slugs(&server, dir.path()), vec![a.slug.clone()]);
    assert!(pending(dir.path()).is_empty());
}

#[test]
fn promote_permanent_slug_is_invalid() {
    let server = TestServer::start();
    let og = server.og();
    let now = Local::now();
    let f = write_note(&og, &new_note("元", None, now - Duration::hours(1))).unwrap();
    let live = write_note(&og, &new_note("まだ inbox", None, now)).unwrap();
    let p = promote(&og, &[(NoteKind::Fleeting, f.slug.clone())], "既存", "既存の本文", now + Duration::hours(1)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let list = format!("{},{}", live.slug, p);
    let output = zl_on(&server, dir.path(), 0.0)
        .args(["--promote", &list, "--title", "T", "--body", "B", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
    // 何も書かれていない: live は inbox に残り、Permanent は 1 件のまま
    assert_eq!(inbox_slugs(&server, dir.path()), vec![live.slug.clone()]);
    assert!(pending(dir.path()).is_empty());
    let count = og
        .query("main", "query q() { match { $n: Permanent } return { $n.slug as slug } }", serde_json::json!({}))
        .unwrap()
        .len();
    assert_eq!(count, 1);
}

#[test]
fn promote_unknown_slug_is_invalid() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let output = zl_on(&server, dir.path(), 0.0)
        .args(["--promote", "nosuch", "--title", "T", "--body", "B", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn promote_blank_title_is_invalid() {
    let server = TestServer::start();
    let og = server.og();
    let a = write_note(&og, &new_note("元", None, Local::now())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let output = zl_on(&server, dir.path(), 0.0)
        .args(["--promote", &a.slug, "--title", "  ", "--body", "B", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(inbox_slugs(&server, dir.path()), vec![a.slug.clone()]);
    assert!(pending(dir.path()).is_empty());
}

#[test]
fn promote_option_usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["--promote", "a", "--title", "T"],
        vec!["--promote", "a", "--body", "B"],
        vec!["--promote", "a", "--title", "T", "--body", "B", "--body-file", "x"],
        vec!["--title", "T"],
        vec!["--body", "B"],
        vec!["--body-file", "x"],
        vec!["--inbox", "--title", "T"],
        vec!["--promote", "a", "--title", "T", "--body", "B", "--inbox"],
        vec!["--promote", "a", "--title", "T", "--body", "B", "メモ"],
    ] {
        let output = zl(dir.path()).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
    assert!(pending(dir.path()).is_empty());
}

#[test]
fn delete_requires_yes() {
    let server = TestServer::start();
    let og = server.og();
    let a = write_note(&og, &new_note("残る", None, Local::now())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let output = zl_on(&server, dir.path(), 0.0).args(["--delete", &a.slug, "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
    assert!(err["error"].as_str().unwrap().contains("--yes"), "{err}");
    assert!(show_exists(&server, dir.path(), &a.slug));
}

#[test]
fn delete_with_yes_removes() {
    let server = TestServer::start();
    let og = server.og();
    let now = Local::now();
    let a = write_note(&og, &new_note("消す A", None, now - Duration::hours(2))).unwrap();
    let b = write_note(&og, &new_note("消す B", None, now - Duration::hours(1))).unwrap();
    let keep = write_note(&og, &new_note("残す", None, now)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let list = format!("{},{}", a.slug, b.slug);
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--delete", &list, "--yes", "--json"]).output().unwrap());
    assert_eq!(out, serde_json::json!({"ok": true}));
    assert_eq!(inbox_slugs(&server, dir.path()), vec![keep.slug.clone()]);
    assert!(!show_exists(&server, dir.path(), &a.slug));
}

#[test]
fn delete_unknown_is_invalid_and_deletes_nothing() {
    let server = TestServer::start();
    let og = server.og();
    let a = write_note(&og, &new_note("残る", None, Local::now())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let list = format!("{},nosuch", a.slug);
    let output = zl_on(&server, dir.path(), 0.0).args(["--delete", &list, "--yes", "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
    assert!(show_exists(&server, dir.path(), &a.slug));
}

#[test]
fn delete_conflicts_with_other_actions() {
    let dir = tempfile::tempdir().unwrap();
    for args in [vec!["--delete", "a", "--yes", "--inbox"], vec!["--delete", "a", "--promote", "b", "--yes"], vec!["--delete", "a", "--yes", "本文"]] {
        let output = zl(dir.path()).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
}

#[test]
fn promote_already_promoted_is_invalid() {
    let server = TestServer::start();
    let og = server.og();
    let a = write_note(&og, &new_note("元", None, Local::now())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let args = ["--promote", a.slug.as_str(), "--title", "T", "--body", "B", "--json"];
    json_out(&zl_on(&server, dir.path(), 0.0).args(args).output().unwrap());
    assert_eq!(pending(dir.path()).len(), 1);
    let output = zl_on(&server, dir.path(), 0.0).args(args).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
    assert!(err["error"].as_str().unwrap().contains("昇格済み"), "{err}");
    let count = og
        .query("main", "query q() { match { $n: Permanent } return { $n.slug as slug } }", serde_json::json!({}))
        .unwrap()
        .len();
    assert_eq!(count, 1);
    assert_eq!(pending(dir.path()).len(), 1);
}

// ---- レビュー (--review、--adopt、--reject、--undecide、--merge) ----

use serde_json::json;
use zettelist::omnigraph::Omnigraph;
use zettelist::propose::ensure_review;

/// 昇格元の Fleeting を 1 件書いて Permanent を作る (埋め込みは付けない)。
fn make_permanent(og: &Omnigraph, title: &str, minute: i64) -> String {
    let base = Local::now() - Duration::hours(3) + Duration::minutes(minute);
    let f = write_note(og, &new_note(&format!("{title} の元"), None, base)).unwrap();
    promote(og, &[(NoteKind::Fleeting, f.slug)], title, &format!("{title} の本文"), base + Duration::minutes(1)).unwrap()
}

/// review に Links の提案を書く。
fn propose_link(og: &Omnigraph, from: &str, to: &str, similarity: f32) {
    ensure_review(og).unwrap();
    og.load("review", &[json!({"edge":"Links","from":from,"to":to,"data":{"similarity":similarity}})]).unwrap();
}

/// 類似度の違う Links の提案を 2 本 (a->b 0.9、a->c 0.8) 作り、(a, b, c) を返す。
fn two_link_proposals(og: &Omnigraph) -> (String, String, String) {
    let a = make_permanent(og, "A", 0);
    let b = make_permanent(og, "B", 10);
    let c = make_permanent(og, "C", 20);
    propose_link(og, &a, &b, 0.9);
    propose_link(og, &a, &c, 0.8);
    (a, b, c)
}

fn main_links(og: &Omnigraph) -> Vec<(String, String, Option<String>)> {
    let rows = og
        .query(
            "main",
            "query q() { match { $a: Permanent $a $l:links $b } return { $a.slug as src, $b.slug as dst, $l.reason as reason } }",
            json!({}),
        )
        .unwrap();
    let mut v: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r["src"].as_str().unwrap().to_string(),
                r["dst"].as_str().unwrap().to_string(),
                r.get("reason").and_then(|x| x.as_str()).map(String::from),
            )
        })
        .collect();
    v.sort();
    v
}

fn review_json_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("state/review.json")
}

#[test]
fn review_json_lists_with_decisions() {
    let server = TestServer::start();
    let og = server.og();
    let (a, b, c) = two_link_proposals(&og);
    let dir = tempfile::tempdir().unwrap();
    let key_ab = format!("link:{a}:{b}");
    json_out(&zl_on(&server, dir.path(), 0.0).args(["--adopt", &key_ab, "--json"]).output().unwrap());
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--review", "--json"]).output().unwrap());
    let items = out.as_array().unwrap();
    assert_eq!(items.len(), 2, "{out}");
    assert_eq!(items[0]["key"], key_ab);
    assert_eq!(items[0]["type"], "link");
    assert_eq!(items[0]["from"], a);
    assert_eq!(items[0]["to"], b);
    assert!((items[0]["similarity"].as_f64().unwrap() - 0.9).abs() < 1e-3, "{out}");
    assert_eq!(items[0]["decision"], "adopt");
    assert!(items[0]["reason"].is_null());
    assert_eq!(items[1]["key"], format!("link:{a}:{c}"));
    assert!(items[1]["decision"].is_null(), "{out}");
    assert!(items[1]["reason"].is_null());
}

#[test]
fn review_json_shows_adopt_reason() {
    let server = TestServer::start();
    let og = server.og();
    let (a, b, _c) = two_link_proposals(&og);
    let dir = tempfile::tempdir().unwrap();
    let key_ab = format!("link:{a}:{b}");
    json_out(&zl_on(&server, dir.path(), 0.0).args(["--adopt", &key_ab, "--reason", "理由", "--json"]).output().unwrap());
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--review", "--json"]).output().unwrap());
    assert_eq!(out[0]["decision"], "adopt");
    assert_eq!(out[0]["reason"], "理由");
    let human = zl_on(&server, dir.path(), 0.0).arg("--review").output().unwrap();
    assert!(human.status.success());
    assert!(String::from_utf8(human.stdout).unwrap().contains(&key_ab));
}

#[test]
fn review_empty_is_empty_array() {
    let server = TestServer::start();
    let dir = tempfile::tempdir().unwrap();
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--review", "--json"]).output().unwrap());
    assert_eq!(out, json!([]));
}

#[test]
fn adopt_with_reason_then_merge() {
    let server = TestServer::start();
    let og = server.og();
    let (a, b, c) = two_link_proposals(&og);
    let dir = tempfile::tempdir().unwrap();
    let key_ab = format!("link:{a}:{b}");
    let ok = json_out(
        &zl_on(&server, dir.path(), 0.0).args(["--adopt", &key_ab, "--reason", "同じ話", "--json"]).output().unwrap(),
    );
    assert_eq!(ok, json!({"ok": true}));
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--merge", "--yes", "--json"]).output().unwrap());
    assert_eq!(out, json!({"links": 1, "hubs": 0}));
    assert_eq!(main_links(&og), vec![(a.clone(), b.clone(), Some("同じ話".to_string()))]);
    let jobs = pending(dir.path());
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    assert!(jobs[0]["job"]["Backup"]["message"].as_str().unwrap().contains("links 1"), "{jobs:?}");
    // 未判断の a->c は review に残る
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--review", "--json"]).output().unwrap());
    assert_eq!(out.as_array().unwrap().len(), 1, "{out}");
    assert_eq!(out[0]["key"], format!("link:{a}:{c}"));
    assert!(out[0]["decision"].is_null());
}

#[test]
fn reject_then_merge_drops() {
    let server = TestServer::start();
    let og = server.og();
    let (a, b, c) = two_link_proposals(&og);
    let dir = tempfile::tempdir().unwrap();
    let key_ab = format!("link:{a}:{b}");
    let ok = json_out(&zl_on(&server, dir.path(), 0.0).args(["--reject", &key_ab, "--json"]).output().unwrap());
    assert_eq!(ok, json!({"ok": true}));
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--merge", "--yes", "--json"]).output().unwrap());
    assert_eq!(out, json!({"links": 0, "hubs": 0}));
    assert!(main_links(&og).is_empty());
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--review", "--json"]).output().unwrap());
    let keys: Vec<_> = out.as_array().unwrap().iter().map(|i| i["key"].as_str().unwrap().to_string()).collect();
    assert_eq!(keys, vec![format!("link:{a}:{c}")]);
}

#[test]
fn undecide_clears() {
    let server = TestServer::start();
    let og = server.og();
    let (a, b, _c) = two_link_proposals(&og);
    let dir = tempfile::tempdir().unwrap();
    let key_ab = format!("link:{a}:{b}");
    json_out(&zl_on(&server, dir.path(), 0.0).args(["--adopt", &key_ab, "--json"]).output().unwrap());
    let ok = json_out(&zl_on(&server, dir.path(), 0.0).args(["--undecide", &key_ab, "--json"]).output().unwrap());
    assert_eq!(ok, json!({"ok": true}));
    let out = json_out(&zl_on(&server, dir.path(), 0.0).args(["--review", "--json"]).output().unwrap());
    assert!(out[0]["decision"].is_null(), "{out}");
    // 判断がないものを外してもエラーにしない
    json_out(&zl_on(&server, dir.path(), 0.0).args(["--undecide", &key_ab, "--json"]).output().unwrap());
}

#[test]
fn unknown_key_is_invalid() {
    let server = TestServer::start();
    let og = server.og();
    let (a, b, _c) = two_link_proposals(&og);
    let dir = tempfile::tempdir().unwrap();
    let key_ab = format!("link:{a}:{b}");
    json_out(&zl_on(&server, dir.path(), 0.0).args(["--adopt", &key_ab, "--json"]).output().unwrap());
    let before = std::fs::read(review_json_path(dir.path())).unwrap();
    for flag in ["--adopt", "--reject", "--undecide"] {
        let output = zl_on(&server, dir.path(), 0.0).args([flag, "link:x:y", "--json"]).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{flag}: {}", String::from_utf8_lossy(&output.stderr));
        let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
        assert_eq!(err["kind"], "invalid");
        assert!(err["error"].as_str().unwrap().contains("提案 'link:x:y' はありません"), "{err}");
        assert!(output.stdout.is_empty());
    }
    assert_eq!(std::fs::read(review_json_path(dir.path())).unwrap(), before);
}

#[test]
fn merge_requires_yes() {
    let server = TestServer::start();
    let og = server.og();
    let (a, b, _c) = two_link_proposals(&og);
    let dir = tempfile::tempdir().unwrap();
    let key_ab = format!("link:{a}:{b}");
    json_out(&zl_on(&server, dir.path(), 0.0).args(["--adopt", &key_ab, "--json"]).output().unwrap());
    let output = zl_on(&server, dir.path(), 0.0).args(["--merge", "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
    assert!(main_links(&og).is_empty());
    assert!(pending(dir.path()).is_empty());
    let state = std::fs::read_to_string(review_json_path(dir.path())).unwrap();
    assert!(state.contains(&key_ab), "判断が残っているはず: {state}");
}

#[test]
fn review_option_usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["--reason", "x"],
        vec!["--reject", "k", "--reason", "x"],
        vec!["--undecide", "k", "--reason", "x"],
        vec!["--adopt", "a", "--reject", "b"],
        vec!["--merge", "--review"],
        vec!["--merge", "--yes", "本文"],
        vec!["--adopt", "k", "本文"],
    ] {
        let output = zl(dir.path()).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
}

// --- 失敗の一覧とやり直し (サーバーなし) ---

use zettelist::queue::{Job, Queue};

/// `state/queue/failed` に項目を作る。積んでから failed へ回し、id の一覧を返す。
fn make_failed(dir: &std::path::Path, jobs: Vec<(Job, &str)>) -> Vec<String> {
    let q = Queue::open(&dir.join("state")).unwrap();
    let now = std::time::SystemTime::now() + std::time::Duration::from_secs(1);
    jobs.into_iter()
        .map(|(job, error)| {
            let id = q.push(job).unwrap();
            let entry = q.next_ready(now).unwrap().unwrap();
            assert_eq!(entry.id, id);
            q.fail(entry, error).unwrap();
            id
        })
        .collect()
}

fn failed_ids(dir: &std::path::Path) -> Vec<String> {
    Queue::open(&dir.join("state")).unwrap().failed().unwrap().into_iter().map(|e| e.id).collect()
}

fn embed_job(slug: &str) -> Job {
    Job::Embed { kind: zettelist::notes::NoteKind::Fleeting, slug: slug.to_string() }
}

fn stderr_json(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず")
}

#[test]
fn failed_json_lists_entries() {
    let dir = tempfile::tempdir().unwrap();
    let note = zettelist::notes::NewNote {
        body: "メモ".to_string(),
        source: None,
        locator: None,
        created: chrono::Local::now(),
    };
    let ids = make_failed(
        dir.path(),
        vec![
            (Job::WriteNote(note), "書けない"),
            (embed_job("a"), "届かない"),
            (Job::Promote { slug: "p".to_string() }, "だめ"),
            (Job::Backup { message: "m".to_string() }, "push できない"),
        ],
    );
    let output = zl(dir.path()).args(["--failed", "--json"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let out: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout は JSON のはず");
    let list = out.as_array().unwrap();
    assert_eq!(list.len(), 4, "{out}");
    let jobs: Vec<_> = list.iter().map(|e| e["job"].as_str().unwrap()).collect();
    assert_eq!(jobs, ["write_note", "embed", "promote", "backup"]);
    assert_eq!(list[1]["id"], ids[1].as_str());
    assert_eq!(list[1]["attempts"], 0);
    assert_eq!(list[1]["last_error"], "届かない");
}

#[test]
fn failed_with_no_entries_is_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--failed", "--json"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(), serde_json::json!([]));
    let output = zl(dir.path()).arg("--failed").output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains("失敗したジョブはありません"));
}

#[test]
fn failed_human_output_shows_id_job_and_error() {
    let dir = tempfile::tempdir().unwrap();
    let ids = make_failed(dir.path(), vec![(embed_job("a"), "届かない")]);
    let output = zl(dir.path()).arg("--failed").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(&ids[0]) && stdout.contains("embed") && stdout.contains("届かない"), "{stdout}");
}

#[test]
fn retry_moves_to_pending() {
    let dir = tempfile::tempdir().unwrap();
    let ids = make_failed(dir.path(), vec![(embed_job("a"), "だめ"), (embed_job("b"), "だめ")]);
    let output = zl(dir.path()).args(["--retry", &ids[0], "--json"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(), serde_json::json!({"ok": true}));
    assert_eq!(failed_ids(dir.path()), vec![ids[1].clone()]);
    let items = pending(dir.path());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], ids[0].as_str());
    assert_eq!(items[0]["attempts"], 0);
}

#[test]
fn retry_all_moves_all() {
    let dir = tempfile::tempdir().unwrap();
    make_failed(dir.path(), vec![(embed_job("a"), "だめ"), (embed_job("b"), "だめ")]);
    let output = zl(dir.path()).args(["--retry-all", "--json"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(), serde_json::json!({"ok": true}));
    assert!(failed_ids(dir.path()).is_empty());
    assert_eq!(pending(dir.path()).len(), 2);
}

#[test]
fn retry_all_with_nothing_failed_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).args(["--retry-all", "--json"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(pending(dir.path()).is_empty());
}

#[test]
fn discard_requires_yes() {
    let dir = tempfile::tempdir().unwrap();
    let ids = make_failed(dir.path(), vec![(embed_job("a"), "だめ")]);
    let output = zl(dir.path()).args(["--discard", &ids[0], "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(stderr_json(&output)["kind"], "invalid");
    assert!(output.stdout.is_empty());
    assert_eq!(failed_ids(dir.path()), ids);
}

#[test]
fn discard_with_yes_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let ids = make_failed(dir.path(), vec![(embed_job("a"), "だめ"), (embed_job("b"), "だめ")]);
    let output = zl(dir.path()).args(["--discard", &ids[0], "--yes", "--json"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(), serde_json::json!({"ok": true}));
    assert_eq!(failed_ids(dir.path()), vec![ids[1].clone()]);
    assert!(pending(dir.path()).is_empty());
}

#[test]
fn retry_unknown_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let ids = make_failed(dir.path(), vec![(embed_job("a"), "だめ")]);
    let output = zl(dir.path()).args(["--retry", "nothing", "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err = stderr_json(&output);
    assert_eq!(err["kind"], "invalid");
    assert!(err["error"].as_str().unwrap().contains("ありません"), "{err}");
    assert!(output.stdout.is_empty());
    assert_eq!(failed_ids(dir.path()), ids);
    assert!(pending(dir.path()).is_empty());
}

#[test]
fn discard_unknown_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let ids = make_failed(dir.path(), vec![(embed_job("a"), "だめ")]);
    let output = zl(dir.path()).args(["--discard", "nothing", "--yes", "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(stderr_json(&output)["kind"], "invalid");
    assert_eq!(failed_ids(dir.path()), ids);
}

#[test]
fn path_like_id_is_invalid_and_touches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let ids = make_failed(dir.path(), vec![(embed_job("a"), "だめ")]);
    let output = zl(dir.path()).args(["--retry", "../failed/x", "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(stderr_json(&output)["kind"], "invalid");
    assert_eq!(failed_ids(dir.path()), ids);
}

#[test]
fn failed_option_usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["--failed", "--retry-all"],
        vec!["--retry", "x", "--discard", "y", "--yes"],
        vec!["--retry-all", "--inbox"],
        vec!["--failed", "本文"],
        vec!["--retry", "x", "本文"],
        vec!["--discard", "x", "--delete", "y", "--yes"],
    ] {
        let output = zl(dir.path()).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
}

#[test]
fn skill_export_writes_file() {
    let dir = tempfile::tempdir().unwrap();
    // 存在しない入れ子のディレクトリも作る
    let target = dir.path().join("skills/zettelist");
    let output = zl(dir.path()).args(["--skill-export", target.to_str().unwrap(), "--json"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let out: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(out, serde_json::json!({"ok": true}));
    let text = std::fs::read_to_string(target.join("SKILL.md")).unwrap();
    assert!(text.starts_with("---\nname: zettelist\n"), "{text}");
}

#[test]
fn skill_export_refuses_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out");
    let path = target.to_str().unwrap();
    assert!(zl(dir.path()).args(["--skill-export", path]).output().unwrap().status.success());
    std::fs::write(target.join("SKILL.md"), "古い内容").unwrap();

    let output = zl(dir.path()).args(["--skill-export", path, "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err = stderr_json(&output);
    assert_eq!(err["kind"], "invalid");
    assert!(err["error"].as_str().unwrap().contains("--force"), "{err}");
    assert_eq!(std::fs::read_to_string(target.join("SKILL.md")).unwrap(), "古い内容");

    let output = zl(dir.path()).args(["--skill-export", path, "--force"]).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(std::fs::read_to_string(target.join("SKILL.md")).unwrap().starts_with("---\nname: zettelist\n"));
}

#[test]
fn skill_export_option_usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out");
    let path = target.to_str().unwrap();
    for args in [
        vec!["--skill-export", path, "--inbox"],
        vec!["--skill-export", path, "本文"],
        vec!["--force"],
        vec!["--inbox", "--force"],
    ] {
        let output = zl(dir.path()).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
    assert!(!target.exists());
}

/// skill に、オプション名がそのまま (別のオプションの前半ではなく) 書かれているか。
/// `--retry` が `--retry-all` の前半に一致して通ってしまわないよう、前後の文字を見る。
fn mentions_option(text: &str, option: &str) -> bool {
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '-';
    text.match_indices(option).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + option.len()..].chars().next();
        !before.is_some_and(is_name) && !after.is_some_and(is_name)
    })
}

#[test]
fn skill_mentions_every_option() {
    let skill = zettelist::skill::SKILL;
    for option in [
        "--inbox", "--show", "--similar", "--promote", "--title", "--body", "--body-file", "--delete", "--review",
        "--adopt", "--reason", "--reject", "--undecide", "--merge", "--failed", "--retry", "--retry-all", "--discard",
        "--yes", "--json", "--reembed", "--backup", "--worker", "--skill-export", "--force", "-s", "-p",
    ] {
        assert!(mentions_option(skill, option), "skill に {option} がありません");
    }
}

#[test]
fn mentions_option_tells_retry_from_retry_all() {
    assert!(mentions_option("`--retry-all` だけ", "--retry-all"));
    assert!(!mentions_option("`--retry-all` だけ", "--retry"));
    assert!(mentions_option("`--retry` と `--retry-all`", "--retry"));
    assert!(!mentions_option("zl --promote", "-s"));
}

#[test]
fn json_without_action_is_an_error_not_success() {
    let dir = tempfile::tempdir().unwrap();
    let output = zl(dir.path()).arg("--json").output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr は JSON のはず");
    assert_eq!(err["kind"], "invalid");
    assert!(err["error"].as_str().is_some_and(|m| m.contains("TUI を起動できません")), "{err}");
    assert!(output.stdout.is_empty());
}
