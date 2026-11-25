mod common;

use chrono::{DateTime, Local, TimeZone};
use common::TestServer;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};
use zettelist::notes::{promote, write_note, NewNote, NoteKind};
use zettelist::omnigraph::Omnigraph;
use zettelist::propose::ensure_review;
use zettelist::queue::{Job, Queue};
use zettelist::review::{apply, hubs, list_proposals, rehub, unrehub, Decision, HubSummary, Proposal, ReviewState, Summary};

fn at(h: u32, m: u32) -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 11, 20, h, m, 0).unwrap()
}

/// 昇格元の Fleeting を 1 件書いて Permanent を作る (埋め込みは付けない)。
fn permanent(og: &Omnigraph, title: &str, minute: u32) -> String {
    let f = write_note(
        og,
        &NewNote { body: format!("{title} の元"), source: None, locator: None, created: at(9, minute) },
    )
    .unwrap();
    promote(og, &[(NoteKind::Fleeting, f.slug)], title, &format!("{title} の本文"), at(11, minute)).unwrap()
}

/// review に Links の提案を書く。
fn propose_link(og: &Omnigraph, from: &str, to: &str, similarity: f32) -> Proposal {
    ensure_review(og).unwrap();
    og.load("review", &[json!({"edge":"Links","from":from,"to":to,"data":{"similarity":similarity}})]).unwrap();
    Proposal::Link { from: from.to_string(), to: to.to_string(), similarity }
}

fn propose_hub(og: &Omnigraph, hub: &str, note: &str) -> Proposal {
    ensure_review(og).unwrap();
    og.load("review", &[json!({"edge":"Includes","from":hub,"to":note})]).unwrap();
    Proposal::Hub { hub: hub.to_string(), note: note.to_string() }
}

/// ブランチの Links を (from, to, reason) で返す。
fn links(og: &Omnigraph, branch: &str) -> Vec<(String, String, Option<String>)> {
    let rows = og
        .query(
            branch,
            "query q() { match { $a: Permanent $a $l:links $b } \
             return { $a.slug as src, $b.slug as dst, $l.reason as reason } }",
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

fn includes(og: &Omnigraph, branch: &str) -> Vec<(String, String)> {
    let rows = og
        .query(
            branch,
            "query q() { match { $h: Hub $h includes $p } return { $h.slug as hub, $p.slug as perm } }",
            json!({}),
        )
        .unwrap();
    rows.iter().map(|r| (r["hub"].as_str().unwrap().to_string(), r["perm"].as_str().unwrap().to_string())).collect()
}

fn adopt() -> Decision {
    Decision::Adopt { reason: None }
}

fn state_path(s: &TestServer) -> std::path::PathBuf {
    s.state_dir().join("review.json")
}

fn save_decisions(path: &std::path::Path, decisions: Vec<(&Proposal, Decision)>) {
    let mut st = ReviewState::load(path).unwrap();
    for (p, d) in decisions {
        st.decisions.insert(p.key(), d);
    }
    st.save(path).unwrap();
}

#[test]
fn lists_only_proposals_not_in_main() {
    let s = TestServer::start();
    let og = s.og();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    let c = permanent(&og, "C", 2);
    og.load("main", &[json!({"edge":"Links","from":a,"to":b,"data":{"similarity":0.9}})]).unwrap();
    ensure_review(&og).unwrap();
    propose_link(&og, &a, &c, 0.6);
    let near = propose_link(&og, &b, &c, 0.8);

    let got = list_proposals(&og).unwrap();
    assert_eq!(got.len(), 2, "{got:?}");
    // 近い順
    assert_eq!(got[0].key(), near.key());
    assert_eq!(got[1].key(), format!("link:{a}:{c}"));
}

#[test]
fn links_come_before_hubs_in_list() {
    let s = TestServer::start();
    let og = s.og();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    og.load("main", &[json!({"type":"Hub","data":{"slug":"hub-x","title":"ハブ X"}})]).unwrap();
    propose_hub(&og, "hub-x", &a);
    propose_link(&og, &a, &b, 0.7);

    let got = list_proposals(&og).unwrap();
    assert_eq!(got.len(), 2);
    assert!(matches!(got[0], Proposal::Link { .. }), "{got:?}");
    assert_eq!(got[1], Proposal::Hub { hub: "hub-x".into(), note: a.clone() });
    assert_eq!(got[1].key(), format!("hub:hub-x:{a}"));
}

#[test]
fn apply_merges_adopted_and_drops_rejected() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    let c = permanent(&og, "C", 2);
    let p1 = propose_link(&og, &a, &b, 0.9);
    let p2 = propose_link(&og, &a, &c, 0.8);
    save_decisions(&state_path(&s), vec![(&p1, adopt()), (&p2, Decision::Reject)]);

    let summary = apply(&og, &state_path(&s), &queue).unwrap();
    assert_eq!(summary, Summary { links: 1, hubs: 0 });
    assert_eq!(links(&og, "main"), [(a.clone(), b.clone(), None)]);
    assert!(list_proposals(&og).unwrap().is_empty());
    // 判断は片付く
    assert_eq!(ReviewState::load(&state_path(&s)).unwrap(), ReviewState::default());
}

#[test]
fn adopt_with_reason_sets_reason_in_main() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    let p = propose_link(&og, &a, &b, 0.9);
    save_decisions(&state_path(&s), vec![(&p, Decision::Adopt { reason: Some("粒度の話".into()) })]);

    apply(&og, &state_path(&s), &queue).unwrap();
    assert_eq!(links(&og, "main"), [(a.clone(), b.clone(), Some("粒度の話".to_string()))]);
    // 類似度は元のまま
    let rows = og
        .query("main", "query q() { match { $a: Permanent $a $l:links $b } return { $l.similarity as sim } }", json!({}))
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!((rows[0]["sim"].as_f64().unwrap() - 0.9).abs() < 1e-3);
}

