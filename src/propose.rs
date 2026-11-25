//! review ブランチへの、リンクと Hub の提案。main には書かない。

use crate::config::LinkConfig;
use crate::embed::Embedder;
use crate::error::Error;
use crate::notes::{get_embedding, nearest, NoteKind};
use crate::omnigraph::Omnigraph;
use serde_json::{json, Value};
use std::collections::HashSet;

pub(crate) const MAIN: &str = "main";
pub(crate) const REVIEW: &str = "review";

/// 1 回の提案の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proposed {
    /// review に書いた Links の本数
    pub links: usize,
    /// review に Includes を書いたか
    pub hub: bool,
}

/// review ブランチがなければ main から作り、あれば main を取り込んで、
/// 後から main に足された Permanent を review からも見えるようにする。
///
/// main から消えた Permanent や Hub (Studio などで消したもの) を端に持つ提案は、
/// 取り込みが `orphan_edge` で衝突するので、先に review から消す。
pub fn ensure_review(og: &Omnigraph) -> Result<(), Error> {
    if og.branches()?.iter().any(|b| b == REVIEW) {
        drop_orphans(og)?;
        og.statement(&format!("branch merge {MAIN} into {REVIEW}"))
    } else {
        og.statement(&format!("branch create {REVIEW} from {MAIN}"))
    }
}

/// review にあって main にない Permanent と Hub に触れる Links と Includes を、review から消す。
fn drop_orphans(og: &Omnigraph) -> Result<(), Error> {
    let slugs = |branch: &str, node: &str| -> Result<HashSet<String>, Error> {
        let q = format!("query q() {{ match {{ $n: {node} }} return {{ $n.slug as slug }} }}");
        Ok(og
            .query(branch, &q, json!({}))?
            .iter()
            .filter_map(|r| r.get("slug").and_then(Value::as_str).map(String::from))
            .collect())
    };
    let gone = |node: &str| -> Result<Vec<String>, Error> {
        let main = slugs(MAIN, node)?;
        Ok(slugs(REVIEW, node)?.into_iter().filter(|s| !main.contains(s)).collect())
    };
    let edges: &[(&str, &[&str])] = &[("Permanent", &["Links", "Includes"]), ("Hub", &["Includes"])];
    for (node, kinds) in edges {
        for slug in gone(node)? {
            for kind in *kinds {
                for end in ["@src", "@dst"] {
                    // Includes の @src は Hub、@dst は Permanent
                    if *kind == "Includes" && ((*node == "Hub") != (end == "@src")) {
                        continue;
                    }
                    let q = format!("query d($s: String) {{ delete {kind} where {end} = $s }}");
                    og.mutate(REVIEW, &q, json!({"s": slug}))?;
                }
            }
        }
    }
    Ok(())
}

/// main と review にすでにある Links の、`slug` につながる相手 (どちら向きでも)。
fn linked_with(og: &Omnigraph, slug: &str) -> Result<HashSet<String>, Error> {
    let mut found = HashSet::new();
    for branch in [MAIN, REVIEW] {
        let rows = og.query(
            branch,
            "query q() { match { $a: Permanent $a links $b } return { $a.slug as src, $b.slug as dst } }",
            json!({}),
        )?;
        for r in rows {
            let get = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let (src, dst) = (get("src"), get("dst"));
            if src == slug {
                found.insert(dst);
            } else if dst == slug {
                found.insert(src);
            }
        }
    }
    Ok(found)
}

/// main と review で `slug` を含んでいる Hub の slug。
fn including_hubs(og: &Omnigraph, slug: &str) -> Result<HashSet<String>, Error> {
    let mut found = HashSet::new();
    for branch in [MAIN, REVIEW] {
        let rows = og.query(
            branch,
            "query q($s: String) { match { $p: Permanent { slug: $s } $h: Hub $h includes $p } return { $h.slug as hub } }",
            json!({ "s": slug }),
        )?;
        found.extend(rows.iter().filter_map(|r| r.get("hub").and_then(Value::as_str).map(String::from)));
    }
    Ok(found)
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denom = norm(a) * norm(b);
    if denom == 0.0 { 0.0 } else { dot / denom }
}

/// Permanent `slug` に近い Permanent へのリンクと、いちばん近い Hub への Includes を review に書く。
/// 同じ組をもう一度書くと理由を上書きしてしまうので、すでにある組は書かない。
pub fn propose_for(og: &Omnigraph, embedder: &dyn Embedder, cfg: &LinkConfig, slug: &str) -> Result<Proposed, Error> {
    let v = get_embedding(og, NoteKind::Permanent, slug)?
        .ok_or_else(|| Error::Transient("埋め込みがまだありません".to_string()))?;
    ensure_review(og)?;

    // 既存のリンク先を除いても candidates 件残るよう、その分多めに引く (自分自身の 1 件も含む)
    let linked = linked_with(og, slug)?;
    let limit = cfg.candidates + 1 + linked.len();
    let near = nearest(og, MAIN, &[NoteKind::Permanent], &v, limit, cfg.min_similarity)?;
    let mut lines = Vec::new();
    for n in near.iter().filter(|n| n.slug != slug && !linked.contains(&n.slug)).take(cfg.candidates) {
        lines.push(json!({"edge":"Links","from":slug,"to":n.slug,"data":{"similarity":n.similarity}}));
    }
    let links = lines.len();
    og.load(REVIEW, &lines)?;

    let hub = propose_hub(og, embedder, cfg, slug, &v)?;
    Ok(Proposed { links, hub })
}

/// `v` は Permanent `slug` の埋め込み。
fn propose_hub(og: &Omnigraph, embedder: &dyn Embedder, cfg: &LinkConfig, slug: &str, v: &[f32]) -> Result<bool, Error> {
    let hubs = og.query(MAIN, "query q() { match { $h: Hub } return { $h.slug as slug, $h.title as title } }", json!({}))?;
    if hubs.is_empty() {
        return Ok(false);
    }
    // 題名の埋め込みは保存していないので、その場で作って比べる
    let mut best: Option<(String, f32)> = None;
    for h in &hubs {
        let text = |k: &str| h.get(k).and_then(Value::as_str).unwrap_or_default();
        let score = cosine(v, &embedder.embed(text("title"))?);
        if best.as_ref().is_none_or(|(_, s)| score > *s) {
            best = Some((text("slug").to_string(), score));
        }
    }
    let Some((hub, score)) = best else { return Ok(false) };
    if score < cfg.min_similarity || including_hubs(og, slug)?.contains(&hub) {
        return Ok(false);
    }
    og.load(REVIEW, &[json!({"edge":"Includes","from":hub,"to":slug})])?;
    Ok(true)
}
