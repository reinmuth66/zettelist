//! `zl` のコマンドライン。引数の定義、操作の振り分け、出力 (人向け / JSON)、終了コード。
//!
//! サブコマンドは持たない。`zl "<本文>"` は必ずメモで、操作はすべて `--` で始まるオプション。
//! 操作のオプションは同時に 1 つだけで、本文とも同時に使えない。

use crate::backup::{self, Outcome};
use crate::error::Error;
use crate::notes::{inbox, show_note, NewNote, NoteKind};
use crate::ops::{self, Similar, WriteOutcome};
use crate::review::{decide, list_proposals, Decision, Proposal, ReviewState};
use crate::worker::{reembed_all, run_worker, run_worker_once};
use crate::Ctx;
use chrono::{DateTime, Local, SecondsFormat, Utc};
use clap::error::ErrorKind;
use clap::{ArgGroup, Parser};
use serde_json::json;
use std::process::ExitCode;

/// Zettelkasten on Omnigraph の書き込みと整理の道具
#[derive(Parser)]
#[command(name = "zettelist", version)]
#[command(group = ArgGroup::new("action").args(["worker", "reembed", "backup", "inbox", "show", "similar", "review", "promote", "delete", "adopt", "reject", "undecide", "merge", "failed", "retry", "retry_all", "discard", "skill_export"]).multiple(false))]
#[command(group = ArgGroup::new("promote_body").args(["promote_body_text", "promote_body_file"]).multiple(false))]
struct Cli {
    /// 出典 (ISBN、DOI、arXiv ID、URL、または登録済みのスラッグ)
    #[arg(short = 's', long = "source", value_name = "出典", conflicts_with = "action")]
    source: Option<String>,

    /// 出典の中の位置 (ページなど)
    #[arg(short = 'p', long = "locator", value_name = "位置", requires = "source")]
    locator: Option<String>,

    /// 本文。`-` で始まるときは `--` のあとに置く
    #[arg(value_name = "本文", conflicts_with = "action")]
    body: Option<String>,

    /// 成功を標準出力に JSON 1 つで、失敗を標準エラーに JSON で出す
    #[arg(long, global = true)]
    json: bool,

    /// 待ち行列を処理する worker として動く (通常は zl が自分で起動する)
    #[arg(long)]
    worker: bool,

    /// worker が ready のものを 1 巡だけ処理して、待ちのあるものを待たずに終わる (テスト用)
    #[arg(long, hide = true, requires = "worker")]
    once: bool,

    /// すべてのノートの埋め込みを作り直す
    #[arg(long)]
    reembed: bool,

    /// main を backup.repo に書き出して commit する
    #[arg(long)]
    backup: bool,

    /// 整理されていないメモの一覧
    #[arg(long)]
    inbox: bool,

    /// ノート 1 件の全文
    #[arg(long, value_name = "スラッグ")]
    show: Option<String>,

    /// そのノートに近い inbox と Permanent
    #[arg(long, value_name = "スラッグ")]
    similar: Option<String>,

    /// 提案の一覧
    #[arg(long)]
    review: bool,

    /// 昇格元のノートから Permanent を作る (スラッグをカンマで区切る)。`--title` と、`--body` か `--body-file` が要る
    #[arg(long, value_name = "スラッグ,...", requires = "title", requires = "promote_body")]
    promote: Option<String>,

    /// 昇格する Permanent の題名
    #[arg(long, value_name = "題", requires = "promote")]
    title: Option<String>,

    /// 昇格する Permanent の本文
    #[arg(long = "body", value_name = "本文", requires = "promote")]
    promote_body_text: Option<String>,

    /// 昇格する Permanent の本文を読むファイル
    #[arg(long = "body-file", value_name = "パス", requires = "promote")]
    promote_body_file: Option<String>,

    /// ノートを消す (スラッグをカンマで区切る)。`--yes` が要る
    #[arg(long, value_name = "スラッグ,...")]
    delete: Option<String>,

    /// 提案を採用する (キーは `--review` に出る)
    #[arg(long, value_name = "キー")]
    adopt: Option<String>,

