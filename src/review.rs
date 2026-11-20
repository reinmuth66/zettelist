//! review ブランチの提案を一覧し、利用者の判断を main に反映する。
//!
//! `apply` は途中で落ちても、もう一度呼べば続きから終わる。
//! 未判断の提案は review から消す前に `review.json` の `reinsert` に書いておき、
//! merge のあとで review に書き戻す。

use crate::error::Error;
use crate::omnigraph::Omnigraph;
use crate::propose::{ensure_review, MAIN, REVIEW};
use crate::queue::{Job, Queue};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Proposal {
    Link { from: String, to: String, similarity: f32 },
    Hub { hub: String, note: String },
}

impl Proposal {
    /// 判断を結びつける鍵。`link:<from>:<to>` / `hub:<hub>:<note>`。
    pub fn key(&self) -> String {
        match self {
            Proposal::Link { from, to, .. } => format!("link:{from}:{to}"),
            Proposal::Hub { hub, note } => format!("hub:{hub}:{note}"),
        }
    }

    /// review に書く JSONL の行
    fn line(&self) -> Value {
        match self {
            Proposal::Link { from, to, similarity } => {
                json!({"edge":"Links","from":from,"to":to,"data":{"similarity":similarity}})
            }
            Proposal::Hub { hub, note } => json!({"edge":"Includes","from":hub,"to":note}),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Decision {
    Adopt { reason: Option<String> },
    Reject,
}

/// 利用者の判断と、書き戻し待ちの提案。`review.json` に置く。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ReviewState {
    #[serde(default)]
    pub decisions: BTreeMap<String, Decision>,
    #[serde(default)]
    pub reinsert: Vec<Proposal>,
}

fn io_err(what: &str, path: &Path, e: std::io::Error) -> Error {
    Error::Permanent(format!("{what}に失敗しました ({}): {e}", path.display()))
}

impl ReviewState {
    /// ファイルがなければ空。
    pub fn load(path: &Path) -> Result<ReviewState, Error> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::Permanent(format!("{} を読めません: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ReviewState::default()),
            Err(e) => Err(io_err("読み込み", path, e)),
        }
    }

    /// 一時ファイルに書いてから rename する。途中で落ちても壊れたファイルを残さない。
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let dir = match path.parent() {
            Some(d) if !d.as_os_str().is_empty() => d,
            _ => Path::new("."),
        };
        std::fs::create_dir_all(dir).map_err(|e| io_err("ディレクトリの作成", dir, e))?;
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| Error::Permanent(format!("レビューの状態を JSON にできません: {e}")))?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| io_err("一時ファイルの作成", dir, e))?;
        tmp.write_all(&json).map_err(|e| io_err("一時ファイルへの書き込み", tmp.path(), e))?;
        tmp.as_file().sync_all().map_err(|e| io_err("一時ファイルの同期", tmp.path(), e))?;
        tmp.persist(path).map_err(|e| io_err("rename", path, e.error))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub links: usize,
    pub hubs: usize,
}

impl Summary {
    /// バックアップのコミットメッセージ。0 件の種類は書かない。
    pub fn message(&self) -> String {
        let mut parts = Vec::new();
        if self.links > 0 {
            parts.push(format!("links {}", self.links));
        }
        if self.hubs > 0 {
            parts.push(format!("hub {}", self.hubs));
        }
        if parts.is_empty() { "merge".to_string() } else { format!("merge: {}", parts.join(", ")) }
    }
}

