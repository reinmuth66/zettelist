//! 待ち行列を処理する worker。

use crate::error::Error;
use crate::notes::{note_text, set_embedding, write_note, NoteKind};
use crate::paths::Paths;
use crate::propose::propose_for;
use crate::queue::{Job, Queue};
use crate::review::lock_review;
use crate::Ctx;
use serde_json::json;
use std::fs::{self, File, OpenOptions};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

const BRANCH: &str = "main";

/// 1 件の Job を実行する。
pub fn execute(ctx: &Ctx, job: &Job) -> Result<(), Error> {
    match job {
        Job::WriteNote(note) => {
            // 再実行しても同じノートに当たる (write_note は冪等) ので、Embed を積むところで失敗しても繰り返せる
            let written = write_note(&ctx.og, note)?;
            ctx.queue
                .push(Job::Embed { kind: written.kind, slug: written.slug })
                .map(|_| ())
                .map_err(|e| Error::Transient(format!("埋め込みの登録に失敗しました: {e}")))
        }
        Job::Embed { kind, slug } => embed_note(ctx, *kind, slug).map(|_| ()),
        Job::Promote { slug } => {
            // 昇格したあとに消えていたら、することがない
            if !embed_note(ctx, NoteKind::Permanent, slug)? {
                return Ok(());
            }
            // 提案を書く間だけ review.lock を持つ (埋め込みの間は持たない)。apply の途中なら待つ
            let _lock = lock_review(&ctx.paths.review_lock())?;
            propose_for(&ctx.og, ctx.embedder.as_ref(), &ctx.config.link, slug).map(|_| ())
        }
        // Task 11 で backup::run に差し替える
        Job::Backup { .. } => Ok(()),
    }
}

/// ノートを埋め込んで保存する。ノートがもうなければ `false`。
fn embed_note(ctx: &Ctx, kind: NoteKind, slug: &str) -> Result<bool, Error> {
    let Some(text) = note_text(&ctx.og, kind, slug)? else {
        return Ok(false);
    };
    let v = ctx.embedder.embed(&text)?;
    set_embedding(&ctx.og, kind, slug, &v)?;
    Ok(true)
}

/// 待ち行列を空になるまで処理する。
pub fn run_queue(
    queue: &Queue,
    lock_path: &Path,
    exec: &mut dyn FnMut(&Job) -> Result<(), Error>,
    now: &dyn Fn() -> SystemTime,
    sleep: &mut dyn FnMut(Duration),
) -> Result<(), Error> {
    run_queue_inner(queue, lock_path, exec, now, sleep, false)
}

/// `once` なら、ready のものを処理し終えた時点で、待ちのあるものを待たずに終わる。
fn run_queue_inner(
    queue: &Queue,
    lock_path: &Path,
    exec: &mut dyn FnMut(&Job) -> Result<(), Error>,
    now: &dyn Fn() -> SystemTime,
    sleep: &mut dyn FnMut(Duration),
    once: bool,
) -> Result<(), Error> {
    loop {
        // 1. ロックを取る。取れなければ、ほかの worker が動いている
        let Some(lock) = try_lock(lock_path)? else {
            return Ok(());
        };
        // 2, 3. 空になるまで処理する
        loop {
            if let Some(entry) = queue.next_ready(now())? {
                match exec(&entry.job) {
                    Ok(()) => queue.complete(&entry.id)?,
                    Err(Error::Transient(m)) => queue.retry_later(entry, &m, now())?,
                    // Invalid も、直すまで通らない。消さずに failed へ残す
                    Err(e) => queue.fail(entry, &e.to_string())?,
                }
                continue;
            }
            if once {
                break;
            }
            match queue.earliest_wait()? {
                Some(at) => sleep(at.duration_since(now()).unwrap_or(Duration::ZERO)),
                None => break,
            }
        }
        // 4. ロックを外してから見直す。外す前に積まれたものを取りこぼさない
        drop(lock);
        let more = if once { queue.next_ready(now())?.is_some() } else { queue.earliest_wait()?.is_some() };
        if !more {
            return Ok(());
        }
    }
}