    /// 採用するリンクの理由
    #[arg(
        long,
        value_name = "理由",
        requires = "adopt",
        // 別の操作が選ばれていると `requires` だけでは弾けないので、並べて排他にする
        conflicts_with_all = ["worker", "reembed", "backup", "inbox", "show", "similar", "review", "promote", "delete", "reject", "undecide", "merge", "failed", "retry", "retry_all", "discard", "skill_export"]
    )]
    reason: Option<String>,

    /// 提案を却下する
    #[arg(long, value_name = "キー")]
    reject: Option<String>,

    /// 提案への判断を外す
    #[arg(long, value_name = "キー")]
    undecide: Option<String>,

    /// 採用した提案を main に取り込み、却下したものを捨てる。`--yes` が要る
    #[arg(long)]
    merge: bool,

    /// 失敗したジョブの一覧
    #[arg(long)]
    failed: bool,

    /// 失敗したジョブ 1 件をやり直す (id は `--failed` に出る)
    #[arg(long, value_name = "ID")]
    retry: Option<String>,

    /// 失敗したジョブをすべてやり直す
    #[arg(long)]
    retry_all: bool,

    /// 失敗したジョブ 1 件を捨てる。`--yes` が要る
    #[arg(long, value_name = "ID")]
    discard: Option<String>,

    /// エージェント向けの skill (SKILL.md) をそのディレクトリに書き出す (なければ作る)
    #[arg(long, value_name = "ディレクトリ")]
    skill_export: Option<String>,

    /// `--skill-export` で、すでにある SKILL.md を上書きする
    #[arg(
        long,
        requires = "skill_export",
        // 別の操作が選ばれていると `requires` だけでは弾けないので、並べて排他にする
        conflicts_with_all = ["worker", "reembed", "backup", "inbox", "show", "similar", "review", "promote", "delete", "adopt", "reject", "undecide", "merge", "failed", "retry", "retry_all", "discard"]
    )]
    force: bool,

    /// 消す・捨てる・マージするを確かめた上で行う
    #[arg(long)]
    yes: bool,
}

/// 昇格する Permanent の本文の出どころ。
enum BodySource {
    Text(String),
    File(String),
}

/// 1 回の実行で行う操作。
enum Action {
    Write(NewNote),
    Worker { once: bool },
    Reembed,
    Backup,
    Inbox,
    Show(String),
    Similar(String),
    Review,
    Promote { slugs: String, title: String, body: BodySource },
    Delete { slugs: String, yes: bool },
    Adopt { key: String, reason: Option<String> },
    Reject(String),
    Undecide(String),
    Merge { yes: bool },
    Failed,
    Retry(String),
    RetryAll,
    Discard { id: String, yes: bool },
    SkillExport { dir: String, force: bool },
    Tui,
}

impl Cli {
    fn action(self) -> Action {
        if self.worker {
            Action::Worker { once: self.once }
        } else if self.reembed {
            Action::Reembed
        } else if self.backup {
            Action::Backup
        } else if self.inbox {
            Action::Inbox
        } else if let Some(slug) = self.show {
            Action::Show(slug)
        } else if let Some(slug) = self.similar {
            Action::Similar(slug)
        } else if self.review {
            Action::Review
        } else if let Some(slugs) = self.promote {
            // clap が `requires` で、題名と本文があることを保証している
            let body = match (self.promote_body_text, self.promote_body_file) {
                (Some(text), _) => BodySource::Text(text),
                (None, file) => BodySource::File(file.unwrap_or_default()),
            };
            Action::Promote { slugs, title: self.title.unwrap_or_default(), body }
        } else if let Some(slugs) = self.delete {
            Action::Delete { slugs, yes: self.yes }
        } else if let Some(key) = self.adopt {
            Action::Adopt { key, reason: self.reason }
        } else if let Some(key) = self.reject {
            Action::Reject(key)
        } else if let Some(key) = self.undecide {
            Action::Undecide(key)
        } else if self.merge {
            Action::Merge { yes: self.yes }
        } else if self.failed {
            Action::Failed
        } else if let Some(id) = self.retry {
            Action::Retry(id)
        } else if self.retry_all {
            Action::RetryAll
        } else if let Some(id) = self.discard {
            Action::Discard { id, yes: self.yes }
        } else if let Some(dir) = self.skill_export {
            Action::SkillExport { dir, force: self.force }
        } else if let Some(body) = self.body {
            Action::Write(NewNote { body, source: self.source, locator: self.locator, created: Local::now() })
        } else {
            Action::Tui
        }
    }
}

