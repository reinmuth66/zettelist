//! CLI と TUI が共有する操作。標準出力にも標準エラーにも書かず、結果と警告を値で返す。

use crate::error::Error;
use crate::notes::{get_embedding, inbox, nearest, show_note, write_note, Near, NewNote, NoteKind};
use crate::omnigraph::Omnigraph;
use crate::queue::Job;
use crate::review::{apply, Summary};
use crate::worker::spawn_worker;
use crate::Ctx;
use chrono::Local;
use serde_json::json;
use std::collections::HashSet;

/// 操作の結果と、本体のあとに起きた後処理の警告。
#[derive(Debug)]
pub struct Done<T> {
    pub value: T,
    pub queue_warning: Option<String>,
    pub worker_warning: Option<String>,
}

/// 書き込みの結果。
#[derive(Debug)]
pub enum WriteOutcome {
    Saved(String),
    Queued(Error),
}

/// 似ているノート。
#[derive(Debug)]
pub struct Similar {
    pub inbox: Vec<Near>,
    pub permanent: Vec<Near>,
}

/// worker の起動に失敗しても、保存は済んでいるので警告だけにする
pub fn start_worker(ctx: &Ctx) -> Option<String> {
    spawn_worker(&ctx.paths).err().map(|e| format!("worker を起動できません: {e}"))
}

/// メモを書き込む。書けなければ待ち行列に保存する。
pub fn write(ctx: &Ctx, note: NewNote) -> Result<Done<WriteOutcome>, Error> {
    match write_note(&ctx.og, &note) {
        Ok(written) => {
            // 保存は済んでいるので、後処理の失敗では失敗にしない
            let queue_warning = ctx
                .queue
                .push(Job::Embed { kind: written.kind, slug: written.slug.clone() })
                .err()
                .map(|e| format!("埋め込みの登録に失敗しました: {e}"));
            let worker_warning = start_worker(ctx);
            Ok(Done { value: WriteOutcome::Saved(written.slug), queue_warning, worker_warning })
        }
        // 入力の誤りは直してもらう。積んでも通らない
        Err(e @ Error::Invalid(_)) => Err(e),
        // それ以外は、サーバー側の拒否でもメモを失わないよう、待ち行列に残す
        Err(e) => {
            ctx.queue.push(Job::WriteNote(note))?;
            let worker_warning = start_worker(ctx);
            Ok(Done { value: WriteOutcome::Queued(e), queue_warning: None, worker_warning })
        }
    }
}

/// inbox のノートを Permanent に昇格する。
pub fn promote(
    ctx: &Ctx,
    from: &[(NoteKind, String)],
    title: &str,
    body: &str,
) -> Result<Done<String>, Error> {
    let slug = crate::notes::promote(&ctx.og, from, title, body, Local::now())?;
    // 昇格は済んでいるので、後処理の失敗では失敗にしない
    let queue_warning = ctx
        .queue
        .push(Job::Promote { slug: slug.clone() })
        .err()
        .map(|e| format!("昇格しましたが、リンク提案を待ち行列に積めませんでした: {e}"));
    let worker_warning = start_worker(ctx);
    Ok(Done { value: slug, queue_warning, worker_warning })
}

/// 採用した提案を main に取り込み、却下したものを捨てる。
pub fn merge(ctx: &Ctx) -> Result<Done<Summary>, Error> {
    // apply が review.lock を自分で取る。ここでは持たない
    let summary = apply(&ctx.og, &ctx.paths.review_json(), &ctx.queue)?;
    let worker_warning = start_worker(ctx);
    Ok(Done { value: summary, queue_warning: None, worker_warning })
}

/// 失敗したジョブ 1 件をやり直す。
pub fn retry(ctx: &Ctx, id: &str) -> Result<Done<()>, Error> {
    ctx.queue.retry_failed(id)?;
    let worker_warning = start_worker(ctx);
    Ok(Done { value: (), queue_warning: None, worker_warning })
}

/// 失敗したジョブをすべてやり直す。件数を返す。
pub fn retry_all(ctx: &Ctx) -> Result<Done<usize>, Error> {
    let count = ctx.queue.failed()?.len();
    ctx.queue.retry_all_failed()?;
    let worker_warning = start_worker(ctx);
    Ok(Done { value: count, queue_warning: None, worker_warning })
}

/// inbox に出す近いノートの上限
const SIMILAR_INBOX: usize = 10;
/// Permanent に出す近いノートの上限
const SIMILAR_PERMANENT: usize = 3;

/// その種類のノートの件数。
fn count_notes(og: &Omnigraph, kind: NoteKind) -> Result<usize, Error> {
    let q = format!("query q() {{ match {{ $n: {} }} return {{ $n.slug as slug }} }}", kind_type(kind));
    Ok(og.query("main", &q, json!({}))?.len())
}

fn kind_type(kind: NoteKind) -> &'static str {
    match kind {
        NoteKind::Fleeting => "Fleeting",
        NoteKind::Literature => "Literature",
        NoteKind::Permanent => "Permanent",
    }
}

fn permanent_near(ctx: &Ctx, kind: NoteKind, slug: &str, embedding: &[f32]) -> Result<Vec<Near>, Error> {
    let min = ctx.config.link.inbox_similarity;
    // 自分が上位に入りうるので、1 件多く引いてから自分を除く
    let mut near = nearest(&ctx.og, "main", &[NoteKind::Permanent], embedding, SIMILAR_PERMANENT + 1, min)?;
    near.retain(|n| !(n.kind == kind && n.slug == slug));
    near.truncate(SIMILAR_PERMANENT);
    Ok(near)
}

/// 指定したノートに似ている Permanent を上位 3 件まで返す。埋め込みがまだなら空。
pub fn similar_permanent(ctx: &Ctx, kind: NoteKind, slug: &str) -> Result<Vec<Near>, Error> {
    let Some(v) = get_embedding(&ctx.og, kind, slug)? else {
        return Ok(Vec::new());
    };
    permanent_near(ctx, kind, slug, &v)
}

/// そのノートに近い inbox と Permanent を返す。埋め込みがまだならどちらも空。
pub fn similar(ctx: &Ctx, slug: &str) -> Result<Similar, Error> {
    let me = show_note(&ctx.og, slug)?;
    let min = ctx.config.link.inbox_similarity;
    let Some(v) = get_embedding(&ctx.og, me.kind, &me.slug)? else {
        return Ok(Similar { inbox: Vec::new(), permanent: Vec::new() });
    };
    // inbox は昇格していないものだけ。`nearest` は昇格を見ないので、inbox の組に絞る。
    // 昇格済みが上位を埋めて本物の inbox を押し出さないよう、Fleeting と Literature の全件まで引く。
    let members: HashSet<(NoteKind, String)> =
        inbox(&ctx.og)?.into_iter().map(|i| (i.kind, i.slug)).collect();
    let mut inbox_near = if members.is_empty() {
        Vec::new()
    } else {
        let all = count_notes(&ctx.og, NoteKind::Fleeting)? + count_notes(&ctx.og, NoteKind::Literature)?;
        nearest(&ctx.og, "main", &[NoteKind::Fleeting, NoteKind::Literature], &v, all.max(members.len() + 1), min)?
    };
    inbox_near.retain(|n| members.contains(&(n.kind, n.slug.clone())) && !(n.kind == me.kind && n.slug == me.slug));
    inbox_near.truncate(SIMILAR_INBOX);
    let permanent = permanent_near(ctx, me.kind, &me.slug, &v)?;
    Ok(Similar { inbox: inbox_near, permanent })
}