/// `worker.lock` を取る。ほかの worker が持っていれば `None`。ロックはファイルを閉じると外れる。
fn try_lock(path: &Path) -> Result<Option<File>, Error> {
    let io = |e: std::io::Error| Error::Permanent(format!("worker.lock を開けません ({}): {e}", path.display()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io)?;
    }
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(path).map_err(io)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(io(e)),
    }
}

/// 待ち行列を処理する。失敗は worker.log に残る。
pub fn run_worker(ctx: &Ctx) -> Result<(), Error> {
    run_worker_with(ctx, false)
}

/// ready のものを 1 巡だけ処理する。待ちのあるものは待たない。
pub fn run_worker_once(ctx: &Ctx) -> Result<(), Error> {
    run_worker_with(ctx, true)
}

fn run_worker_with(ctx: &Ctx, once: bool) -> Result<(), Error> {
    let mut exec = |job: &Job| {
        let result = execute(ctx, job);
        if let Err(e) = &result {
            eprintln!("Job に失敗しました ({job:?}): {e}");
        }
        result
    };
    run_queue_inner(
        &ctx.queue,
        &ctx.paths.worker_lock(),
        &mut exec,
        &SystemTime::now,
        &mut std::thread::sleep,
        once,
    )
}

/// worker を別プロセスで起動する。待たない。`ZETTELIST_NO_SPAWN` があれば何もしない。
pub fn spawn_worker(paths: &Paths) -> Result<(), Error> {
    if std::env::var_os("ZETTELIST_NO_SPAWN").is_some() {
        return Ok(());
    }
    let fail = |what: &str, e: std::io::Error| Error::Transient(format!("{what}に失敗しました: {e}"));
    let exe = std::env::current_exe().map_err(|e| fail("実行ファイルの特定", e))?;
    let log_path = paths.worker_log();
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|e| fail("ログのディレクトリの作成", e))?;
    }
    let log = OpenOptions::new().create(true).append(true).open(&log_path).map_err(|e| fail("worker.log を開くこと", e))?;
    let log2 = log.try_clone().map_err(|e| fail("worker.log の複製", e))?;
    // 呼び出し元の端末の Ctrl-C や終了に巻き込まれないよう、別のプロセスグループにする
    Command::new(exe)
        .arg("--worker")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2))
        .spawn()
        .map_err(|e| fail("worker の起動", e))?;
    Ok(())
}

