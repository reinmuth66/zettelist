mod common;

use chrono::{DateTime, Local, TimeZone};
use common::TestServer;
use serde_json::{json, Value};
use zettelist::notes::{write_note, NewNote, NoteKind};
use zettelist::omnigraph::Omnigraph;

fn note(body: &str, source: Option<&str>, locator: Option<&str>, created: DateTime<Local>) -> NewNote {
    NewNote {
        body: body.to_string(),
        source: source.map(String::from),
        locator: locator.map(String::from),
        created,
    }
}

fn at_ten() -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 11, 20, 10, 0, 0).unwrap()
}

fn count(og: &Omnigraph, node: &str) -> usize {
    let q = format!("query q() {{ match {{ $n: {node} }} return {{ $n.slug as slug }} }}");
    og.query("main", &q, json!({})).unwrap().len()
}

fn sources(og: &Omnigraph) -> Vec<serde_json::Map<String, Value>> {
    og.query(
        "main",
        "query q() { match { $s: Source } return { $s.id as id, $s.slug as slug, $s.title as title, $s.kind as kind, $s.input as input } }",
        json!({}),
    )
    .unwrap()
}

#[test]
fn plain_text_becomes_fleeting() {
    let s = TestServer::start();
    let og = s.og();
    let w = write_note(&og, &note("メモ", None, None, at_ten())).unwrap();
    assert_eq!(w.kind, NoteKind::Fleeting);
    let rows = og
        .query(
            "main",
            "query q($s: String) { match { $f: Fleeting { slug: $s } } return { $f.slug as slug, $f.body as body } }",
            json!({"s": w.slug}),
        )
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["body"], "メモ");
}

#[test]
fn same_second_gets_next_slug() {
    let s = TestServer::start();
    let og = s.og();
    let t = at_ten();
    let a = write_note(&og, &note("一件目", None, None, t)).unwrap();
    let b = write_note(&og, &note("二件目", None, None, t)).unwrap();
    assert_eq!(a.slug, "20261120100000");
    assert_eq!(b.slug, "20261120100001");
    let rows = og
        .query(
            "main",
            "query q($s: String) { match { $f: Fleeting { slug: $s } } return { $f.body as body } }",
            json!({"s": a.slug}),
        )
        .unwrap();
    assert_eq!(rows[0]["body"], "一件目");
    assert_eq!(count(&og, "Fleeting"), 2);
}

#[test]
fn same_second_collides_across_fleeting_and_literature() {
    let s = TestServer::start();
    let og = s.og();
    let t = at_ten();
    let a = write_note(&og, &note("一件目", None, None, t)).unwrap();
    let b = write_note(&og, &note("二件目", Some("978-4-480-06899-8"), None, t)).unwrap();
    assert_eq!(a.slug, "20261120100000");
    assert_eq!(b.slug, "20261120100001");
    assert_eq!(b.kind, NoteKind::Literature);
}

#[test]
fn unknown_isbn_creates_provisional_source() {
    let s = TestServer::start();
    let og = s.og();
    let w = write_note(&og, &note("本の一節", Some("978-4-480-06899-8"), Some("p.42"), at_ten())).unwrap();
    assert_eq!(w.kind, NoteKind::Literature);
    let src = sources(&og);
    assert_eq!(src.len(), 1);
    assert_eq!(src[0]["id"], "isbn:9784480068998");
    // slug が "" の行は null 扱いで出ないことがあるので、空か欠けているかを許す
    assert!(src[0].get("slug").map_or(true, |v| v == ""), "{:?}", src[0]);
    assert_eq!(src[0]["kind"], "other");
    assert_eq!(src[0]["title"], "978-4-480-06899-8");
    assert_eq!(src[0]["input"], "978-4-480-06899-8");
    let dump = og.export("main").unwrap();
    let cites: Vec<Value> = dump
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["edge"] == "Cites")
        .collect();
    assert_eq!(cites.len(), 1, "{dump}");
    assert_eq!(cites[0]["from"], w.slug.as_str());
    assert_eq!(cites[0]["to"], "isbn:9784480068998");
    assert_eq!(cites[0]["data"]["locator"], "p.42");
}

#[test]
fn same_isbn_twice_reuses_source() {
    let s = TestServer::start();
    let og = s.og();
    write_note(&og, &note("一つ目", Some("978-4-480-06899-8"), None, at_ten())).unwrap();
    let t2 = Local.with_ymd_and_hms(2026, 11, 20, 10, 5, 0).unwrap();
    write_note(&og, &note("二つ目", Some("9784480068998"), None, t2)).unwrap();
    assert_eq!(sources(&og).len(), 1);
    assert_eq!(count(&og, "Literature"), 2);
}

#[test]
fn registered_slug_is_cited() {
    let s = TestServer::start();
    let og = s.og();
    og.load(
        "main",
        &[json!({"type":"Source","data":{"id":"isbn:1","slug":"tanaka-2024","title":"S","kind":"book","input":"x","provenance":"{}"}})],
    )
    .unwrap();
    let w = write_note(&og, &note("引用", Some("tanaka-2024"), None, at_ten())).unwrap();
    assert_eq!(w.kind, NoteKind::Literature);
    assert_eq!(sources(&og).len(), 1);
    let dump = og.export("main").unwrap();
    assert!(dump.contains("\"Cites\"") && dump.contains("\"isbn:1\""), "{dump}");
}

#[test]
fn unknown_slug_is_rejected_and_writes_nothing() {
    let s = TestServer::start();
    let og = s.og();
    let e = write_note(&og, &note("本文", Some("nosuch"), None, at_ten())).unwrap_err();
    assert!(!e.is_transient());
    assert_eq!(e.to_string(), "出典 'nosuch' は登録されていません");
    assert_eq!(count(&og, "Fleeting"), 0);
    assert_eq!(count(&og, "Literature"), 0);
}

#[test]
fn blank_body_is_rejected() {
    let s = TestServer::start();
    let og = s.og();
    let e = write_note(&og, &note(" \n\t ", None, None, at_ten())).unwrap_err();
    assert!(!e.is_transient());
    assert_eq!(e.to_string(), "本文が空です");
    assert_eq!(count(&og, "Fleeting"), 0);
}
