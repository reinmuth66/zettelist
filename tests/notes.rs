mod common;

use chrono::{DateTime, Local, TimeZone, Utc};
use common::{unit, TestServer};
use serde_json::{json, Value};
use zettelist::error::Error;
use zettelist::notes::{
    delete_notes, get_embedding, inbox, inbox_embeddings, nearest, note_text, promote, set_embedding,
    sources as list_sources, write_note, InboxItem, NewNote, NoteKind, SourceSummary,
};
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
    assert!(matches!(e, Error::Invalid(_)), "{e:?}");
    assert_eq!(e.to_string(), "source 'nosuch' is not registered");
    assert_eq!(count(&og, "Fleeting"), 0);
    assert_eq!(count(&og, "Literature"), 0);
}

#[test]
fn blank_body_is_rejected() {
    let s = TestServer::start();
    let og = s.og();
    let e = write_note(&og, &note(" \n\t ", None, None, at_ten())).unwrap_err();
    assert!(matches!(e, Error::Invalid(_)), "{e:?}");
    assert_eq!(e.to_string(), "the body is empty");
    assert_eq!(count(&og, "Fleeting"), 0);
}

#[test]
fn replaying_the_same_note_does_not_duplicate() {
    let s = TestServer::start();
    let og = s.og();
    let n = note("同じメモ", None, None, at_ten());
    let a = write_note(&og, &n).unwrap();
    let b = write_note(&og, &n).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.slug, "20261120100000");
    assert_eq!(count(&og, "Fleeting"), 1);
}

#[test]
fn replaying_a_literature_note_does_not_duplicate() {
    let s = TestServer::start();
    let og = s.og();
    let n = note("引用", Some("978-4-480-06899-8"), Some("p.1"), at_ten());
    let a = write_note(&og, &n).unwrap();
    let b = write_note(&og, &n).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.kind, NoteKind::Literature);
    assert_eq!(count(&og, "Literature"), 1);
    assert_eq!(sources(&og).len(), 1);
}

#[test]
fn same_second_with_different_body_still_advances() {
    let s = TestServer::start();
    let og = s.og();
    let a = write_note(&og, &note("一件目", None, None, at_ten())).unwrap();
    let b = write_note(&og, &note("別の本文", None, None, at_ten())).unwrap();
    assert_ne!(a.slug, b.slug);
}

#[test]
fn replaying_a_note_with_milliseconds_does_not_duplicate() {
    let s = TestServer::start();
    let og = s.og();
    let n = note("ミリ秒つき", None, None, at_ten() + chrono::Duration::milliseconds(456));
    let a = write_note(&og, &n).unwrap();
    let b = write_note(&og, &n).unwrap();
    assert_eq!(a, b);
    assert_eq!(count(&og, "Fleeting"), 1);
}

#[test]
fn concurrent_writes_in_the_same_second_get_distinct_slugs() {
    let s = TestServer::start();
    // 別のクライアント (別プロセスの zl や worker の再実行) として、同じ状態ディレクトリを指す
    let og_a = s.og();
    let og_b = s.og();
    // 競り合いは運なので、秒を変えて数回繰り返す
    for round in 0..3u32 {
        let created = Local.with_ymd_and_hms(2026, 11, 20, 10, round, 0).unwrap();
        let barrier = std::sync::Barrier::new(2);
        let (a, b) = std::thread::scope(|scope| {
            let a = scope.spawn(|| {
                barrier.wait();
                write_note(&og_a, &note(&format!("本文 A {round}"), None, None, created)).unwrap()
            });
            let b = scope.spawn(|| {
                barrier.wait();
                write_note(&og_b, &note(&format!("本文 B {round}"), None, None, created)).unwrap()
            });
            (a.join().unwrap(), b.join().unwrap())
        });
        assert_ne!(a.slug, b.slug, "round {round}");
    }
    let rows = og_a
        .query("main", "query q() { match { $f: Fleeting } return { $f.body as body } }", json!({}))
        .unwrap();
    let mut bodies: Vec<String> = rows.iter().map(|r| r["body"].as_str().unwrap().to_string()).collect();
    bodies.sort();
    let mut want: Vec<String> = (0..3).flat_map(|r| [format!("本文 A {r}"), format!("本文 B {r}")]).collect();
    want.sort();
    assert_eq!(bodies, want);
}