pub fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => return usage_error(e),
    };
    let json = cli.json;
    match dispatch(json, cli.action()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(json, &e),
    }
}

/// clap の誤り。`--json` が (`--` より前に) あれば JSON で、なければ clap の表示のまま出す。
/// `--help` と `--version` は誤りではないので、そのまま表示する。
fn usage_error(e: clap::Error) -> ExitCode {
    if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) || !json_requested() {
        e.exit();
    }
    fail(true, &Error::Invalid(clap_message(&e)))
}

fn json_requested() -> bool {
    std::env::args_os().skip(1).take_while(|a| a != "--").any(|a| a == "--json")
}

/// clap の表示から、先頭の `error: ` と、そのあとの使い方の案内を除いた本文。
fn clap_message(e: &clap::Error) -> String {
    let rendered = e.render().to_string();
    let first = rendered.split("\n\n").next().unwrap_or("").trim();
    first.strip_prefix("error: ").unwrap_or(first).to_string()
}

/// 成功の出力。JSON のときは値を 1 つ、人向けのときは文言を標準出力に出す。
fn output(json: bool, value: serde_json::Value, human: &str) {
    if json {
        println!("{value}");
    } else {
        println!("{human}");
    }
}

/// 失敗の出力と終了コード。入力の誤り (Invalid) は 2、ほかは 1。
fn fail(json: bool, e: &Error) -> ExitCode {
    if json {
        eprintln!("{}", json!({"error": e.to_string(), "kind": e.kind()}));
    } else {
        eprintln!("{e}");
    }
    match e {
        Error::Invalid(_) => ExitCode::from(2),
        _ => ExitCode::FAILURE,
    }
}

fn dispatch(json: bool, action: Action) -> Result<(), Error> {
    match action {
        Action::Tui => {
            use std::io::IsTerminal;
            if json {
                return Err(Error::Invalid(
                    "--json では TUI を起動できません。操作のオプションか本文を渡します".into(),
                ));
            }
            if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                return Err(Error::Invalid("TUI は端末で起動します".into()));
            }
            let ctx = Ctx::from_env()?;
            crate::tui::run(ctx)
        }
        Action::Backup => backup_now(json),
        Action::Worker { once } => {
            let ctx = Ctx::from_env()?;
            if once { run_worker_once(&ctx) } else { run_worker(&ctx) }
        }
        Action::Reembed => {
            let ctx = Ctx::from_env()?;
            let count = reembed_all(&ctx)?;
            warn_worker(ops::start_worker(&ctx));
            output(json, json!({"count": count}), &format!("{count} 件の埋め込みを積みました"));
            Ok(())
        }
        Action::Write(note) => write(json, note),
        Action::Inbox => inbox_list(json),
        Action::Show(slug) => show(json, &slug),
        Action::Similar(slug) => similar(json, &slug),
        Action::Promote { slugs, title, body } => promote_notes(json, &slugs, &title, body),
        Action::Delete { slugs, yes } => delete_notes_cmd(json, &slugs, yes),
        Action::Review => review_list(json),
        Action::Adopt { key, reason } => decide_cmd(json, &key, Some(Decision::Adopt { reason }), "採用しました"),
        Action::Reject(key) => decide_cmd(json, &key, Some(Decision::Reject), "却下しました"),
        Action::Undecide(key) => decide_cmd(json, &key, None, "判断を外しました"),
        Action::Merge { yes } => merge(json, yes),
        Action::Failed => failed_list(json),
        Action::Retry(id) => retry(json, &id),
        Action::RetryAll => retry_all(json),
        Action::Discard { id, yes } => discard(json, &id, yes),
        Action::SkillExport { dir, force } => skill_export(json, &dir, force),
    }
}