/// すべての Fleeting / Literature / Permanent に `Embed` を積み、積んだ件数を返す。
pub fn reembed_all(ctx: &Ctx) -> Result<usize, Error> {
    let mut count = 0;
    for (kind, node) in
        [(NoteKind::Fleeting, "Fleeting"), (NoteKind::Literature, "Literature"), (NoteKind::Permanent, "Permanent")]
    {
        let q = format!("query q() {{ match {{ $n: {node} }} return {{ $n.slug as slug }} }}");
        for row in ctx.og.query(BRANCH, &q, json!({}))? {
            let Some(slug) = row.get("slug").and_then(|v| v.as_str()) else { continue };
            ctx.queue.push(Job::Embed { kind, slug: slug.to_string() })?;
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::NoteKind;
    use std::cell::{Cell, RefCell};
    use tempfile::tempdir;

    fn job(slug: &str) -> Job {
        Job::Embed { kind: NoteKind::Fleeting, slug: slug.to_string() }
    }

    /// 偽の時計。`sleep` で進む。
    struct Clock {
        t: Cell<SystemTime>,
        slept: RefCell<Vec<Duration>>,
    }

    impl Clock {
        fn new() -> Clock {
            // 積んだ Job の not_before (実時間) より後ろから始める
            Clock { t: Cell::new(SystemTime::now() + Duration::from_secs(1)), slept: RefCell::new(Vec::new()) }
        }
        fn now(&self) -> SystemTime {
            self.t.get()
        }
        fn sleep(&self, d: Duration) {
            self.slept.borrow_mut().push(d);
            self.t.set(self.t.get() + d);
        }
    }

    fn run(
        q: &Queue,
        dir: &Path,
        clock: &Clock,
        exec: &mut dyn FnMut(&Job) -> Result<(), Error>,
    ) -> Result<(), Error> {
        run_queue(q, &dir.join("worker.lock"), exec, &|| clock.now(), &mut |d| clock.sleep(d))
    }

    #[test]
    fn successful_jobs_empty_the_queue() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(job("a")).unwrap();
        q.push(job("b")).unwrap();
        let clock = Clock::new();
        let mut seen = Vec::new();
        run(&q, dir.path(), &clock, &mut |j| {
            seen.push(j.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, vec![job("a"), job("b")]);
        assert!(q.earliest_wait().unwrap().is_none());
        assert!(q.failed().unwrap().is_empty());
    }

    #[test]
    fn permanent_error_goes_to_failed_without_retry() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(job("a")).unwrap();
        let clock = Clock::new();
        let mut calls = 0;
        run(&q, dir.path(), &clock, &mut |_| {
            calls += 1;
            Err(Error::Permanent("直さないと通らない".to_string()))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(q.earliest_wait().unwrap().is_none());
        let failed = q.failed().unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].last_error.as_deref(), Some("直さないと通らない"));
    }

    #[test]
    fn invalid_error_goes_to_failed_without_retry() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(job("a")).unwrap();
        let clock = Clock::new();
        let mut calls = 0;
        run(&q, dir.path(), &clock, &mut |_| {
            calls += 1;
            Err(Error::Invalid("入力の誤り".to_string()))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(q.failed().unwrap().len(), 1);
    }

    #[test]
    fn transient_error_retries_three_times_then_fails() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(job("a")).unwrap();
        let clock = Clock::new();
        let mut calls = 0;
        run(&q, dir.path(), &clock, &mut |_| {
            calls += 1;
            Err(Error::Transient("届かない".to_string()))
        })
        .unwrap();
        assert_eq!(calls, 4);
        assert_eq!(
            *clock.slept.borrow(),
            vec![Duration::from_secs(10), Duration::from_secs(60), Duration::from_secs(300)]
        );
        assert!(q.earliest_wait().unwrap().is_none());
        assert_eq!(q.failed().unwrap().len(), 1);
    }

    #[test]
    fn transient_then_success_completes() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(job("a")).unwrap();
        let clock = Clock::new();
        let mut calls = 0;
        run(&q, dir.path(), &clock, &mut |_| {
            calls += 1;
            if calls == 1 {
                Err(Error::Transient("届かない".to_string()))
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert!(q.earliest_wait().unwrap().is_none());
        assert!(q.failed().unwrap().is_empty());
    }

    #[test]
    fn locked_queue_returns_immediately() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(job("a")).unwrap();
        // ほかの worker が動いている
        let other = std::fs::File::create(dir.path().join("worker.lock")).unwrap();
        other.lock().unwrap();
        let clock = Clock::new();
        let mut calls = 0;
        run(&q, dir.path(), &clock, &mut |_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, 0);
        assert!(q.earliest_wait().unwrap().is_some());
    }

    #[test]
    fn job_pushed_during_last_job_is_run() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(job("a")).unwrap();
        let clock = Clock::new();
        let mut seen = Vec::new();
        run(&q, dir.path(), &clock, &mut |j| {
            seen.push(j.clone());
            if *j == job("a") {
                q.push(job("b")).unwrap();
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, vec![job("a"), job("b")]);
        assert!(q.earliest_wait().unwrap().is_none());
    }

    #[test]
    fn queue_failure_is_returned_instead_of_looping() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(job("a")).unwrap();
        // pending を消して、ディレクトリ読み込みを失敗させる
        std::fs::remove_dir_all(dir.path().join("queue/pending")).unwrap();
        let clock = Clock::new();
        let r = run(&q, dir.path(), &clock, &mut |_| Ok(()));
        assert!(r.is_err());
    }
}