fn at(h: u32, m: u32) -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 11, 20, h, m, 0).unwrap()
}

fn permanents(og: &Omnigraph) -> Vec<serde_json::Map<String, Value>> {
    og.query(
        "main",
        "query q() { match { $p: Permanent } return { $p.slug as slug, $p.title as title, $p.body as body } order { $p.slug } }",
        json!({}),
    )
    .unwrap()
}

fn edge_count(og: &Omnigraph, edge: &str) -> usize {
    og.export("main")
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["edge"] == edge)
        .count()
}

#[test]
fn inbox_lists_unprocessed_oldest_first() {
    let s = TestServer::start();
    let og = s.og();
    // 書く順は created の順と違えておく
    write_note(&og, &note("二番目", None, None, at(10, 5))).unwrap();
    write_note(&og, &note("三番目", Some("978-4-480-06899-8"), None, at(10, 10))).unwrap();
    write_note(&og, &note("一番目", None, None, at(10, 0))).unwrap();
    let items = inbox(&og).unwrap();
    let bodies: Vec<&str> = items.iter().map(|i| i.body.as_str()).collect();
    assert_eq!(bodies, ["一番目", "二番目", "三番目"]);
    assert_eq!(items[0].kind, NoteKind::Fleeting);
    assert_eq!(items[2].kind, NoteKind::Literature);
    assert_eq!(items[0].slug, "20261120100000");
    assert_eq!(items[0].created, at(10, 0).with_timezone(&Utc));
    assert_eq!(items[0].source_id, None);
}

#[test]
fn inbox_shows_source_of_literature() {
    let s = TestServer::start();
    let og = s.og();
    write_note(&og, &note("本の一節", Some("978-4-480-06899-8"), Some("p.42"), at_ten())).unwrap();
    let items = inbox(&og).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, NoteKind::Literature);
    assert_eq!(items[0].source_id.as_deref(), Some("isbn:9784480068998"));
    assert_eq!(items[0].locator.as_deref(), Some("p.42"));
    // 仮の Source の slug は "" なので、出典の slug はなし
    assert_eq!(items[0].source_slug, None);
}

#[test]
fn inbox_shows_registered_source_slug_and_missing_locator() {
    let s = TestServer::start();
    let og = s.og();
    og.load(
        "main",
        &[json!({"type":"Source","data":{"id":"isbn:1","slug":"tanaka-2024","title":"S","kind":"book","input":"x","provenance":"{}"}})],
    )
    .unwrap();
    write_note(&og, &note("引用", Some("tanaka-2024"), None, at_ten())).unwrap();
    let items = inbox(&og).unwrap();
    assert_eq!(items[0].source_slug.as_deref(), Some("tanaka-2024"));
    assert_eq!(items[0].source_id.as_deref(), Some("isbn:1"));
    assert_eq!(items[0].locator, None);
}

#[test]
fn promoted_notes_leave_inbox() {
    let s = TestServer::start();
    let og = s.og();
    let f = write_note(&og, &note("メモ", None, None, at(10, 0))).unwrap();
    let l = write_note(&og, &note("引用", Some("978-4-480-06899-8"), None, at(10, 5))).unwrap();
    write_note(&og, &note("残るメモ", None, None, at(10, 10))).unwrap();
    let slug = promote(
        &og,
        &[(NoteKind::Fleeting, f.slug.clone()), (NoteKind::Literature, l.slug.clone())],
        "考え",
        "二つから生まれた考え",
        at(11, 0),
    )
    .unwrap();
    assert_eq!(slug, "20261120110000");
    let items = inbox(&og).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].body, "残るメモ");
    let p = permanents(&og);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0]["slug"], slug.as_str());
    assert_eq!(p[0]["title"], "考え");
    assert_eq!(p[0]["body"], "二つから生まれた考え");
    assert_eq!(edge_count(&og, "DerivedFrom"), 1);
    assert_eq!(edge_count(&og, "DerivedFromLit"), 1);
}

