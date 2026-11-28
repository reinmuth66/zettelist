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

/// Write and organize a Zettelkasten on Omnigraph
#[derive(Parser)]
#[command(name = "zettelist", version)]
#[command(group = ArgGroup::new("action").args(["worker", "reembed", "backup", "inbox", "show", "similar", "review", "promote", "delete", "adopt", "reject", "undecide", "merge", "failed", "retry", "retry_all", "discard", "skill_export"]).multiple(false))]
#[command(group = ArgGroup::new("promote_body").args(["promote_body_text", "promote_body_file"]).multiple(false))]
struct Cli {
    /// Source (ISBN, DOI, arXiv ID, URL, or a registered slug)
    #[arg(short = 's', long = "source", value_name = "SOURCE", conflicts_with = "action")]
    source: Option<String>,

    /// Location within the source (page, etc.)
    #[arg(short = 'p', long = "locator", value_name = "LOCATOR", requires = "source")]
    locator: Option<String>,

    /// Note body. Put it after `--` when it starts with `-`
    #[arg(value_name = "BODY", conflicts_with = "action")]
    body: Option<String>,

    /// Print success as one JSON value on stdout and failure as JSON on stderr
    #[arg(long, global = true)]
    json: bool,

    /// Run as the worker that processes the queue (zl normally starts it itself)
    #[arg(long)]
    worker: bool,

    /// Process ready jobs once and exit without waiting for delayed ones (for tests)
    #[arg(long, hide = true, requires = "worker")]
    once: bool,

    /// Rebuild the embeddings of all notes
    #[arg(long)]
    reembed: bool,

    /// Export main to backup.repo and commit it
    #[arg(long)]
    backup: bool,

    /// List unprocessed notes
    #[arg(long)]
    inbox: bool,

    /// Show the full text of one note
    #[arg(long, value_name = "SLUG")]
    show: Option<String>,

    /// Inbox and permanent notes similar to the note
    #[arg(long, value_name = "SLUG")]
    similar: Option<String>,

    /// List proposals
    #[arg(long)]
    review: bool,

    /// Create a permanent note from source notes (comma-separated slugs). Needs `--title` and `--body` or `--body-file`
    #[arg(long, value_name = "SLUG,...", requires = "title", requires = "promote_body")]
    promote: Option<String>,

    /// Title of the permanent note to promote
    #[arg(long, value_name = "TITLE", requires = "promote")]
    title: Option<String>,

    /// Body of the permanent note to promote
    #[arg(long = "body", value_name = "BODY", requires = "promote")]
    promote_body_text: Option<String>,

    /// File to read the body of the permanent note from
    #[arg(long = "body-file", value_name = "PATH", requires = "promote")]
    promote_body_file: Option<String>,

    /// Delete notes (comma-separated slugs). Needs `--yes`
    #[arg(long, value_name = "SLUG,...")]
    delete: Option<String>,

    /// Adopt a proposal (keys are shown by `--review`)
    #[arg(long, value_name = "KEY")]
    adopt: Option<String>,