fn text(r: &serde_json::Map<String, Value>, k: &str) -> String {
    r.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// ブランチの Links を (from, to, similarity) で返す。
fn links_on(og: &Omnigraph, branch: &str) -> Result<Vec<(String, String, f32)>, Error> {
    let rows = og.query(
        branch,
        "query q() { match { $a: Permanent $a $l:links $b } return { $a.slug as src, $b.slug as dst, $l.similarity as sim } }",
        json!({}),
    )?;
    Ok(rows
        .iter()
        .map(|r| (text(r, "src"), text(r, "dst"), r.get("sim").and_then(Value::as_f64).unwrap_or(0.0) as f32))
        .collect())
}

/// ブランチの Includes を (hub, note) で返す。
fn includes_on(og: &Omnigraph, branch: &str) -> Result<Vec<(String, String)>, Error> {
    let rows = og.query(
        branch,
        "query q() { match { $h: Hub $h includes $p } return { $h.slug as hub, $p.slug as perm } }",
        json!({}),
    )?;
    Ok(rows.iter().map(|r| (text(r, "hub"), text(r, "perm"))).collect())
}

/// review にあって main にない Links と Includes。Links を先に、近い順。
pub fn list_proposals(og: &Omnigraph) -> Result<Vec<Proposal>, Error> {
    let in_main: HashSet<(String, String)> = links_on(og, MAIN)?.into_iter().map(|(a, b, _)| (a, b)).collect();
    let mut proposals: Vec<Proposal> = links_on(og, REVIEW)?
        .into_iter()
        .filter(|(a, b, _)| !in_main.contains(&(a.clone(), b.clone())))
        .map(|(from, to, similarity)| Proposal::Link { from, to, similarity })
        .collect();
    let sim = |p: &Proposal| match p {
        Proposal::Link { similarity, .. } => *similarity,
        Proposal::Hub { .. } => 0.0,
    };
    // 類似度が同じなら鍵の順にして、並びを毎回同じにする
    proposals.sort_by(|x, y| sim(y).total_cmp(&sim(x)).then_with(|| x.key().cmp(&y.key())));

    let hubs_in_main: HashSet<(String, String)> = includes_on(og, MAIN)?.into_iter().collect();
    let mut hubs: Vec<(String, String)> =
        includes_on(og, REVIEW)?.into_iter().filter(|p| !hubs_in_main.contains(p)).collect();
    hubs.sort();
    proposals.extend(hubs.into_iter().map(|(hub, note)| Proposal::Hub { hub, note }));
    Ok(proposals)
}

/// review から提案の 1 本を消す。なければ何も起きない。
fn delete_from_review(og: &Omnigraph, p: &Proposal) -> Result<(), Error> {
    let (query, a, b) = match p {
        Proposal::Link { from, to, .. } => {
            ("query d($a: String, $b: String) { delete Links where @src = $a and @dst = $b }", from, to)
        }
        Proposal::Hub { hub, note } => {
            ("query d($a: String, $b: String) { delete Includes where @src = $a and @dst = $b }", hub, note)
        }
    };
    og.mutate(REVIEW, query, json!({"a": a, "b": b}))
}

/// 判断を main に反映する。何度呼んでも、途中から続きを終わらせる。
pub fn apply(og: &Omnigraph, state_path: &Path, queue: &Queue) -> Result<Summary, Error> {
    // 1. review を main に追いつかせる
    ensure_review(og)?;

    // 2. 判断のない提案を、消す前に reinsert へ書く。前回の reinsert は鍵ごとに和を取って失わない
    let mut state = ReviewState::load(state_path)?;
    let proposals = list_proposals(og)?;
    for p in &proposals {
        let key = p.key();
        if !state.decisions.contains_key(&key) && !state.reinsert.iter().any(|r| r.key() == key) {
            state.reinsert.push(p.clone());
        }
    }
    state.save(state_path)?;

    let by_key: HashMap<String, &Proposal> = proposals.iter().map(|p| (p.key(), p)).collect();

    // 3. 却下したものと reinsert のものを review から消す
    for (key, d) in &state.decisions {
        if let (Decision::Reject, Some(p)) = (d, by_key.get(key)) {
            delete_from_review(og, p)?;
        }
    }
    for p in &state.reinsert {
        delete_from_review(og, p)?;
    }

    // 4. 理由つきで採用したリンクを、理由を付けて書き直す (`@key` なので 1 本のまま)。
    //    すでに merge 済みで提案に残っていないものは、main に理由まで入っているので書かない
    let mut with_reason = Vec::new();
    for (key, d) in &state.decisions {
        if let (Decision::Adopt { reason: Some(reason) }, Some(Proposal::Link { from, to, similarity })) =
            (d, by_key.get(key).copied())
        {
            if !reason.is_empty() {
                with_reason.push(json!({"edge":"Links","from":from,"to":to,"data":{"reason":reason,"similarity":similarity}}));
            }
        }
    }
    og.load(REVIEW, &with_reason)?;

    // 5. main へ merge する
    og.statement(&format!("branch merge {REVIEW} into {MAIN}"))?;

    // 6. 未判断のものを review に書き戻す。判断は片付ける
    ensure_review(og)?;
    let lines: Vec<Value> = state.reinsert.iter().map(Proposal::line).collect();
    og.load(REVIEW, &lines)?;
    let adopted = |prefix: &str| {
        state.decisions.iter().filter(|(k, d)| k.starts_with(prefix) && matches!(d, Decision::Adopt { .. })).count()
    };
    let summary = Summary { links: adopted("link:"), hubs: adopted("hub:") };
    state.decisions.clear();
    state.reinsert.clear();
    state.save(state_path)?;

    // 7. バックアップを積む
    queue.push(Job::Backup { message: summary.message() })?;
    Ok(summary)
}
