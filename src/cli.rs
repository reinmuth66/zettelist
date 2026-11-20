//! `zl` のコマンドライン。引数の定義、操作の振り分け、出力 (人向け / JSON)、終了コード。
//!
//! サブコマンドは持たない。`zl "<本文>"` は必ずメモで、操作はすべて `--` で始まるオプション。
//! 操作のオプションは同時に 1 つだけで、本文とも同時に使えない。

use crate::backup::{self, Outcome};
use crate::error::Error;
use crate::notes::{get_embedding, inbox, nearest, show_note, write_note, NewNote, Near, NoteKind};
use crate::queue::Job;
use crate::worker::{reembed_all, run_worker, run_worker_once, spawn_worker};
use crate::Ctx;
use chrono::{DateTime, Local, SecondsFormat, Utc};
use clap::error::ErrorKind;
use clap::{ArgGroup, Parser};
use serde_json::json;
use std::process::ExitCode;

/// Zettelkasten on Omnigraph の書き込みと整理の道具
#[derive(Parser)]
#[command(name = "zettelist", version)]
#[command(group = ArgGroup::new("action").args(["worker", "reembed", "backup", "inbox", "show", "similar", "review"]).multiple(false))]
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
            output(json, json!({"ok": true}), "TUI は計画 3 で作ります");
            Ok(())
        }
        Action::Backup => backup_now(json),
        Action::Worker { once } => {
            let ctx = Ctx::from_env()?;
            if once { run_worker_once(&ctx) } else { run_worker(&ctx) }
        }
        Action::Reembed => {
            let ctx = Ctx::from_env()?;
            let count = reembed_all(&ctx)?;
            start_worker(&ctx);
            output(json, json!({"count": count}), &format!("{count} 件の埋め込みを積みました"));
            Ok(())
        }
        Action::Write(note) => write(json, note),
        Action::Inbox => inbox_list(json),
        Action::Show(slug) => show(json, &slug),
        Action::Similar(slug) => similar(json, &slug),
        // 次のタスクで実装する
        Action::Review => Err(Error::Permanent("未実装".to_string())),
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

/// inbox に出す近いノートの上限
const SIMILAR_INBOX: usize = 10;
/// Permanent に出す近いノートの上限
const SIMILAR_PERMANENT: usize = 3;

/// `--similar <slug>`
fn similar(json: bool, slug: &str) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    let me = show_note(&ctx.og, slug)?;
    let min = ctx.config.link.inbox_similarity;
    let (inbox_near, permanent_near) = match get_embedding(&ctx.og, me.kind, &me.slug)? {
        // 埋め込みがまだなら、どちらも空
        None => (Vec::new(), Vec::new()),
        Some(v) => {
            // 自分が上位に入りうるので、1 件多く引いてから自分を除く
            let not_me = |n: &Near| !(n.kind == me.kind && n.slug == me.slug);
            let mut inbox_near =
                nearest(&ctx.og, "main", &[NoteKind::Fleeting, NoteKind::Literature], &v, SIMILAR_INBOX + 1, min)?;
            inbox_near.retain(not_me);
            inbox_near.truncate(SIMILAR_INBOX);
            let mut permanent_near = nearest(&ctx.og, "main", &[NoteKind::Permanent], &v, SIMILAR_PERMANENT + 1, min)?;
            permanent_near.retain(not_me);
            permanent_near.truncate(SIMILAR_PERMANENT);
            (inbox_near, permanent_near)
        }
    };
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
    match write_note(&ctx.og, &note) {
        Ok(written) => {
            // 保存は済んでいるので、後処理の失敗では終了コードを変えない
            if let Err(e) = ctx.queue.push(Job::Embed { kind: written.kind, slug: written.slug.clone() }) {
                eprintln!("埋め込みの登録に失敗しました: {e}");
            }
            start_worker(&ctx);
            output(json, json!({"slug": written.slug}), &format!("保存しました {}", written.slug));
            Ok(())
        }
        // 入力の誤りは直してもらう。積んでも通らない
        Err(e @ Error::Invalid(_)) => Err(e),
        // それ以外は、サーバー側の拒否でもメモを失わないよう、待ち行列に残す
        Err(e) => {
            ctx.queue.push(Job::WriteNote(note))?;
            start_worker(&ctx);
            let human = match e {
                Error::Transient(_) => "omnigraph-server に届かないので待ち行列に保存しました".to_string(),
                _ => format!("書き込めなかったので待ち行列に保存しました: {e}"),
            };
            output(json, json!({"queued": true}), &human);
            Ok(())
        }
    }
}

/// worker の起動に失敗しても、保存は済んでいるので警告だけにする
fn start_worker(ctx: &Ctx) {
    if let Err(e) = spawn_worker(&ctx.paths) {
        eprintln!("worker を起動できません: {e}");
    }
}
