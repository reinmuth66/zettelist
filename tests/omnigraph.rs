mod common;

use common::TestServer;
use serde_json::json;
use zettelist::config::OmnigraphConfig;
use zettelist::omnigraph::Omnigraph;

#[test]
fn load_then_query_returns_rows() {
    let s = TestServer::start();
    let og = s.og();
    og.load(
        "main",
        &[json!({"type":"Fleeting","data":{"slug":"20261120100000","body":"a","created":"2026-11-20T01:00:00.000Z"}})],
    )
    .unwrap();
    let rows = og
        .query("main", "query q() { match { $f: Fleeting } return { $f.slug } }", json!({}))
        .unwrap();
    // 行のキーは return の式の文字列 (`$f.slug` なら `f.slug`)。別名は `as` で付ける。
    assert_eq!(rows[0]["f.slug"], "20261120100000");
}

#[test]
fn card_violation_is_permanent() {
    let s = TestServer::start();
    let e = s
        .og()
        .load(
            "main",
            &[json!({"type":"Literature","data":{"slug":"x","body":"b","created":"2026-11-20T01:00:00.000Z"}})],
        )
        .unwrap_err();
    assert!(!e.is_transient(), "{e}");
}

#[test]
fn stopped_server_is_transient() {
    let cfg = OmnigraphConfig { server: "http://127.0.0.1:1".into(), graph: "zettelkasten".into() };
    let dir = tempfile::tempdir().unwrap();
    let e = Omnigraph::new(&cfg, dir.path())
        .unwrap()
        .with_home(dir.path())
        .query("main", "query q() { match { $f: Fleeting } return { $f.slug } }", json!({}))
        .unwrap_err();
    assert!(e.is_transient(), "{e}");
}

#[test]
fn branch_create_shows_in_branches() {
    let s = TestServer::start();
    let og = s.og();
    og.statement("branch create review from main").unwrap();
    assert!(og.branches().unwrap().contains(&"review".to_string()));
}

#[test]
fn column_types_shape() {
    let s = TestServer::start();
    let og = s.og();
    let vector: Vec<f64> = (0..1024).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect();
    og.load(
        "main",
        &[
            json!({"type":"Source","data":{"id":"isbn:1","slug":"","title":"S","kind":"other","input":"x","provenance":"{}","year":2024,"accessed":"2026-11-20"}}),
            json!({"type":"Permanent","data":{"slug":"p1","title":"t","body":"b","created":"2026-11-20T01:02:03.456Z","embedding":vector}}),
        ],
    )
    .unwrap();
    let src = og
        .query("main", "query q() { match { $s: Source } return { $s.year, $s.accessed } }", json!({}))
        .unwrap();
    let perm = og
        .query("main", "query q() { match { $p: Permanent } return { $p.created, $p.embedding } }", json!({}))
        .unwrap();
    eprintln!("I32/Date row: {}", serde_json::Value::Object(src[0].clone()));
    let created = &perm[0]["p.created"];
    let emb = &perm[0]["p.embedding"];
    eprintln!("DateTime: {created}");
    eprintln!("Vector is_array={} len={:?} first={:?}", emb.is_array(), emb.as_array().map(Vec::len), emb.get(0));
    assert!(src[0]["s.year"].is_i64() || src[0]["s.year"].is_u64(), "{:?}", src[0]);
    assert!(created.is_string());
    assert_eq!(emb.as_array().map(Vec::len), Some(1024));
}

#[test]
fn mutate_deletes_and_params_are_passed() {
    let s = TestServer::start();
    let og = s.og();
    og.load(
        "main",
        &[
            json!({"type":"Hub","data":{"slug":"h1","title":"one"}}),
            json!({"type":"Hub","data":{"slug":"h2","title":"two"}}),
        ],
    )
    .unwrap();
    og.mutate("main", "query d($s: String) { delete Hub where slug = $s }", json!({"s":"h1"})).unwrap();
    let rows = og
        .query("main", "query q($s: String) { match { $h: Hub { slug: $s } } return { $h.slug } }", json!({"s":"h2"}))
        .unwrap();
    assert_eq!(rows.len(), 1);
    let none = og
        .query("main", "query q($s: String) { match { $h: Hub { slug: $s } } return { $h.slug } }", json!({"s":"h1"}))
        .unwrap();
    assert!(none.is_empty());
}

#[test]
fn export_and_branch_delete() {
    let s = TestServer::start();
    let og = s.og();
    og.load("main", &[json!({"type":"Hub","data":{"slug":"h1","title":"one"}})]).unwrap();
    og.statement("branch create tmpb from main").unwrap();
    assert!(og.branches().unwrap().contains(&"tmpb".to_string()));
    og.statement("branch delete tmpb").unwrap();
    assert!(!og.branches().unwrap().contains(&"tmpb".to_string()));
    let dump = og.export("main").unwrap();
    assert!(dump.contains("\"h1\""), "{dump}");
}

#[test]
fn bad_query_is_permanent_and_stopped_export_is_transient() {
    let s = TestServer::start();
    let e = s.og().query("main", "query q() { match", json!({})).unwrap_err();
    assert!(!e.is_transient(), "{e}");
    let e = s.og().query("nobranch", "query q() { match { $f: Fleeting } return { $f.slug } }", json!({})).unwrap_err();
    assert!(!e.is_transient(), "{e}");

    let cfg = OmnigraphConfig { server: "http://127.0.0.1:1".into(), graph: "zettelkasten".into() };
    let dir = tempfile::tempdir().unwrap();
    let og = Omnigraph::new(&cfg, dir.path()).unwrap().with_home(dir.path());
    assert!(og.export("main").unwrap_err().is_transient());
    assert!(og.load("main", &[json!({"type":"Hub","data":{"slug":"h","title":"t"}})]).unwrap_err().is_transient());
    assert!(og.statement("branch create x from main").unwrap_err().is_transient());
    assert!(og.branches().unwrap_err().is_transient());
}