#[test]
fn undecided_stays_in_review() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    let c = permanent(&og, "C", 2);
    let d = permanent(&og, "D", 3);
    let p1 = propose_link(&og, &a, &b, 0.9);
    let p2 = propose_link(&og, &a, &c, 0.8);
    let p3 = propose_link(&og, &a, &d, 0.7);
    save_decisions(&state_path(&s), vec![(&p1, adopt())]);

    apply(&og, &state_path(&s), &queue).unwrap();
    assert_eq!(links(&og, "main"), [(a.clone(), b.clone(), None)]);
    let left: Vec<String> = list_proposals(&og).unwrap().iter().map(Proposal::key).collect();
    assert_eq!(left, [p2.key(), p3.key()]);
}

#[test]
fn apply_twice_in_a_row_works() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    let p1 = propose_link(&og, &a, &b, 0.9);
    save_decisions(&state_path(&s), vec![(&p1, adopt())]);
    apply(&og, &state_path(&s), &queue).unwrap();

    // main が進み、新しい提案が来る
    let c = permanent(&og, "C", 2);
    let p2 = propose_link(&og, &a, &c, 0.8);
    save_decisions(&state_path(&s), vec![(&p2, adopt())]);
    let summary = apply(&og, &state_path(&s), &queue).unwrap();
    assert_eq!(summary.links, 1);
    assert_eq!(links(&og, "main"), [(a.clone(), b.clone(), None), (a.clone(), c.clone(), None)]);
    assert!(list_proposals(&og).unwrap().is_empty());
}

#[test]
fn rerun_after_crash_keeps_undecided() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    let c = permanent(&og, "C", 2);
    let d = permanent(&og, "D", 3);
    let p1 = propose_link(&og, &a, &b, 0.9);
    let p2 = propose_link(&og, &a, &c, 0.8);
    let p3 = propose_link(&og, &a, &d, 0.7);

    // 手順 3 のあとで止まった状態: 未判断の 2 本は review から消え、reinsert にだけ残る
    let path = state_path(&s);
    let mut st = ReviewState::load(&path).unwrap();
    st.decisions.insert(p1.key(), adopt());
    st.reinsert = vec![p2.clone(), p3.clone()];
    st.save(&path).unwrap();
    for (x, y) in [(&a, &c), (&a, &d)] {
        og.mutate(
            "review",
            "query d($a: String, $b: String) { delete Links where @src = $a and @dst = $b }",
            json!({"a": x, "b": y}),
        )
        .unwrap();
    }
    assert_eq!(list_proposals(&og).unwrap().len(), 1);

    apply(&og, &path, &queue).unwrap();
    assert_eq!(links(&og, "main"), [(a.clone(), b.clone(), None)]);
    let left: Vec<String> = list_proposals(&og).unwrap().iter().map(Proposal::key).collect();
    assert_eq!(left, [p2.key(), p3.key()]);
    assert_eq!(ReviewState::load(&path).unwrap(), ReviewState::default());
}