fn kind_name(kind: NoteKind) -> &'static str {
    match kind {
        NoteKind::Fleeting => "fleeting",
        NoteKind::Literature => "literature",
        NoteKind::Permanent => "permanent",
    }
}

/// 人向けの一覧に出す本文の 1 行 (最初の行を 60 文字まで)。
fn one_line(body: &str) -> String {
    let first = body.lines().next().unwrap_or("");
    let mut line: String = first.chars().take(60).collect();
    if first.chars().count() > 60 || body.lines().nth(1).is_some() {
        line.push('…');
    }
    line
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// 類似度は JSON では小数 4 桁にそろえる。
fn rounded(similarity: f32) -> f64 {
    (f64::from(similarity) * 1e4).round() / 1e4
}

/// `--inbox`
fn inbox_list(json: bool) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    let items = inbox(&ctx.og)?;
    let now = Utc::now();
    let value: Vec<_> = items
        .iter()
        .map(|i| {
            json!({
                "kind": kind_name(i.kind),
                "slug": i.slug,
                "body": i.body,
                "created": rfc3339(i.created),
                "old": i.is_old(now),
                "source_id": i.source_id,
                "source_slug": i.source_slug,
                "locator": i.locator,
            })
        })
        .collect();
    let human = if items.is_empty() {
        "整理されていないメモはありません".to_string()
    } else {
        items
            .iter()
            .map(|i| {
                let old = if i.is_old(now) { " (古い)" } else { "" };
                format!("{}  {}{}  {}", i.slug, kind_name(i.kind), old, one_line(&i.body))
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    output(json, json!(value), &human);
    Ok(())
}

/// `--show <slug>`
fn show(json: bool, slug: &str) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    let note = show_note(&ctx.og, slug)?;
    let human = match &note.title {
        Some(title) => format!("{} {}\n{title}\n\n{}", note.slug, kind_name(note.kind), note.body),
        None => format!("{} {}\n\n{}", note.slug, kind_name(note.kind), note.body),
    };
    output(
        json,
        json!({
            "kind": kind_name(note.kind),
            "slug": note.slug,
            "title": note.title,
            "body": note.body,
            "created": rfc3339(note.created),
        }),
        &human,
    );
    Ok(())
}

/// カンマ区切りのスラッグを、順序を保って重複なしの一覧にする。空なら入力の誤り。
fn parse_slugs(list: &str) -> Result<Vec<String>, Error> {
    let mut slugs: Vec<String> = Vec::new();
    for s in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !slugs.iter().any(|x| x == s) {
            slugs.push(s.to_string());
        }
    }
    if slugs.is_empty() {
        return Err(Error::Invalid("スラッグがありません".to_string()));
    }
    Ok(slugs)
}

/// スラッグの一覧を (種類, スラッグ) にする。1 件でも知らないスラッグがあれば入力の誤り。
fn resolve_kinds(ctx: &Ctx, slugs: &[String]) -> Result<Vec<(NoteKind, String)>, Error> {
    slugs.iter().map(|s| show_note(&ctx.og, s).map(|n| (n.kind, n.slug))).collect()
}

/// `--promote <slug,...> --title <題> (--body <本文> | --body-file <パス>)`
fn promote_notes(json: bool, slugs: &str, title: &str, body: BodySource) -> Result<(), Error> {
    let slugs = parse_slugs(slugs)?;
    let body = match body {
        BodySource::Text(text) => text,
        BodySource::File(path) => std::fs::read_to_string(&path)
            .map_err(|e| Error::Invalid(format!("本文のファイル '{path}' を読めません: {e}")))?,
    };
    let ctx = Ctx::from_env()?;
    let from = resolve_kinds(&ctx, &slugs)?;
    let done = ops::promote(&ctx, &from, title, &body)?;
    if let Some(message) = done.queue_warning {
        if json {
            eprintln!("{}", json!({"warning": message, "slug": done.value}));
        } else {
            eprintln!("{message}");
        }
    }
    warn_worker(done.worker_warning);
    output(json, json!({"slug": done.value}), &format!("昇格しました {}", done.value));
    Ok(())
}

/// `--delete <slug,...> --yes`
fn delete_notes_cmd(json: bool, slugs: &str, yes: bool) -> Result<(), Error> {
    if !yes {
        return Err(Error::Invalid("ノートを消すには --yes が要ります".to_string()));
    }
    let slugs = parse_slugs(slugs)?;
    let ctx = Ctx::from_env()?;
    // 先にすべての種類を確かめる。知らないスラッグがあれば、何も消さない
    let notes = resolve_kinds(&ctx, &slugs)?;
    crate::notes::delete_notes(&ctx.og, &notes)?;
    output(json, json!({"ok": true}), &format!("{} 件消しました", notes.len()));
    Ok(())
}

/// `--review`
fn review_list(json: bool) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    let proposals = list_proposals(&ctx.og)?;
    let state = ReviewState::load(&ctx.paths.review_json())?;
    let mark = |d: Option<&Decision>| match d {
        Some(Decision::Adopt { .. }) => "採用",
        Some(Decision::Reject) => "却下",
        None => "未判断",
    };
    let mut values = Vec::new();
    let mut lines = Vec::new();
    for p in &proposals {
        let key = p.key();
        let decision = state.decisions.get(&key);
        let (name, reason) = match decision {
            Some(Decision::Adopt { reason }) => (json!("adopt"), json!(reason)),
            Some(Decision::Reject) => (json!("reject"), json!(null)),
            None => (json!(null), json!(null)),
        };
        let mut line = format!("{key}  {}", mark(decision));
        match p {
            Proposal::Link { from, to, similarity } => {
                values.push(json!({
                    "key": key, "type": "link", "from": from, "to": to,
                    "similarity": rounded(*similarity), "decision": name, "reason": reason,
                }));
                line = format!("{line}  {similarity:.2}");
            }
            Proposal::Hub { hub, note } => {
                values.push(json!({
                    "key": key, "type": "hub", "hub": hub, "note": note,
                    "decision": name, "reason": reason,
                }));
            }
        }
        if let Some(Decision::Adopt { reason: Some(r) }) = decision {
            line = format!("{line}  {r}");
        }
        lines.push(line);
    }
    let human = if lines.is_empty() { "提案はありません".to_string() } else { lines.join("\n") };
    output(json, json!(values), &human);
    Ok(())
}

