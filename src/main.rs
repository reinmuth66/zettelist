use chrono::Local;
use clap::Parser;
use std::process::ExitCode;
use zettelist::error::Error;
use zettelist::notes::{write_note, NewNote};
use zettelist::queue::Job;
use zettelist::worker::spawn_worker;
use zettelist::Ctx;

/// Zettelkasten on Omnigraph の書き込みと整理の道具
#[derive(Parser)]
#[command(name = "zettelist", version)]
struct Cli {
    /// 出典 (ISBN、DOI、arXiv ID、URL、または登録済みのスラッグ)
    #[arg(short = 's', long = "source", value_name = "出典")]
    source: Option<String>,

    /// 出典の中の位置 (ページなど)
    #[arg(short = 'p', long = "locator", value_name = "位置")]
    locator: Option<String>,

    /// 本文。`-` で始まるときは `--` のあとに置く
    #[arg(value_name = "本文")]
    body: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(body) = cli.body else {
        println!("TUI は計画 3 で作ります");
        return ExitCode::SUCCESS;
    };
    match run(NewNote { body, source: cli.source, locator: cli.locator, created: Local::now() }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run(note: NewNote) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    match write_note(&ctx.og, &note) {
        Ok(written) => {
            // 保存は済んでいるので、後処理の失敗では終了コードを変えない
            if let Err(e) = ctx.queue.push(Job::Embed { kind: written.kind, slug: written.slug.clone() }) {
                eprintln!("埋め込みの登録に失敗しました: {e}");
            }
            start_worker(&ctx);
            println!("保存しました {}", written.slug);
            Ok(())
        }
        Err(Error::Transient(_)) => {
            ctx.queue.push(Job::WriteNote(note))?;
            start_worker(&ctx);
            println!("omnigraph-server に届かないので待ち行列に保存しました");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn start_worker(ctx: &Ctx) {
    if std::env::var_os("ZETTELIST_NO_SPAWN").is_some() {
        return;
    }
    if let Err(e) = spawn_worker(&ctx.paths) {
        eprintln!("worker を起動できません: {e}");
    }
}