#[test]
fn rerun_after_merge_still_restores_undecided() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    let c = permanent(&og, "C", 2);
    let p1 = propose_link(&og, &a, &b, 0.9);
    let p2 = propose_link(&og, &a, &c, 0.8);

    // 手順 5 (merge) のあとで止まった状態
    let path = state_path(&s);
    let mut st = ReviewState::load(&path).unwrap();
    st.decisions.insert(p1.key(), Decision::Adopt { reason: Some("理由".into()) });
    st.reinsert = vec![p2.clone()];
    st.save(&path).unwrap();
    og.mutate(
        "review",
        "query d($a: String, $b: String) { delete Links where @src = $a and @dst = $b }",
        json!({"a": a, "b": c}),
    )
    .unwrap();
    og.statement("branch merge review into main").unwrap();

    let summary = apply(&og, &path, &queue).unwrap();
    assert_eq!(summary.links, 1);
    assert_eq!(links(&og, "main"), [(a.clone(), b.clone(), None)]);
    let left: Vec<String> = list_proposals(&og).unwrap().iter().map(Proposal::key).collect();
    assert_eq!(left, [p2.key()]);
}

#[test]
fn hub_proposal_is_adopted_and_counted() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    og.load("main", &[json!({"type":"Hub","data":{"slug":"hub-x","title":"ハブ X"}})]).unwrap();
    let h = propose_hub(&og, "hub-x", &a);
    let l = propose_link(&og, &a, &b, 0.9);
    save_decisions(&state_path(&s), vec![(&h, adopt()), (&l, adopt())]);

    let summary = apply(&og, &state_path(&s), &queue).unwrap();
    assert_eq!(summary.message(), "merge: links 1, hub 1");
    assert_eq!(includes(&og, "main"), [("hub-x".to_string(), a.clone())]);
    assert!(list_proposals(&og).unwrap().is_empty());
}

#[test]
fn summary_message_skips_zero() {
    assert_eq!(Summary { links: 3, hubs: 0 }.message(), "merge: links 3");
    assert_eq!(Summary { links: 3, hubs: 1 }.message(), "merge: links 3, hub 1");
    assert_eq!(Summary { links: 0, hubs: 0 }.message(), "merge");
}

#[test]
fn state_file_is_human_readable_and_missing_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("review.json");
    assert_eq!(ReviewState::load(&path).unwrap(), ReviewState::default());
    let st = ReviewState {
        decisions: BTreeMap::from([
            ("link:a:b".to_string(), Decision::Adopt { reason: Some("粒度の話".into()) }),
            ("hub:h:a".to_string(), Decision::Reject),
        ]),
        reinsert: vec![Proposal::Link { from: "a".into(), to: "c".into(), similarity: 0.5 }],
    };
    st.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("粒度の話"), "{text}");
    assert_eq!(ReviewState::load(&path).unwrap(), st);
    // 一時ファイルを残さない
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn apply_queues_backup() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    let p = propose_link(&og, &a, &b, 0.9);
    save_decisions(&state_path(&s), vec![(&p, adopt())]);

    apply(&og, &state_path(&s), &queue).unwrap();
    let e = queue.next_ready(SystemTime::now() + Duration::from_secs(1)).unwrap().unwrap();
    assert_eq!(e.job, Job::Backup { message: "merge: links 1".to_string() });
}

/// 手順 2 の save のあとで止まった状態: X は未判断として reinsert にあり、review にもまだある。
/// そのあとで X に判断が付いた。
fn crashed_with_x_in_reinsert(s: &TestServer, remove_from_review: bool) -> (Omnigraph, Queue, String, String, Proposal) {
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    let x = propose_link(&og, &a, &b, 0.9);
    let path = state_path(s);
    let mut st = ReviewState::load(&path).unwrap();
    st.reinsert = vec![x.clone()];
    st.save(&path).unwrap();
    if remove_from_review {
        og.mutate(
            "review",
            "query d($a: String, $b: String) { delete Links where @src = $a and @dst = $b }",
            json!({"a": a, "b": b}),
        )
        .unwrap();
    }
    (og, queue, a, b, x)
}