/// `--adopt <key> [--reason <理由>]`、`--reject <key>`、`--undecide <key>`
fn decide_cmd(json: bool, key: &str, decision: Option<Decision>, done: &str) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    // 空の理由は付けない
    let decision = match decision {
        Some(Decision::Adopt { reason }) => {
            Some(Decision::Adopt { reason: reason.map(|r| r.trim().to_string()).filter(|r| !r.is_empty()) })
        }
        other => other,
    };
    // decide が review.lock を自分で取る。ここでは持たない
    decide(&ctx.og, &ctx.paths.review_json(), &ctx.paths.review_lock(), key, decision)?;
    output(json, json!({"ok": true}), &format!("{done} {key}"));
    Ok(())
}

/// `--merge --yes`
fn merge(json: bool, yes: bool) -> Result<(), Error> {
    if !yes {
        return Err(Error::Invalid("マージするには --yes が要ります".to_string()));
    }
    let ctx = Ctx::from_env()?;
    let done = ops::merge(&ctx)?;
    warn_worker(done.worker_warning);
    output(
        json,
        json!({"links": done.value.links, "hubs": done.value.hubs}),
        &format!("マージしました (リンク {} 本、ハブ {} 件)", done.value.links, done.value.hubs),
    );
    Ok(())
}

/// `--failed`
fn failed_list(json: bool) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    let entries = ctx.queue.failed()?;
    let value: Vec<_> = entries
        .iter()
        .map(|e| json!({"id": e.id, "job": e.job.name(), "attempts": e.attempts, "last_error": e.last_error}))
        .collect();
    let human = if entries.is_empty() {
        "失敗したジョブはありません".to_string()
    } else {
        entries
            .iter()
            .map(|e| {
                let error = e.last_error.as_deref().map(one_line).unwrap_or_default();
                format!("{}  {}  {} 回  {error}", e.id, e.job.name(), e.attempts)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    output(json, json!(value), &human);
    Ok(())
}

/// `--retry <id>`
fn retry(json: bool, id: &str) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    let done = ops::retry(&ctx, id)?;
    warn_worker(done.worker_warning);
    output(json, json!({"ok": true}), &format!("やり直しを積みました {id}"));
    Ok(())
}

/// `--retry-all`
fn retry_all(json: bool) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    let done = ops::retry_all(&ctx)?;
    warn_worker(done.worker_warning);
    output(json, json!({"ok": true}), &format!("{} 件のやり直しを積みました", done.value));
    Ok(())
}

