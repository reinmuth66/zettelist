//! ノートの種類と、書き込み前のノートの形。

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum NoteKind {
    Fleeting,
    Literature,
    Permanent,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NewNote {
    pub body: String,
    pub source: Option<String>,
    pub locator: Option<String>,
    pub created: chrono::DateTime<chrono::Local>,
}

use crate::embed::similarity;
use crate::error::Error;
use crate::omnigraph::Omnigraph;
use crate::paths::lock_file;
use crate::source_ref::SourceRef;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde_json::{json, Map, Value};

const BRANCH: &str = "main";

/// 書き込みに成功したノード。
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    pub slug: String,
    pub kind: NoteKind,
}

/// `created` (ローカル時刻) から作るスラッグ。
pub fn note_slug(t: NaiveDateTime) -> String {
    t.format("%Y%m%d%H%M%S").to_string()
}

fn exists(og: &Omnigraph, node: &str, key: &str, value: &str) -> Result<bool, Error> {
    let q = format!("query q($v: String) {{ match {{ $n: {node} {{ {key}: $v }} }} return {{ $n.{key} as k }} }}");
    Ok(!og.query(BRANCH, &q, json!({ "v": value }))?.is_empty())
}

/// slug の探索の結果。
enum Slot {
    /// 空いている slug
    Free(String),
    /// 同じ `created` と本文のノートがもう書かれている (再実行)
    Done(Written),
}

/// 同じ slug のノートの `created` と本文。なければ `None`。
fn existing(og: &Omnigraph, node: &str, slug: &str) -> Result<Option<(String, String)>, Error> {
    let q = format!(
        "query q($v: String) {{ match {{ $n: {node} {{ slug: $v }} }} return {{ $n.created as created, $n.body as body }} }}"
    );
    let rows = og.query(BRANCH, &q, json!({ "v": slug }))?;
    Ok(rows.first().map(|r| {
        let text = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        (text("created"), text("body"))
    }))
}

/// `2026-11-20T01:00:00.000Z` / `2026-11-20T01:00:00` の両方の形を UTC の日時として読む。
fn parse_created(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S%.f").ok()
}

/// Fleeting と Literature のどちらにもない slug まで、1 秒ずつ進める。
/// 進める途中で、同じ `created` と本文のノートに当たったら、書き込み済みとして返す。
/// サーバーが書いたあとに Transient を返した書き込みを待ち行列から再実行しても、重複しないため。
/// `created` はサーバーが `Z` なしで返し、小数部が 0 なら小数部も省くので、文字列ではなく日時にして比べる。
fn find_slot(og: &Omnigraph, note: &NewNote, created_iso: &str) -> Result<Slot, Error> {
    let want = parse_created(created_iso);
    let mut t = note.created.naive_local();
    loop {
        let slug = note_slug(t);
        let mut taken = false;
        for (node, kind) in [("Fleeting", NoteKind::Fleeting), ("Literature", NoteKind::Literature)] {
            if let Some((created, body)) = existing(og, node, &slug)? {
                let same_time = want.is_some() && parse_created(&created) == want;
                if same_time && body == note.body {
                    return Ok(Slot::Done(Written { slug, kind }));
                }
                taken = true;
            }
        }
        if !taken {
            return Ok(Slot::Free(slug));
        }
        t += Duration::seconds(1);
    }
}

/// 出典の指定から、Cites の宛先の Source の ID と、新しく作る Source の行 (あれば) を決める。
fn resolve_source(og: &Omnigraph, raw: &str) -> Result<(String, Option<Value>), Error> {
    let not_registered = || Error::Invalid(format!("出典 '{raw}' は登録されていません"));
    match SourceRef::parse(raw) {
        SourceRef::Slug(slug) => {
            // slug が "" の仮の Source に当たらないよう、空の指定は登録なしとして扱う
            if slug.is_empty() {
                return Err(not_registered());
            }
            let rows = og.query(
                BRANCH,
                "query q($v: String) { match { $s: Source { slug: $v } } return { $s.id as id } }",
                json!({ "v": slug }),
            )?;
            let id = rows.first().and_then(|r| r.get("id")).and_then(Value::as_str).ok_or_else(not_registered)?;
            Ok((id.to_string(), None))
        }
        SourceRef::Id(id) => {
            if exists(og, "Source", "id", &id)? {
                return Ok((id, None));
            }
            let provisional = json!({"type":"Source","data":{
                "id": id,
                "slug": "",
                "title": raw,
                "kind": "other",
                "input": raw,
                "provenance": "{}",
            }});
            Ok((id, Some(provisional)))
        }
    }
}

