mod common;

use chrono::{DateTime, Local, TimeZone};
use common::{unit, FixedEmbedder, TestServer};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use zettelist::config::LinkConfig;
use zettelist::error::Error;
use zettelist::notes::{promote, set_embedding, write_note, NewNote, NoteKind};
use zettelist::omnigraph::Omnigraph;
use zettelist::propose::{ensure_review, propose_for};

fn at(h: u32, m: u32) -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 11, 20, h, m, 0).unwrap()
}

/// 昇格元の Fleeting を 1 件書き、Permanent を作って埋め込みを付ける。
fn permanent(og: &Omnigraph, title: &str, minute: u32, v: &[f32]) -> String {
    let f = write_note(
        og,
        &NewNote { body: format!("{title} の元"), source: None, locator: None, created: at(9, minute) },
    )
    .unwrap();
    let slug = promote(og, &[(NoteKind::Fleeting, f.slug)], title, &format!("{title} の本文"), at(11, minute)).unwrap();
    set_embedding(og, NoteKind::Permanent, &slug, v).unwrap();
    slug
}

fn cfg(candidates: usize, min_similarity: f32) -> LinkConfig {
    LinkConfig { candidates, min_similarity, inbox_similarity: 0.0 }
}

fn no_embedder() -> FixedEmbedder {
    FixedEmbedder(HashMap::new())
}