#[test]
fn adopted_after_crash_wins_over_stale_reinsert_still_in_review() {
    let s = TestServer::start();
    let (og, queue, a, b, x) = crashed_with_x_in_reinsert(&s, false);
    save_decisions(&state_path(&s), vec![(&x, Decision::Adopt { reason: Some("あとから".into()) })]);

    let summary = apply(&og, &state_path(&s), &queue).unwrap();
    assert_eq!(summary.links, 1);
    assert_eq!(links(&og, "main"), [(a, b, Some("あとから".to_string()))]);
    assert!(list_proposals(&og).unwrap().is_empty());
    assert_eq!(ReviewState::load(&state_path(&s)).unwrap(), ReviewState::default());
}

#[test]
fn adopted_after_crash_is_restored_when_already_deleted_from_review() {
    let s = TestServer::start();
    let (og, queue, a, b, x) = crashed_with_x_in_reinsert(&s, true);
    assert!(list_proposals(&og).unwrap().is_empty());
    save_decisions(&state_path(&s), vec![(&x, Decision::Adopt { reason: Some("戻して採用".into()) })]);

    let summary = apply(&og, &state_path(&s), &queue).unwrap();
    assert_eq!(summary.links, 1);
    // 元の類似度で書き戻され、理由も付く
    assert_eq!(links(&og, "main"), [(a.clone(), b.clone(), Some("戻して採用".to_string()))]);
    let rows = og
        .query("main", "query q() { match { $a: Permanent $a $l:links $b } return { $l.similarity as sim } }", json!({}))
        .unwrap();
    assert!((rows[0]["sim"].as_f64().unwrap() - 0.9).abs() < 1e-3);
    assert!(list_proposals(&og).unwrap().is_empty());
}

#[test]
fn rejected_after_crash_is_neither_in_main_nor_back_in_review() {
    for remove_from_review in [false, true] {
        let s = TestServer::start();
        let (og, queue, _a, _b, x) = crashed_with_x_in_reinsert(&s, remove_from_review);
        save_decisions(&state_path(&s), vec![(&x, Decision::Reject)]);

        apply(&og, &state_path(&s), &queue).unwrap();
        assert!(links(&og, "main").is_empty());
        assert!(list_proposals(&og).unwrap().is_empty(), "remove_from_review={remove_from_review}");
        assert_eq!(ReviewState::load(&state_path(&s)).unwrap(), ReviewState::default());
    }
}

#[test]
fn apply_waits_while_review_lock_is_held() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let path = state_path(&s);
    let lock_path = path.with_file_name("review.lock");
    let done = AtomicBool::new(false);

    let held = zettelist::review::lock_review(&lock_path).unwrap();
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            apply(&og, &path, &queue).unwrap();
            done.store(true, Ordering::SeqCst);
        });
        std::thread::sleep(Duration::from_millis(700));
        assert!(!done.load(Ordering::SeqCst), "ロックを持たれたまま apply が進んだ");
        drop(held);
        worker.join().unwrap();
    });
    assert!(done.load(Ordering::SeqCst));
}

/// main から Permanent を消す。
fn delete_permanent(og: &Omnigraph, slug: &str) {
    zettelist::notes::delete_notes(og, &[(NoteKind::Permanent, slug.to_string())]).unwrap();
}

#[test]
fn undecided_reinsert_with_deleted_endpoint_is_dropped() {
    let s = TestServer::start();
    // 手順 3 のあと (review からは消えて reinsert にだけある) に、相手の Permanent が main から消えた
    let (og, queue, _a, b, _x) = crashed_with_x_in_reinsert(&s, true);
    delete_permanent(&og, &b);

    apply(&og, &state_path(&s), &queue).unwrap();
    assert!(links(&og, "main").is_empty());
    assert!(list_proposals(&og).unwrap().is_empty());
    assert_eq!(ReviewState::load(&state_path(&s)).unwrap(), ReviewState::default());
}

#[test]
fn adopted_reinsert_with_deleted_endpoint_is_dropped() {
    let s = TestServer::start();
    let (og, queue, _a, b, x) = crashed_with_x_in_reinsert(&s, true);
    save_decisions(&state_path(&s), vec![(&x, adopt())]);
    delete_permanent(&og, &b);

    let summary = apply(&og, &state_path(&s), &queue).unwrap();
    assert_eq!(summary.links, 0);
    assert!(links(&og, "main").is_empty());
    assert!(list_proposals(&og).unwrap().is_empty());
    assert_eq!(ReviewState::load(&state_path(&s)).unwrap(), ReviewState::default());
}

/// main から消す (Studio で消したのと同じ)。つながるエッジは main から一緒に消える。
fn delete_on_main(og: &Omnigraph, node: &str, slug: &str) {
    let q = format!("query d($s: String) {{ delete {node} where slug = $s }}");
    og.mutate("main", &q, json!({"s": slug})).unwrap();
}