#[test]
fn promote_advances_slug_on_permanent_collision() {
    let s = TestServer::start();
    let og = s.og();
    let f1 = write_note(&og, &note("メモ一", None, None, at(10, 0))).unwrap();
    let f2 = write_note(&og, &note("メモ二", None, None, at(10, 1))).unwrap();
    let a = promote(&og, &[(NoteKind::Fleeting, f1.slug)], "一つ目", "本文一", at(11, 0)).unwrap();
    let b = promote(&og, &[(NoteKind::Fleeting, f2.slug)], "二つ目", "本文二", at(11, 0)).unwrap();
    assert_eq!(a, "20261120110000");
    assert_eq!(b, "20261120110001");
    assert_eq!(permanents(&og).len(), 2);
}

#[test]
fn promote_rejects_already_promoted_notes() {
    let s = TestServer::start();
    let og = s.og();
    let f = write_note(&og, &note("メモ", None, None, at(10, 0))).unwrap();
    let l = write_note(&og, &note("引用", Some("978-4-480-06899-8"), Some("p.1"), at(10, 5))).unwrap();
    let fresh = write_note(&og, &note("まだ", None, None, at(10, 10))).unwrap();
    let from = [(NoteKind::Fleeting, f.slug.clone()), (NoteKind::Literature, l.slug.clone())];
    promote(&og, &from, "題", "本文", at(11, 0)).unwrap();
    // 片方だけでも昇格済みなら、どちらも昇格しない
    for again in [
        vec![(NoteKind::Fleeting, f.slug.clone())],
        vec![(NoteKind::Literature, l.slug.clone())],
        vec![(NoteKind::Fleeting, fresh.slug.clone()), (NoteKind::Fleeting, f.slug.clone())],
    ] {
        let e = promote(&og, &again, "題2", "本文2", at(12, 0)).unwrap_err();
        assert!(matches!(&e, Error::Invalid(m) if m.contains("already promoted")), "{e:?}");
    }
    assert_eq!(count(&og, "Permanent"), 1);
    assert_eq!(inbox(&og).unwrap().len(), 1);
}

#[test]
fn promote_with_blank_body_writes_nothing() {
    let s = TestServer::start();
    let og = s.og();
    let f = write_note(&og, &note("メモ", None, None, at(10, 0))).unwrap();
    let from = [(NoteKind::Fleeting, f.slug)];
    for (title, body) in [("題", " \n"), (" ", "本文")] {
        let e = promote(&og, &from, title, body, at(11, 0)).unwrap_err();
        assert!(matches!(e, Error::Invalid(_)), "{e:?}");
    }
    let e = promote(&og, &[], "題", "本文", at(11, 0)).unwrap_err();
    assert!(matches!(e, Error::Invalid(_)), "{e:?}");
    assert_eq!(count(&og, "Permanent"), 0);
    assert_eq!(inbox(&og).unwrap().len(), 1);
}

#[test]
fn promote_rejects_permanent_in_from() {
    let s = TestServer::start();
    let og = s.og();
    let e = promote(&og, &[(NoteKind::Permanent, "x".to_string())], "題", "本文", at(11, 0)).unwrap_err();
    assert!(matches!(e, Error::Invalid(_)), "{e:?}");
    assert_eq!(count(&og, "Permanent"), 0);
}

#[test]
fn delete_removes_from_inbox() {
    let s = TestServer::start();
    let og = s.og();
    let f = write_note(&og, &note("メモ", None, None, at(10, 0))).unwrap();
    let l = write_note(&og, &note("引用", Some("978-4-480-06899-8"), Some("p.1"), at(10, 5))).unwrap();
    let keep = write_note(&og, &note("残す", None, None, at(10, 10))).unwrap();
    delete_notes(&og, &[(NoteKind::Fleeting, f.slug), (NoteKind::Literature, l.slug)]).unwrap();
    let items = inbox(&og).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].slug, keep.slug);
    assert_eq!(count(&og, "Literature"), 0);
    assert_eq!(edge_count(&og, "Cites"), 0);
    // 出典は残る
    assert_eq!(sources(&og).len(), 1);
}