/// ノートを 1 件書く。出典なしは Fleeting、出典ありは Literature と Cites。
pub fn write_note(og: &Omnigraph, note: &NewNote) -> Result<Written, Error> {
    if note.body.trim().is_empty() {
        return Err(Error::Invalid("本文が空です".to_string()));
    }
    // 出典の確認を先にして、登録されていない出典なら何も書かない
    let source = match &note.source {
        Some(raw) => Some(resolve_source(og, raw)?),
        None => None,
    };
    let created = note.created.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    // load は merge (同じ slug なら上書き) なので、空き slug の探索から load まで排他にする。
    // 同じ秒に書く別の zl や worker の再実行が、同じ slug を選んで先のノートを消さないため。
    let _lock = lock_file(&og.write_lock())?;
    let slug = match find_slot(og, note, &created)? {
        Slot::Free(slug) => slug,
        Slot::Done(written) => return Ok(written),
    };

    let mut lines = Vec::new();
    let kind = match source {
        None => {
            lines.push(json!({"type":"Fleeting","data":{"slug":slug,"body":note.body,"created":created}}));
            NoteKind::Fleeting
        }
        Some((source_id, provisional)) => {
            // Literature は Cites が必須 (@card(1..1)) なので、同じ load に入れる
            lines.extend(provisional);
            lines.push(json!({"type":"Literature","data":{"slug":slug,"body":note.body,"created":created}}));
            let mut data = serde_json::Map::new();
            if let Some(locator) = &note.locator {
                data.insert("locator".to_string(), json!(locator));
            }
            lines.push(json!({"edge":"Cites","from":slug,"to":source_id,"data":data}));
            NoteKind::Literature
        }
    };
    og.load(BRANCH, &lines)?;
    Ok(Written { slug, kind })
}

impl NoteKind {
    /// スキーマ上のノードの型名。クエリの型名はここからだけ作る。
    fn node_type(self) -> &'static str {
        match self {
            NoteKind::Fleeting => "Fleeting",
            NoteKind::Literature => "Literature",
            NoteKind::Permanent => "Permanent",
        }
    }
}

/// inbox の 1 件。まだ Permanent に昇格していない Fleeting か Literature。
#[derive(Debug, Clone, PartialEq)]
pub struct InboxItem {
    pub kind: NoteKind,
    pub slug: String,
    pub body: String,
    pub created: DateTime<Utc>,
    /// Literature の出典が登録済みなら、その slug (仮の Source は slug が空なので `None`)
    pub source_slug: Option<String>,
    /// Literature の出典の ID
    pub source_id: Option<String>,
    /// Literature の引用箇所
    pub locator: Option<String>,
}

impl InboxItem {
    /// 作成から 30 日を超えていれば true (ちょうど 30 日は false)。
    pub fn is_old(&self, now: DateTime<Utc>) -> bool {
        now - self.created > Duration::days(30)
    }
}

fn text(r: &Map<String, Value>, key: &str) -> Option<String> {
    r.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from)
}

fn item_of(kind: NoteKind, r: &Map<String, Value>) -> Result<InboxItem, Error> {
    let created = text(r, "created")
        .and_then(|s| parse_created(&s))
        .ok_or_else(|| Error::Permanent(format!("created を読めません: {r:?}")))?
        .and_utc();
    let slug = text(r, "slug").ok_or_else(|| Error::Permanent(format!("slug がありません: {r:?}")))?;
    Ok(InboxItem {
        kind,
        slug,
        body: r.get("body").and_then(Value::as_str).unwrap_or_default().to_string(),
        created,
        source_slug: text(r, "source_slug"),
        source_id: text(r, "source_id"),
        locator: text(r, "locator"),
    })
}

/// 昇格していない (DerivedFrom / DerivedFromLit のない) Fleeting と Literature を、古い順に返す。
pub fn inbox(og: &Omnigraph) -> Result<Vec<InboxItem>, Error> {
    let fleeting = og.query(
        BRANCH,
        "query q() { match { $f: Fleeting not { $p: Permanent $p derivedFrom $f } } \
         return { $f.slug as slug, $f.body as body, $f.created as created } }",
        json!({}),
    )?;
    let literature = og.query(
        BRANCH,
        "query q() { match { $l: Literature $l $c:cites $s not { $p: Permanent $p derivedFromLit $l } } \
         return { $l.slug as slug, $l.body as body, $l.created as created, \
         $s.id as source_id, $s.slug as source_slug, $c.locator as locator } }",
        json!({}),
    )?;
    let mut items = Vec::new();
    for r in &fleeting {
        items.push(item_of(NoteKind::Fleeting, r)?);
    }
    for r in &literature {
        items.push(item_of(NoteKind::Literature, r)?);
    }
    items.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.slug.cmp(&b.slug)));
    Ok(items)
}