#[test]
fn apply_survives_permanent_deleted_on_main() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    let c = permanent(&og, "C", 2);
    let keep = propose_link(&og, &a, &b, 0.9);
    let dead_adopted = propose_link(&og, &a, &c, 0.8);
    let _dead_undecided = propose_link(&og, &b, &c, 0.7);
    save_decisions(&state_path(&s), vec![(&keep, adopt()), (&dead_adopted, adopt())]);
    delete_on_main(&og, "Permanent", &c);

    let summary = apply(&og, &state_path(&s), &queue).unwrap();
    assert_eq!(summary, Summary { links: 1, hubs: 0 });
    assert_eq!(links(&og, "main"), [(a.clone(), b.clone(), None)]);
    assert!(list_proposals(&og).unwrap().is_empty());
    // 2 回目も通る
    apply(&og, &state_path(&s), &queue).unwrap();
}

#[test]
fn apply_survives_hub_deleted_on_main() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    og.load("main", &[json!({"type":"Hub","data":{"slug":"hub-x","title":"ハブ X"}})]).unwrap();
    let _h = propose_hub(&og, "hub-x", &a);
    delete_on_main(&og, "Hub", "hub-x");

    apply(&og, &state_path(&s), &queue).unwrap();
    assert!(includes(&og, "main").is_empty());
    assert!(list_proposals(&og).unwrap().is_empty());
}

#[test]
fn proposing_after_deletion_still_works() {
    let s = TestServer::start();
    let og = s.og();
    let a = permanent(&og, "A", 0);
    let b = permanent(&og, "B", 1);
    let c = permanent(&og, "C", 2);
    propose_link(&og, &a, &c, 0.8);
    delete_on_main(&og, "Permanent", &c);
    // worker の提案も ensure_review を通る
    let p = propose_link(&og, &a, &b, 0.9);
    assert_eq!(list_proposals(&og).unwrap(), [p]);
}

#[test]
fn apply_survives_permanent_recreated_with_same_slug() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let a = permanent(&og, "A", 0);
    let c = permanent(&og, "C", 2);
    let p = propose_link(&og, &a, &c, 0.8);
    save_decisions(&state_path(&s), vec![(&p, adopt())]);
    delete_on_main(&og, "Permanent", &c);
    og.load(
        "main",
        &[json!({"type":"Permanent","data":{"slug":c,"title":"C2","body":"作り直し","created":"2026-11-20T12:00:00Z"}})],
    )
    .unwrap();

    // エッジは slug で結ばれるので、作り直したノードに付く (orphan_edge にならない)
    let summary = apply(&og, &state_path(&s), &queue).unwrap();
    assert_eq!(summary, Summary { links: 1, hubs: 0 });
    assert_eq!(links(&og, "main"), [(a, c, None)]);
}

#[test]
fn rehub_adopts_new_hub_and_rejects_old() {
    let s = TestServer::start();
    let og = s.og();
    let queue = Queue::open(s.state_dir()).unwrap();
    let p = permanent(&og, "P", 0);
    og.load(
        "main",
        &[
            json!({"type":"Hub","data":{"slug":"h1","title":"ハブ 1"}}),
            json!({"type":"Hub","data":{"slug":"h2","title":"ハブ 2"}}),
        ],
    )
    .unwrap();
    propose_hub(&og, "h1", &p);

    let path = state_path(&s);
    let lock = path.with_file_name("review.lock");
    let old_key = format!("hub:h1:{p}");
    let new_key = format!("hub:h2:{p}");
    let r = rehub(&og, &path, &lock, &old_key, "h2").unwrap();
    assert!(r.created);
    assert_eq!(r.old_key, old_key);
    assert_eq!(r.new_key, new_key);
    assert_eq!(r.previous, None);

    let props = list_proposals(&og).unwrap();
    assert!(props.iter().any(|x| x.key() == new_key), "{props:?}");
    let st = ReviewState::load(&path).unwrap();
    assert_eq!(st.decisions.get(&old_key), Some(&Decision::Reject));
    assert_eq!(st.decisions.get(&new_key), Some(&Decision::Adopt { reason: None }));

    apply(&og, &path, &queue).unwrap();
    assert_eq!(includes(&og, "main"), vec![("h2".to_string(), p)]);
}

