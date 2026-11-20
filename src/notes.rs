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

use crate::error::Error;
use crate::omnigraph::Omnigraph;
use crate::source_ref::SourceRef;
use chrono::{Duration, NaiveDateTime};
use serde_json::{json, Value};

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

/// Fleeting と Literature のどちらにもない slug まで、1 秒ずつ進める。
fn free_slug(og: &Omnigraph, created: NaiveDateTime) -> Result<String, Error> {
    let mut t = created;
    loop {
        let slug = note_slug(t);
        if !exists(og, "Fleeting", "slug", &slug)? && !exists(og, "Literature", "slug", &slug)? {
            return Ok(slug);
        }
        t += Duration::seconds(1);
    }
}

/// 出典の指定から、Cites の宛先の Source の ID と、新しく作る Source の行 (あれば) を決める。
fn resolve_source(og: &Omnigraph, raw: &str) -> Result<(String, Option<Value>), Error> {
    let not_registered = || Error::Permanent(format!("出典 '{raw}' は登録されていません"));
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
        return Err(Error::Permanent("本文が空です".to_string()));
    }
    // 出典の確認を先にして、登録されていない出典なら何も書かない
    let source = match &note.source {
        Some(raw) => Some(resolve_source(og, raw)?),
        None => None,
    };
    let slug = free_slug(og, note.created.naive_local())?;
    let created = note.created.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

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