/// `--discard <id> --yes`
fn discard(json: bool, id: &str, yes: bool) -> Result<(), Error> {
    if !yes {
        return Err(Error::Invalid("失敗したジョブを捨てるには --yes が要ります".to_string()));
    }
    let ctx = Ctx::from_env()?;
    ctx.queue.discard_failed(id)?;
    output(json, json!({"ok": true}), &format!("捨てました {id}"));
    Ok(())
}

/// `--skill-export <dir> [--force]`。サーバーにも設定にも触らない。
fn skill_export(json: bool, dir: &str, force: bool) -> Result<(), Error> {
    crate::skill::export(std::path::Path::new(dir), force)?;
    output(json, json!({"ok": true}), &format!("skill を書き出しました {dir}/SKILL.md"));
    Ok(())
}

/// `--similar <slug>`
fn similar(json: bool, slug: &str) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    let Similar { inbox: inbox_near, permanent: permanent_near } = ops::similar(&ctx, slug)?;
    let value = json!({
        "inbox": inbox_near.iter().map(|n| json!({
            "kind": kind_name(n.kind),
            "slug": n.slug,
            "body": n.body,
            "similarity": rounded(n.similarity),
        })).collect::<Vec<_>>(),
        "permanent": permanent_near.iter().map(|n| json!({
            "slug": n.slug,
            "title": n.title,
            "body": n.body,
            "similarity": rounded(n.similarity),
        })).collect::<Vec<_>>(),
    });
    let section = |title: &str, lines: Vec<String>| {
        if lines.is_empty() {
            format!("{title}\n  (なし)")
        } else {
            format!("{title}\n{}", lines.join("\n"))
        }
    };
    let human = format!(
        "{}\n{}",
        section(
            "inbox",
            inbox_near
                .iter()
                .map(|n| format!("  {:.2}  {}  {}  {}", n.similarity, n.slug, kind_name(n.kind), one_line(&n.body)))
                .collect(),
        ),
        section(
            "permanent",
            permanent_near
                .iter()
                .map(|n| {
                    let title = n.title.as_deref().unwrap_or("");
                    format!("  {:.2}  {}  {}  {}", n.similarity, n.slug, title, one_line(&n.body))
                })
                .collect(),
        ),
    );
    output(json, value, &human);
    Ok(())
}

/// `--backup`
fn backup_now(json: bool) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    match backup::run(&ctx, "backup")? {
        Outcome::Committed => output(json, json!({"ok": true}), "バックアップしました"),
        Outcome::NoChanges => output(json, json!({"ok": true}), "変更はありません"),
        Outcome::NotConfigured => return Err(Error::Permanent("backup.repo が設定されていません".to_string())),
    }
    Ok(())
}

/// `zl "<本文>"`
fn write(json: bool, note: NewNote) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    let done = ops::write(&ctx, note)?;
    if let Some(w) = done.queue_warning {
        eprintln!("{w}");
    }
    warn_worker(done.worker_warning);
    match done.value {
        WriteOutcome::Saved(slug) => {
            output(json, json!({"slug": slug}), &format!("保存しました {slug}"));
        }
        WriteOutcome::Queued(e) => {
            let human = match e {
                Error::Transient(_) => "omnigraph-server に届かないので待ち行列に保存しました".to_string(),
                _ => format!("書き込めなかったので待ち行列に保存しました: {e}"),
            };
            output(json, json!({"queued": true}), &human);
        }
    }
    Ok(())
}

fn warn_worker(warning: Option<String>) {
    if let Some(w) = warning {
        eprintln!("{w}");
    }
}
