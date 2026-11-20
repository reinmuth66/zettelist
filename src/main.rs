use chrono::Local;
use clap::Parser;
use std::process::ExitCode;
use zettelist::backup::{self, Outcome};
use zettelist::error::Error;
use zettelist::notes::{write_note, NewNote};
use zettelist::queue::Job;
use zettelist::worker::{reembed_all, run_worker, run_worker_once, spawn_worker};
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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.worker || cli.reembed {
        return finish(admin(&cli));
    }
    if cli.backup {
        return backup_now();
    }
    let Some(body) = cli.body else {
        println!("TUI は計画 3 で作ります");
        return ExitCode::SUCCESS;
    };
    finish(run(NewNote { body, source: cli.source, locator: cli.locator, created: Local::now() }))
}

fn finish(result: Result<(), Error>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// `--worker` と `--reembed`
fn admin(cli: &Cli) -> Result<(), Error> {
    let ctx = Ctx::from_env()?;
    if cli.worker {
        return if cli.once { run_worker_once(&ctx) } else { run_worker(&ctx) };
    }
    let count = reembed_all(&ctx)?;
    start_worker(&ctx);
    println!("{count} 件の埋め込みを積みました");
    Ok(())
}

/// `--backup`
fn backup_now() -> ExitCode {
    let result = Ctx::from_env().and_then(|ctx| backup::run(&ctx, "backup"));
    match result {
        Ok(Outcome::Committed) => {
            println!("バックアップしました");
            ExitCode::SUCCESS
        }
        Ok(Outcome::NoChanges) => {
            println!("変更はありません");
            ExitCode::SUCCESS
        }
        Ok(Outcome::NotConfigured) => {
            eprintln!("backup.repo が設定されていません");
            ExitCode::FAILURE
        }
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
        // 入力の誤りは直してもらう。積んでも通らない
        Err(e @ Error::Invalid(_)) => Err(e),
        // それ以外は、サーバー側の拒否でもメモを失わないよう、待ち行列に残す
        Err(e) => {
            ctx.queue.push(Job::WriteNote(note))?;
            start_worker(&ctx);
            match e {
                Error::Transient(_) => println!("omnigraph-server に届かないので待ち行列に保存しました"),
                _ => println!("書き込めなかったので待ち行列に保存しました: {e}"),
            }
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