#[test]
fn delete_nothing_is_ok() {
    let s = TestServer::start();
    let og = s.og();
    delete_notes(&og, &[]).unwrap();
}

#[test]
fn is_old_after_30_days() {
    let created = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
    let item = InboxItem {
        kind: NoteKind::Fleeting,
        slug: "s".to_string(),
        body: "b".to_string(),
        created,
        source_slug: None,
        source_id: None,
        locator: None,
    };
    assert!(!item.is_old(created + chrono::Duration::days(30)));
    assert!(item.is_old(created + chrono::Duration::days(30) + chrono::Duration::seconds(1)));
}

fn permanent(og: &Omnigraph, from: &(NoteKind, String), title: &str, body: &str, t: DateTime<Local>) -> String {
    promote(og, std::slice::from_ref(from), title, body, t).unwrap()
}

/// まだ昇格していない Fleeting を 1 件書いて、昇格元の組で返す。
fn fresh_source(og: &Omnigraph, n: u32) -> (NoteKind, String) {
    let f = write_note(og, &note(&format!("元{n}"), None, None, at(10, n))).unwrap();
    (NoteKind::Fleeting, f.slug)
}

#[test]
fn nearest_orders_by_similarity_and_skips_missing() {
    let s = TestServer::start();
    let og = s.og();
    let a = permanent(&og, &fresh_source(&og, 0), "A", "a の本文", at(11, 0));
    let b = permanent(&og, &fresh_source(&og, 1), "B", "b の本文", at(11, 1));
    let _c = permanent(&og, &fresh_source(&og, 2), "C", "c の本文", at(11, 2)); // 埋め込みなし
    set_embedding(&og, NoteKind::Permanent, &a, &unit(&[(0, 1.0)])).unwrap();
    set_embedding(&og, NoteKind::Permanent, &b, &unit(&[(0, 1.0), (1, 1.0)])).unwrap();

    let near = nearest(&og, "main", &[NoteKind::Permanent], &unit(&[(0, 1.0)]), 5, 0.0).unwrap();
    let slugs: Vec<&str> = near.iter().map(|n| n.slug.as_str()).collect();
    assert_eq!(slugs, [a.as_str(), b.as_str()]);
    assert!((near[0].similarity - 1.0).abs() < 1e-4, "{}", near[0].similarity);
    assert!(near[1].similarity < near[0].similarity);
    assert_eq!(near[0].kind, NoteKind::Permanent);
    assert_eq!(near[0].title.as_deref(), Some("A"));
    assert_eq!(near[0].body, "a の本文");
}

#[test]
fn nearest_merges_kinds_by_similarity_and_limits() {
    let s = TestServer::start();
    let og = s.og();
    let f = write_note(&og, &note("メモ", None, None, at(10, 0))).unwrap();
    let l = write_note(&og, &note("引用", Some("978-4-480-06899-8"), None, at(10, 5))).unwrap();
    set_embedding(&og, NoteKind::Fleeting, &f.slug, &unit(&[(0, 1.0), (1, 1.0)])).unwrap();
    set_embedding(&og, NoteKind::Literature, &l.slug, &unit(&[(0, 1.0)])).unwrap();
    let kinds = [NoteKind::Fleeting, NoteKind::Literature];
    let near = nearest(&og, "main", &kinds, &unit(&[(0, 1.0)]), 5, 0.0).unwrap();
    assert_eq!(near.iter().map(|n| n.kind).collect::<Vec<_>>(), [NoteKind::Literature, NoteKind::Fleeting]);
    assert_eq!(near[1].title, None);
    let one = nearest(&og, "main", &kinds, &unit(&[(0, 1.0)]), 1, 0.0).unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].kind, NoteKind::Literature);
}