/// ブランチの Links を (from, to, similarity) で返す。
fn links(og: &Omnigraph, branch: &str) -> Vec<(String, String, f64)> {
    let rows = og
        .query(
            branch,
            "query q() { match { $a: Permanent $a $l:links $b } \
             return { $a.slug as src, $b.slug as dst, $l.similarity as sim } }",
            json!({}),
        )
        .unwrap();
    rows.iter()
        .map(|r| {
            (
                r["src"].as_str().unwrap().to_string(),
                r["dst"].as_str().unwrap().to_string(),
                r["sim"].as_f64().unwrap(),
            )
        })
        .collect()
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

fn add_hub(og: &Omnigraph, slug: &str, title: &str) {
    og.load("main", &[json!({"type":"Hub","data":{"slug":slug,"title":title}})]).unwrap();
}

#[test]
fn proposes_top_candidates_to_review_only() {
    let s = TestServer::start();
    let og = s.og();
    let target = permanent(&og, "対象", 0, &unit(&[(0, 1.0)]));
    let near = permanent(&og, "近い", 1, &unit(&[(0, 1.0), (1, 0.1)]));
    let mid = permanent(&og, "中くらい", 2, &unit(&[(0, 1.0), (1, 1.0)]));
    let _far = permanent(&og, "遠い", 3, &unit(&[(0, 1.0), (1, 3.0)]));

    let p = propose_for(&og, &no_embedder(), &cfg(2, 0.0), &target).unwrap();
    assert_eq!(p.links, 2);
    assert!(!p.hub);

    let got = links(&og, "review");
    let mut tos: Vec<&str> = got.iter().map(|(_, to, _)| to.as_str()).collect();
    tos.sort();
    let mut want = vec![near.as_str(), mid.as_str()];
    want.sort();
    assert_eq!(tos, want);
    for (from, to, sim) in &got {
        assert_eq!(from, &target);
        let expected = if to == &near { 0.995 } else { std::f64::consts::FRAC_1_SQRT_2 };
        assert!((sim - expected).abs() < 1e-3, "{to}: {sim}");
    }
    assert!(links(&og, "main").is_empty());
}

#[test]
fn nothing_above_threshold_proposes_nothing() {
    let s = TestServer::start();
    let og = s.og();
    let target = permanent(&og, "対象", 0, &unit(&[(0, 1.0)]));
    permanent(&og, "直交 1", 1, &unit(&[(1, 1.0)]));
    permanent(&og, "直交 2", 2, &unit(&[(2, 1.0)]));

    let p = propose_for(&og, &no_embedder(), &cfg(5, 0.5), &target).unwrap();
    assert_eq!(p.links, 0);
    assert!(links(&og, "review").is_empty());
}

#[test]
fn existing_link_is_not_proposed_again() {
    let s = TestServer::start();
    let og = s.og();
    let a = permanent(&og, "A", 0, &unit(&[(0, 1.0)]));
    let b = permanent(&og, "B", 1, &unit(&[(0, 1.0), (1, 0.1)]));
    let c = permanent(&og, "C", 2, &unit(&[(0, 1.0), (1, 1.0)]));
    og.load("main", &[json!({"edge":"Links","from":b,"to":a,"data":{"reason":"自分で書いた","similarity":0.5}})])
        .unwrap();

    // 既存のリンクで枠が埋まらず、残りの候補から candidates 件が出る
    let p = propose_for(&og, &no_embedder(), &cfg(1, 0.0), &a).unwrap();
    assert_eq!(p.links, 1);
    let got = links(&og, "review");
    let proposed: Vec<(&str, &str)> = got.iter().map(|(f, t, _)| (f.as_str(), t.as_str())).collect();
    // review には main から取り込んだ b -> a があり、新しい提案は a -> c だけ
    assert!(proposed.contains(&(a.as_str(), c.as_str())), "{proposed:?}");
    assert!(!proposed.contains(&(a.as_str(), b.as_str())), "{proposed:?}");
    // main の理由は上書きされない
    let main = og
        .query("main", "query q() { match { $a: Permanent $a $l:links $b } return { $l.reason as reason } }", json!({}))
        .unwrap();
    assert_eq!(main[0]["reason"], "自分で書いた");
}

#[test]
fn proposing_twice_does_not_duplicate() {
    let s = TestServer::start();
    let og = s.og();
    let a = permanent(&og, "A", 0, &unit(&[(0, 1.0)]));
    let b = permanent(&og, "B", 1, &unit(&[(0, 1.0), (1, 0.1)]));
    let c = permanent(&og, "C", 2, &unit(&[(0, 1.0), (1, 1.0)]));

    let first = propose_for(&og, &no_embedder(), &cfg(5, 0.0), &a).unwrap();
    assert_eq!(first.links, 2);
    let again = propose_for(&og, &no_embedder(), &cfg(5, 0.0), &a).unwrap();
    assert_eq!(again.links, 0);
    assert_eq!(links(&og, "review").len(), 2);

    // 逆向きに同じ組は出さない (a -> b はあるので、b の提案は c だけ)
    let from_b = propose_for(&og, &no_embedder(), &cfg(5, 0.0), &b).unwrap();
    assert_eq!(from_b.links, 1);
    let pairs: HashSet<(String, String)> = links(&og, "review").into_iter().map(|(f, t, _)| (f, t)).collect();
    assert_eq!(pairs.len(), 3);
    assert!(pairs.contains(&(b.clone(), c.clone())));
    assert!(!pairs.contains(&(b, a)));
}

#[test]
fn sees_permanent_added_to_main_after_review_was_created() {
    let s = TestServer::start();
    let og = s.og();
    let a = permanent(&og, "A", 0, &unit(&[(0, 1.0)]));
    ensure_review(&og).unwrap();
    assert!(og.branches().unwrap().contains(&"review".to_string()));

    let late = permanent(&og, "後から", 5, &unit(&[(0, 1.0), (1, 0.1)]));
    let p = propose_for(&og, &no_embedder(), &cfg(5, 0.0), &a).unwrap();
    assert_eq!(p.links, 1);
    let got = links(&og, "review");
    assert_eq!((got[0].0.as_str(), got[0].1.as_str()), (a.as_str(), late.as_str()));
}

#[test]
fn proposes_nearest_hub() {
    let s = TestServer::start();
    let og = s.og();
    let a = permanent(&og, "A", 0, &unit(&[(0, 1.0)]));
    add_hub(&og, "hub-x", "ハブ X");
    add_hub(&og, "hub-y", "ハブ Y");
    let embedder = FixedEmbedder(HashMap::from([
        ("ハブ X".to_string(), unit(&[(1, 1.0)])),
        ("ハブ Y".to_string(), unit(&[(0, 1.0), (1, 0.2)])),
    ]));

    let p = propose_for(&og, &embedder, &cfg(5, 0.5), &a).unwrap();
    assert!(p.hub);
    assert_eq!(includes(&og, "review"), [("hub-y".to_string(), a.clone())]);
    assert!(includes(&og, "main").is_empty());

    // すでに review でその Hub に入っていれば、もう書かない
    let again = propose_for(&og, &embedder, &cfg(5, 0.5), &a).unwrap();
    assert!(!again.hub);
    assert_eq!(includes(&og, "review").len(), 1);
}

#[test]
fn hub_below_threshold_or_missing_is_skipped() {
    let s = TestServer::start();
    let og = s.og();
    let a = permanent(&og, "A", 0, &unit(&[(0, 1.0)]));
    // Hub がなければ embedder は呼ばれない (no_embedder は知らない文で失敗する)
    assert!(!propose_for(&og, &no_embedder(), &cfg(5, 0.5), &a).unwrap().hub);

    add_hub(&og, "hub-x", "ハブ X");
    let embedder = FixedEmbedder(HashMap::from([("ハブ X".to_string(), unit(&[(1, 1.0)]))]));
    assert!(!propose_for(&og, &embedder, &cfg(5, 0.5), &a).unwrap().hub);
    assert!(includes(&og, "review").is_empty());
}

#[test]
fn hub_already_including_in_main_is_not_proposed() {
    let s = TestServer::start();
    let og = s.og();
    let a = permanent(&og, "A", 0, &unit(&[(0, 1.0)]));
    add_hub(&og, "hub-x", "ハブ X");
    og.load("main", &[json!({"edge":"Includes","from":"hub-x","to":a})]).unwrap();
    let embedder = FixedEmbedder(HashMap::from([("ハブ X".to_string(), unit(&[(0, 1.0)]))]));

    assert!(!propose_for(&og, &embedder, &cfg(5, 0.5), &a).unwrap().hub);
}

#[test]
fn missing_embedding_is_transient() {
    let s = TestServer::start();
    let og = s.og();
    let f = write_note(&og, &NewNote { body: "元".into(), source: None, locator: None, created: at(9, 0) }).unwrap();
    let slug = promote(&og, &[(NoteKind::Fleeting, f.slug)], "題", "本文", at(11, 0)).unwrap();

    let e = propose_for(&og, &no_embedder(), &cfg(5, 0.0), &slug).unwrap_err();
    assert!(matches!(e, Error::Transient(_)), "{e:?}");
    assert_eq!(e.to_string(), "埋め込みがまだありません");
}