#[test]
fn unrehub_restores_previous_state() {
    let s = TestServer::start();
    let og = s.og();
    let p = permanent(&og, "P", 0);
    og.load(
        "main",
        &[
            json!({"type":"Hub","data":{"slug":"h1","title":"ハブ 1"}}),
            json!({"type":"Hub","data":{"slug":"h2","title":"ハブ 2"}}),
        ],
    )
    .unwrap();
    let prop = propose_hub(&og, "h1", &p);
    let path = state_path(&s);
    let lock = path.with_file_name("review.lock");
    save_decisions(&path, vec![(&prop, adopt())]);
    let before = ReviewState::load(&path).unwrap();

    let r = rehub(&og, &path, &lock, &prop.key(), "h2").unwrap();
    assert_eq!(r.previous, Some(adopt()));
    unrehub(&og, &path, &lock, &r).unwrap();

    let props = list_proposals(&og).unwrap();
    assert!(!props.iter().any(|x| x.key() == format!("hub:h2:{p}")), "{props:?}");
    assert_eq!(ReviewState::load(&path).unwrap(), before);
}

#[test]
fn unrehub_restores_rejected_existing_destination() {
    let s = TestServer::start();
    let og = s.og();
    let p = permanent(&og, "P", 0);
    og.load(
        "main",
        &[
            json!({"type":"Hub","data":{"slug":"h1","title":"ハブ 1"}}),
            json!({"type":"Hub","data":{"slug":"h2","title":"ハブ 2"}}),
        ],
    )
    .unwrap();
    let p1 = propose_hub(&og, "h1", &p);
    let p2 = propose_hub(&og, "h2", &p);
    let path = state_path(&s);
    let lock = path.with_file_name("review.lock");
    save_decisions(&path, vec![(&p2, Decision::Reject)]);
    let before = ReviewState::load(&path).unwrap();

    let r = rehub(&og, &path, &lock, &p1.key(), "h2").unwrap();
    assert!(!r.created);
    assert_eq!(r.new_previous, Some(Decision::Reject));
    unrehub(&og, &path, &lock, &r).unwrap();

    assert_eq!(ReviewState::load(&path).unwrap(), before);
}

#[test]
fn unrehub_deletes_created_proposal_when_hub_slug_has_colon() {
    let s = TestServer::start();
    let og = s.og();
    let p = permanent(&og, "P", 0);
    og.load(
        "main",
        &[
            json!({"type":"Hub","data":{"slug":"h1","title":"ハブ 1"}}),
            json!({"type":"Hub","data":{"slug":"h:2","title":"ハブ 2"}}),
        ],
    )
    .unwrap();
    let prop = propose_hub(&og, "h1", &p);
    let path = state_path(&s);
    let lock = path.with_file_name("review.lock");

    let r = rehub(&og, &path, &lock, &prop.key(), "h:2").unwrap();
    assert!(r.created);
    unrehub(&og, &path, &lock, &r).unwrap();

    let props = list_proposals(&og).unwrap();
    assert!(!props.iter().any(|x| x.key() == format!("hub:h:2:{p}")), "{props:?}");
}

#[test]
fn rehub_rejects_unknown_hub_and_same_hub() {
    let s = TestServer::start();
    let og = s.og();
    let p = permanent(&og, "P", 0);
    og.load("main", &[json!({"type":"Hub","data":{"slug":"h1","title":"ハブ 1"}})]).unwrap();
    let prop = propose_hub(&og, "h1", &p);
    let path = state_path(&s);
    let lock = path.with_file_name("review.lock");

    for bad in ["nosuch", "h1"] {
        let e = rehub(&og, &path, &lock, &prop.key(), bad).unwrap_err();
        assert!(matches!(e, zettelist::error::Error::Invalid(_)), "{bad}: {e:?}");
        assert_eq!(ReviewState::load(&path).unwrap(), ReviewState::default());
    }
}

#[test]
fn hubs_are_sorted_by_title() {
    let s = TestServer::start();
    let og = s.og();
    og.load(
        "main",
        &[
            json!({"type":"Hub","data":{"slug":"h-b","title":"B ハブ"}}),
            json!({"type":"Hub","data":{"slug":"h-a","title":"A ハブ"}}),
        ],
    )
    .unwrap();
    assert_eq!(
        hubs(&og).unwrap(),
        vec![
            HubSummary { slug: "h-a".to_string(), title: "A ハブ".to_string() },
            HubSummary { slug: "h-b".to_string(), title: "B ハブ".to_string() },
        ]
    );
}