/// 元のノートから Permanent を 1 件作り、DerivedFrom / DerivedFromLit を同じ load で張る。
/// Permanent の slug を返す。
///
/// 呼ぶ側は、成功後に `Job::Promote { slug }` を待ち行列に積み、`worker::spawn_worker` を呼ぶこと
/// (この関数は積みも worker の起動もしない)。
/// slug の探索から load までは `write.lock` を持つ (`write_note` と同じ)。
pub fn promote(
    og: &Omnigraph,
    from: &[(NoteKind, String)],
    title: &str,
    body: &str,
    created: DateTime<chrono::Local>,
) -> Result<String, Error> {
    if title.trim().is_empty() {
        return Err(Error::Invalid("題名が空です".to_string()));
    }
    if body.trim().is_empty() {
        return Err(Error::Invalid("本文が空です".to_string()));
    }
    if from.is_empty() {
        return Err(Error::Invalid("昇格元のノートがありません".to_string()));
    }
    // Permanent から Permanent は DerivedFrom で結べない
    if from.iter().any(|(kind, _)| *kind == NoteKind::Permanent) {
        return Err(Error::Invalid("Permanent は昇格元にできません".to_string()));
    }

    // `write_note` と同じロックを、空き slug の探索から load まで持つ
    let _lock = lock_file(&og.write_lock())?;
    let mut t = created.naive_local();
    let slug = loop {
        let slug = note_slug(t);
        if !exists(og, "Permanent", "slug", &slug)? {
            break slug;
        }
        t += Duration::seconds(1);
    };

    let created = created.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let mut lines = vec![json!({"type":"Permanent","data":{"slug":slug,"title":title,"body":body,"created":created}})];
    for (kind, from_slug) in from {
        let edge = if *kind == NoteKind::Literature { "DerivedFromLit" } else { "DerivedFrom" };
        lines.push(json!({"edge":edge,"from":slug,"to":from_slug}));
    }
    og.load(BRANCH, &lines)?;
    Ok(slug)
}

/// ノートを消す。ノードを消せば、つながるエッジ (Literature の Cites など) も一緒に消える。
/// 途中で失敗しても、同じ指定で再実行できる (存在しないノートの削除は何もしない)。
pub fn delete_notes(og: &Omnigraph, notes: &[(NoteKind, String)]) -> Result<(), Error> {
    for (kind, slug) in notes {
        // delete は insert と同じクエリに混ぜられないので、1 件ずつ別の mutate にする
        let q = format!("query d($s: String) {{ delete {} where slug = $s }}", kind.node_type());
        og.mutate(BRANCH, &q, json!({ "s": slug }))?;
    }
    Ok(())
}

/// 埋め込みを書き込む。ノートがなければ何も書かない。
pub fn set_embedding(og: &Omnigraph, kind: NoteKind, slug: &str, v: &[f32]) -> Result<(), Error> {
    let q = format!(
        "query q($slug: String, $v: Vector(1024)) {{ update {} set {{ embedding: $v }} where slug = $slug }}",
        kind.node_type()
    );
    og.mutate(BRANCH, &q, json!({ "slug": slug, "v": v }))
}

/// 埋め込みの元にする文。Permanent は題名と本文を改行でつなぎ、ほかは本文。ノートがなければ `None`。
pub fn note_text(og: &Omnigraph, kind: NoteKind, slug: &str) -> Result<Option<String>, Error> {
    let q = format!(
        "query q($s: String) {{ match {{ $n: {t} {{ slug: $s }} }} return {{ $n.body as body{title} }} }}",
        t = kind.node_type(),
        title = if kind == NoteKind::Permanent { ", $n.title as title" } else { "" },
    );
    let rows = og.query(BRANCH, &q, json!({ "s": slug }))?;
    Ok(rows.first().map(|r| {
        let body = r.get("body").and_then(Value::as_str).unwrap_or_default();
        match r.get("title").and_then(Value::as_str) {
            Some(title) if kind == NoteKind::Permanent => format!("{title}\n{body}"),
            _ => body.to_string(),
        }
    }))
}