#[test]
fn nearest_drops_below_min_similarity() {
    let s = TestServer::start();
    let og = s.og();
    let a = permanent(&og, &fresh_source(&og, 0), "A", "a", at(11, 0));
    let b = permanent(&og, &fresh_source(&og, 1), "B", "b", at(11, 1));
    set_embedding(&og, NoteKind::Permanent, &a, &unit(&[(0, 1.0)])).unwrap();
    set_embedding(&og, NoteKind::Permanent, &b, &unit(&[(1, 1.0)])).unwrap(); // 直交 (similarity 0)
    let near = nearest(&og, "main", &[NoteKind::Permanent], &unit(&[(0, 1.0)]), 5, 0.5).unwrap();
    assert_eq!(near.len(), 1);
    assert_eq!(near[0].slug, a);
}

#[test]
fn get_embedding_roundtrip() {
    let s = TestServer::start();
    let og = s.og();
    let f = write_note(&og, &note("メモ", None, None, at(10, 0))).unwrap();
    assert_eq!(get_embedding(&og, NoteKind::Fleeting, &f.slug).unwrap(), None);
    assert_eq!(get_embedding(&og, NoteKind::Fleeting, "nothing").unwrap(), None);
    let v = unit(&[(3, 1.0), (7, 2.0)]);
    set_embedding(&og, NoteKind::Fleeting, &f.slug, &v).unwrap();
    let got = get_embedding(&og, NoteKind::Fleeting, &f.slug).unwrap().unwrap();
    assert_eq!(got.len(), 1024);
    assert!(got.iter().zip(&v).all(|(x, y)| (x - y).abs() < 1e-6));
}

#[test]
fn note_text_by_kind() {
    let s = TestServer::start();
    let og = s.og();
    let f = write_note(&og, &note("メモの本文", None, None, at(10, 0))).unwrap();
    let from = (NoteKind::Fleeting, f.slug.clone());
    let p = permanent(&og, &from, "題名", "本文", at(11, 0));
    assert_eq!(note_text(&og, NoteKind::Fleeting, &f.slug).unwrap().as_deref(), Some("メモの本文"));
    assert_eq!(note_text(&og, NoteKind::Permanent, &p).unwrap().as_deref(), Some("題名\n本文"));
    assert_eq!(note_text(&og, NoteKind::Permanent, "nothing").unwrap(), None);
}

#[test]
fn inbox_embeddings_skip_promoted_and_unembedded() {
    let s = TestServer::start();
    let og = s.og();
    let f1 = write_note(&og, &note("埋め込みあり", None, None, at(10, 0))).unwrap();
    let _f2 = write_note(&og, &note("埋め込みなし", None, None, at(10, 1))).unwrap();
    let f3 = write_note(&og, &note("昇格済み", None, None, at(10, 2))).unwrap();
    let v1 = unit(&[(0, 1.0)]);
    let v3 = unit(&[(1, 1.0)]);
    set_embedding(&og, NoteKind::Fleeting, &f1.slug, &v1).unwrap();
    set_embedding(&og, NoteKind::Fleeting, &f3.slug, &v3).unwrap();
    promote(&og, &[(NoteKind::Fleeting, f3.slug)], "題", "本文", at(11, 0)).unwrap();

    let got = inbox_embeddings(&og).unwrap();
    assert_eq!(got.len(), 1);
    let got_v1 = got.get(&(NoteKind::Fleeting, f1.slug)).unwrap();
    assert_eq!(got_v1.len(), 1024);
    assert!(got_v1.iter().zip(&v1).all(|(x, y)| (x - y).abs() < 1e-6));
}

#[test]
fn sources_list_registered_slugs_only() {
    let s = TestServer::start();
    let og = s.og();
    og.load(
        "main",
        &[json!({"type":"Source","data":{"id":"isbn:1","slug":"ahrens2017","title":"S","kind":"book","input":"x","provenance":"{}"}})],
    )
    .unwrap();
    // 仮の Source (slug 空) を作る
    write_note(&og, &note("引用", Some("978-4-480-06899-8"), None, at_ten())).unwrap();

    assert_eq!(
        list_sources(&og).unwrap(),
        vec![SourceSummary { slug: "ahrens2017".to_string(), kind: "book".to_string() }]
    );
}