    /// Reason for the adopted link
    #[arg(
        long,
        value_name = "REASON",
        requires = "adopt",
        // 別の操作が選ばれていると `requires` だけでは弾けないので、並べて排他にする
        conflicts_with_all = ["worker", "reembed", "backup", "inbox", "show", "similar", "review", "promote", "delete", "reject", "undecide", "merge", "failed", "retry", "retry_all", "discard", "skill_export"]
    )]
    reason: Option<String>,

    /// Reject a proposal
    #[arg(long, value_name = "KEY")]
    reject: Option<String>,

    /// Clear the decision on a proposal
    #[arg(long, value_name = "KEY")]
    undecide: Option<String>,

    /// Merge adopted proposals into main and drop rejected ones. Needs `--yes`
    #[arg(long)]
    merge: bool,

    /// List failed jobs
    #[arg(long)]
    failed: bool,

    /// Retry one failed job (ids are shown by `--failed`)
    #[arg(long, value_name = "ID")]
    retry: Option<String>,

    /// Retry all failed jobs
    #[arg(long)]
    retry_all: bool,

    /// Discard one failed job. Needs `--yes`
    #[arg(long, value_name = "ID")]
    discard: Option<String>,

    /// Write the agent skill (SKILL.md) into the directory (created if missing)
    #[arg(long, value_name = "DIR")]
    skill_export: Option<String>,

    /// Overwrite an existing SKILL.md with `--skill-export`
    #[arg(
        long,
        requires = "skill_export",
        // 別の操作が選ばれていると `requires` だけでは弾けないので、並べて排他にする
        conflicts_with_all = ["worker", "reembed", "backup", "inbox", "show", "similar", "review", "promote", "delete", "adopt", "reject", "undecide", "merge", "failed", "retry", "retry_all", "discard"]
    )]
    force: bool,

    /// Confirm deleting, discarding, or merging
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
                    "cannot start the TUI with --json; pass an action option or a note body".into(),
                ));
            }
            if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                return Err(Error::Invalid("the TUI needs a terminal".into()));
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
            output(json, json!({"count": count}), &format!("Queued {count} embeddings"));
            Ok(())
        }
        Action::Write(note) => write(json, note),
        Action::Inbox => inbox_list(json),
        Action::Show(slug) => show(json, &slug),
        Action::Similar(slug) => similar(json, &slug),
        Action::Promote { slugs, title, body } => promote_notes(json, &slugs, &title, body),
        Action::Delete { slugs, yes } => delete_notes_cmd(json, &slugs, yes),
        Action::Review => review_list(json),
        Action::Adopt { key, reason } => decide_cmd(json, &key, Some(Decision::Adopt { reason }), "Adopted"),
        Action::Reject(key) => decide_cmd(json, &key, Some(Decision::Reject), "Rejected"),
        Action::Undecide(key) => decide_cmd(json, &key, None, "Cleared the decision on"),
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
        "Inbox is empty".to_string()
    } else {
        items
            .iter()
            .map(|i| {
                let old = if i.is_old(now) { " (old)" } else { "" };
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
        return Err(Error::Invalid("no slugs given".to_string()));
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
            .map_err(|e| Error::Invalid(format!("cannot read body file '{path}': {e}")))?,
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
    output(json, json!({"slug": done.value}), &format!("Promoted {}", done.value));
    Ok(())
}

/// `--delete <slug,...> --yes`
fn delete_notes_cmd(json: bool, slugs: &str, yes: bool) -> Result<(), Error> {
    if !yes {
        return Err(Error::Invalid("deleting notes needs --yes".to_string()));
    }
    let slugs = parse_slugs(slugs)?;
    let ctx = Ctx::from_env()?;
    // 先にすべての種類を確かめる。知らないスラッグがあれば、何も消さない
    let notes = resolve_kinds(&ctx, &slugs)?;
    crate::notes::delete_notes(&ctx.og, &notes)?;
    output(json, json!({"ok": true}), &format!("Deleted {} note(s)", notes.len()));
    Ok(())
}

/// `--review`
fn review_list(json: bool) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    let proposals = list_proposals(&ctx.og)?;
    let state = ReviewState::load(&ctx.paths.review_json())?;
    let mark = |d: Option<&Decision>| match d {
        Some(Decision::Adopt { .. }) => "adopted",
        Some(Decision::Reject) => "rejected",
        None => "undecided",
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
    let human = if lines.is_empty() { "No proposals".to_string() } else { lines.join("\n") };
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
        return Err(Error::Invalid("merging needs --yes".to_string()));
    }
    let ctx = Ctx::from_env()?;
    let done = ops::merge(&ctx)?;
    warn_worker(done.worker_warning);
    output(
        json,
        json!({"links": done.value.links, "hubs": done.value.hubs}),
        &format!("Merged ({} links, {} hubs)", done.value.links, done.value.hubs),
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
        "No failed jobs".to_string()
    } else {
        entries
            .iter()
            .map(|e| {
                let error = e.last_error.as_deref().map(one_line).unwrap_or_default();
                format!("{}  {}  {}x  {error}", e.id, e.job.name(), e.attempts)
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
    output(json, json!({"ok": true}), &format!("Queued a retry of {id}"));
    Ok(())
}

/// `--retry-all`
fn retry_all(json: bool) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    let done = ops::retry_all(&ctx)?;
    warn_worker(done.worker_warning);
    output(json, json!({"ok": true}), &format!("Queued {} retries", done.value));
    Ok(())
}

/// `--discard <id> --yes`
fn discard(json: bool, id: &str, yes: bool) -> Result<(), Error> {
    if !yes {
        return Err(Error::Invalid("discarding a failed job needs --yes".to_string()));
    }
    let ctx = Ctx::from_env()?;
    ctx.queue.discard_failed(id)?;
    output(json, json!({"ok": true}), &format!("Discarded {id}"));
    Ok(())
}

/// `--skill-export <dir> [--force]`。サーバーにも設定にも触らない。
fn skill_export(json: bool, dir: &str, force: bool) -> Result<(), Error> {
    crate::skill::export(std::path::Path::new(dir), force)?;
    output(json, json!({"ok": true}), &format!("Wrote the skill to {dir}/SKILL.md"));
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
            format!("{title}\n  (none)")
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
        Outcome::Committed => output(json, json!({"ok": true}), "Backed up"),
        Outcome::NoChanges => output(json, json!({"ok": true}), "No changes"),
        Outcome::NotConfigured => return Err(Error::Permanent("backup.repo is not set".to_string())),
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
            output(json, json!({"slug": slug}), &format!("Saved {slug}"));
        }
        WriteOutcome::Queued(e) => {
            let human = match e {
                Error::Transient(_) => "omnigraph-server is unreachable; queued the note".to_string(),
                _ => format!("Could not write; queued the note: {e}"),
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