/// ノートの埋め込み。ノートがない、または埋め込みがまだなら `None`。
pub fn get_embedding(og: &Omnigraph, kind: NoteKind, slug: &str) -> Result<Option<Vec<f32>>, Error> {
    let q = format!(
        "query q($s: String) {{ match {{ $n: {} {{ slug: $s }} }} return {{ $n.embedding as embedding }} }}",
        kind.node_type()
    );
    let rows = og.query(BRANCH, &q, json!({ "s": slug }))?;
    let Some(value) = rows.first().and_then(|r| r.get("embedding")) else {
        return Ok(None);
    };
    parse_vector(value).map(Some)
}

fn parse_vector(value: &Value) -> Result<Vec<f32>, Error> {
    let bad = || Error::Permanent(format!("埋め込みを読めません: {value}"));
    value
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|x| x.as_f64().map(|f| f as f32).ok_or_else(bad))
        .collect()
}

/// 近いノート 1 件。
#[derive(Debug, Clone, PartialEq)]
pub struct Near {
    pub kind: NoteKind,
    pub slug: String,
    /// Permanent だけが持つ
    pub title: Option<String>,
    pub body: String,
    pub similarity: f32,
}

/// 種類ごとに `nearest` で引き、近い順に並べ直して `limit` 件を返す。
/// 類似度が `min_similarity` に満たないものと、埋め込みのないノートは出ない。
pub fn nearest(
    og: &Omnigraph,
    branch: &str,
    kinds: &[NoteKind],
    v: &[f32],
    limit: usize,
    min_similarity: f32,
) -> Result<Vec<Near>, Error> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for &kind in kinds {
        let title =if kind == NoteKind::Permanent { "$n.title as title, " } else { "" };
        // nearest は末尾の limit が要る。種類ごとに limit 件まで引けば、合わせた上位 limit 件を必ず含む。
        let q = format!(
            "query q($v: Vector(1024)) {{ match {{ $n: {t} }} \
             return {{ $n.slug as slug, {title}$n.body as body, nearest($n.embedding, $v) as distance }} \
             order {{ nearest($n.embedding, $v) }} limit {limit} }}",
            t = kind.node_type(),
        );
        for r in og.query(branch, &q, json!({ "v": v }))? {
            let distance = r
                .get("distance")
                .and_then(Value::as_f64)
                .ok_or_else(|| Error::Permanent(format!("distance を読めません: {r:?}")))?;
            let slug = text(&r, "slug").ok_or_else(|| Error::Permanent(format!("slug がありません: {r:?}")))?;
            found.push(Near {
                kind,
                slug,
                title: text(&r, "title"),
                body: r.get("body").and_then(Value::as_str).unwrap_or_default().to_string(),
                similarity: similarity(distance as f32),
            });
        }
    }
    found.retain(|n| n.similarity >= min_similarity);
    found.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
    found.truncate(limit);
    Ok(found)
}

/// `show_note` が返す、種類を問わないノート 1 件。
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub kind: NoteKind,
    pub slug: String,
    /// Permanent だけが持つ
    pub title: Option<String>,
    pub body: String,
    pub created: DateTime<Utc>,
}

/// slug のノートを、Fleeting、Literature、Permanent の順に探して返す。
/// どれにもなければ `Error::Invalid`。
pub fn show_note(og: &Omnigraph, slug: &str) -> Result<Note, Error> {
    for kind in [NoteKind::Fleeting, NoteKind::Literature, NoteKind::Permanent] {
        let title = if kind == NoteKind::Permanent { ", $n.title as title" } else { "" };
        let q = format!(
            "query q($s: String) {{ match {{ $n: {t} {{ slug: $s }} }} \
             return {{ $n.slug as slug, $n.body as body, $n.created as created{title} }} }}",
            t = kind.node_type(),
        );
        let rows = og.query(BRANCH, &q, json!({ "s": slug }))?;
        let Some(r) = rows.first() else { continue };
        let created = text(r, "created")
            .and_then(|s| parse_created(&s))
            .ok_or_else(|| Error::Permanent(format!("created を読めません: {r:?}")))?
            .and_utc();
        return Ok(Note {
            kind,
            slug: slug.to_string(),
            title: text(r, "title"),
            body: r.get("body").and_then(Value::as_str).unwrap_or_default().to_string(),
            created,
        });
    }
    Err(Error::Invalid(format!("ノート '{slug}' はありません")))
}
