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
use std::fs::File;
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

    /// review に書く JSONL の行。`reason` は Links だけに付く。
    fn line(&self, reason: Option<&str>) -> Value {
        match self {
            Proposal::Link { from, to, similarity } => {
                let mut data = json!({"similarity":similarity});
                if let Some(r) = reason {
                    data["reason"] = json!(r);
                }
                json!({"edge":"Links","from":from,"to":to,"data":data})
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
/// review ブランチがまだなければ、提案はない (ブランチは作らない)。
pub fn list_proposals(og: &Omnigraph) -> Result<Vec<Proposal>, Error> {
    if !og.branches()?.iter().any(|b| b == REVIEW) {
        return Ok(Vec::new());
    }
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

/// `review.lock` を取る (ほかが持っていれば待つ)。ロックはファイルを閉じると外れる。
/// `apply` と、worker が review に提案を書く間 (`propose_for`) が同じロックを使い、
/// 提案が apply の途中で review に入って未判断のまま merge されるのを防ぐ。
pub fn lock_review(path: &Path) -> Result<File, Error> {
    crate::paths::lock_file(path)
}

/// main の Permanent と Hub の slug。
fn main_nodes(og: &Omnigraph) -> Result<(HashSet<String>, HashSet<String>), Error> {
    let slugs = |node: &str| -> Result<HashSet<String>, Error> {
        let q = format!("query q() {{ match {{ $n: {node} }} return {{ $n.slug as slug }} }}");
        Ok(og.query(MAIN, &q, json!({}))?.iter().map(|r| text(r, "slug")).collect())
    };
    Ok((slugs("Permanent")?, slugs("Hub")?))
}

/// 提案の両端 (Permanent と Hub) が main にあれば true。
/// ないものを review に書くと、load が毎回 400 になって apply が終わらない。
fn endpoints_exist(p: &Proposal, permanents: &HashSet<String>, hubs: &HashSet<String>) -> bool {
    match p {
        Proposal::Link { from, to, .. } => permanents.contains(from) && permanents.contains(to),
        Proposal::Hub { hub, note } => hubs.contains(hub) && permanents.contains(note),
    }
}

/// main にある Links と Includes の鍵
fn main_keys(og: &Omnigraph) -> Result<HashSet<String>, Error> {
    let mut keys: HashSet<String> =
        links_on(og, MAIN)?.into_iter().map(|(from, to, similarity)| Proposal::Link { from, to, similarity }.key()).collect();
    keys.extend(includes_on(og, MAIN)?.into_iter().map(|(hub, note)| Proposal::Hub { hub, note }.key()));
    Ok(keys)
}

/// 判断を main に反映する。何度呼んでも、途中から続きを終わらせる。
///
/// 呼ぶ側の注意:
/// - `state_path` には必ず `Paths::review_json()` を渡すこと。ロックは `state_path` の隣の `review.lock`
///   (`Paths::review_lock()` と同じ場所) を取るので、別の場所を渡すと worker と排他にならない。
/// - 最後に `Job::Backup` を待ち行列に積むが、worker は起動しない。
///   呼んだあとで `worker::spawn_worker` を呼ばないと、バックアップは次に worker が動くまで実行されない。
pub fn apply(og: &Omnigraph, state_path: &Path, queue: &Queue) -> Result<Summary, Error> {
    // worker が review に提案を書く間は待つ。apply の間は worker も書けない
    let _lock = lock_review(&state_path.with_file_name("review.lock"))?;

    // 1. review を main に追いつかせる
    ensure_review(og)?;

    // 2. 判断のない提案を、消す前に reinsert へ書く。前回の reinsert は鍵ごとに和を取って失わない。
    //    前回の途中で落ちたあとに判断が付いたものは、reinsert の古い「未判断」に負けない:
    //    却下は reinsert から外し、採用は reinsert に残して (手順 3 と 6 では判断を優先し)、
    //    review から消えていれば手順 4 で書き戻す
    let mut state = ReviewState::load(state_path)?;
    let proposals = list_proposals(og)?;
    // 手順 3 は review にしか触れないので、手順 4 でもこのまま使える
    let in_main = main_keys(og)?;
    {
        let ReviewState { decisions, reinsert } = &mut state;
        // 端の Permanent や Hub が main から消えた提案は、書き戻せないので判断ごと捨てる
        // (残すと review への load が毎回失敗して、apply が終わらない)
        let (permanents, hubs) = main_nodes(og)?;
        let dead: Vec<String> =
            reinsert.iter().filter(|p| !endpoints_exist(p, &permanents, &hubs)).map(Proposal::key).collect();
        reinsert.retain(|p| !dead.contains(&p.key()));
        for key in &dead {
            decisions.remove(key);
        }
        // review から提案ごと消えた (ensure_review が片付けた) ものの判断も捨てる。
        // 判断は提案にしか付かないので、review にも reinsert にも main にもない鍵は、消えた提案のもの
        decisions.retain(|key, _| {
            in_main.contains(key) || proposals.iter().chain(reinsert.iter()).any(|p| p.key() == *key)
        });
        reinsert.retain(|p| !matches!(decisions.get(&p.key()), Some(Decision::Reject)));
        for p in &proposals {
            let key = p.key();
            if !decisions.contains_key(&key) && !reinsert.iter().any(|r| r.key() == key) {
                reinsert.push(p.clone());
            }
        }
    }
    state.save(state_path)?;

    let by_key: HashMap<String, &Proposal> = proposals.iter().map(|p| (p.key(), p)).collect();
    let undecided: Vec<&Proposal> = state.reinsert.iter().filter(|p| !state.decisions.contains_key(&p.key())).collect();

    // 3. 却下したものと、判断のない reinsert のものを review から消す
    for (key, d) in &state.decisions {
        if let (Decision::Reject, Some(p)) = (d, by_key.get(key)) {
            delete_from_review(og, p)?;
        }
    }
    for p in &undecided {
        delete_from_review(og, p)?;
    }

    // 4. 採用したもののうち、理由つきのリンクは理由を付けて書き直す (`@key` なので 1 本のまま)。
    //    前回の手順 3 で review から消えたまま採用されたもの (reinsert にだけ残る) は、元の類似度で書き戻す。
    //    すでに merge 済み (main にある) ものは、理由まで main に入っているので書かない
    let mut writes = Vec::new();
    for (key, d) in &state.decisions {
        let Decision::Adopt { reason } = d else { continue };
        let reason = reason.as_deref().filter(|r| !r.is_empty());
        if let Some(p) = by_key.get(key) {
            if reason.is_some() {
                writes.push(p.line(reason));
            }
        } else if !in_main.contains(key) {
            if let Some(p) = state.reinsert.iter().find(|p| p.key() == *key) {
                writes.push(p.line(reason));
            }
        }
    }
    og.load(REVIEW, &writes)?;

    // 5. main へ merge する
    og.statement(&format!("branch merge {REVIEW} into {MAIN}"))?;

    // 6. 未判断のものを review に書き戻す。判断は片付ける
    ensure_review(og)?;
    let lines: Vec<Value> = undecided.iter().map(|p| p.line(None)).collect();
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

/// 提案 1 本への判断を `review.json` に書く。`None` は判断を外す。
///
/// `review.lock` を自分で取るので、呼ぶ側は持ったまま呼ばないこと。
/// `key` が `list_proposals` になければ、何も書かずに `Error::Invalid`。
pub fn decide(
    og: &Omnigraph,
    state_path: &Path,
    lock_path: &Path,
    key: &str,
    decision: Option<Decision>,
) -> Result<(), Error> {
    let _lock = lock_review(lock_path)?;
    if !list_proposals(og)?.iter().any(|p| p.key() == key) {
        return Err(Error::Invalid(format!("提案 '{key}' はありません")));
    }
    let mut state = ReviewState::load(state_path)?;
    match decision {
        Some(d) => {
            state.decisions.insert(key.to_string(), d);
        }
        None => {
            state.decisions.remove(key);
        }
    }
    state.save(state_path)
}

/// main の Hub の概要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubSummary {
    pub slug: String,
    pub title: String,
}

/// main の Hub を題名の順に返す。
pub fn hubs(og: &Omnigraph) -> Result<Vec<HubSummary>, Error> {
    let rows = og.query(
        MAIN,
        "query q() { match { $h: Hub } return { $h.slug as slug, $h.title as title } }",
        json!({}),
    )?;
    let mut list: Vec<HubSummary> = rows
        .iter()
        .map(|r| HubSummary { slug: text(r, "slug"), title: text(r, "title") })
        .collect();
    list.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.slug.cmp(&b.slug)));
    Ok(list)
}

/// `rehub` の結果。`unrehub` で元に戻すために使う。
#[derive(Debug, Clone, PartialEq)]
pub struct Rehubbed {
    pub old_key: String,
    pub new_key: String,
    pub hub: String,
    pub note: String,
    pub created: bool,
    pub previous: Option<Decision>,
    pub new_previous: Option<Decision>,
}

/// Hub の提案 `key` を別の Hub `hub` へ選び直す。
///
/// `review.lock` を自分で取るので、呼ぶ側は持ったまま呼ばないこと。
pub fn rehub(og: &Omnigraph, state_path: &Path, lock_path: &Path, key: &str, hub: &str) -> Result<Rehubbed, Error> {
    let _lock = lock_review(lock_path)?;
    let proposals = list_proposals(og)?;
    let Some((old_hub, note)) = proposals.iter().find_map(|p| match p {
        Proposal::Hub { hub: h, note: n } if p.key() == key => Some((h.clone(), n.clone())),
        _ => None,
    }) else {
        return Err(Error::Invalid(format!("Hub の提案 '{key}' はありません")));
    };
    let (_, hubs_in_main) = main_nodes(og)?;
    if !hubs_in_main.contains(hub) {
        return Err(Error::Invalid(format!("Hub '{hub}' はありません")));
    }
    if hub == old_hub {
        return Err(Error::Invalid("元と同じ Hub です".to_string()));
    }
    if includes_on(og, MAIN)?.iter().any(|(h, n)| h == hub && n == &note) {
        return Err(Error::Invalid("すでにその Hub に入っています".to_string()));
    }

    let mut state = ReviewState::load(state_path)?;
    let new_proposal = Proposal::Hub { hub: hub.to_string(), note: note.clone() };
    let new_key = new_proposal.key();
    let created = !proposals.iter().any(|p| p.key() == new_key);
    if created {
        ensure_review(og)?;
        og.load(REVIEW, &[new_proposal.line(None)])?;
    }
    let previous = state.decisions.get(key).cloned();
    let new_previous = state.decisions.get(&new_key).cloned();
    state.decisions.insert(key.to_string(), Decision::Reject);
    state.decisions.insert(new_key.clone(), Decision::Adopt { reason: None });
    state.save(state_path)?;
    Ok(Rehubbed {
        old_key: key.to_string(),
        new_key,
        hub: hub.to_string(),
        note,
        created,
        previous,
        new_previous,
    })
}

/// `rehub` を取り消して、元の状態に戻す。
///
/// `review.lock` を自分で取るので、呼ぶ側は持ったまま呼ばないこと。
pub fn unrehub(og: &Omnigraph, state_path: &Path, lock_path: &Path, r: &Rehubbed) -> Result<(), Error> {
    let _lock = lock_review(lock_path)?;
    if r.created {
        delete_from_review(
            og,
            &Proposal::Hub {
                hub: r.hub.clone(),
                note: r.note.clone(),
            },
        )?;
    }
    let mut state = ReviewState::load(state_path)?;
    match &r.new_previous {
        Some(d) => {
            state.decisions.insert(r.new_key.clone(), d.clone());
        }
        None => {
            state.decisions.remove(&r.new_key);
        }
    }
    match &r.previous {
        Some(d) => {
            state.decisions.insert(r.old_key.clone(), d.clone());
        }
        None => {
            state.decisions.remove(&r.old_key);
        }
    }
    state.save(state_path)
}
