//! ファイルで持つ待ち行列。1 件 1 ファイル (`queue/pending/<id>.json`、`queue/failed/<id>.json`)。
//! id は `<UNIX 時刻のナノ秒 20 桁>-<pid>-<連番>` で、名前の順が積んだ順になる。

use crate::error::Error;
use crate::notes::{NewNote, NoteKind};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Job {
    WriteNote(NewNote),
    Embed { kind: NoteKind, slug: String },
    Promote { slug: String },
    Backup { message: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub job: Job,
    pub attempts: u32,
    pub not_before: SystemTime,
    pub last_error: Option<String>,
}

pub struct Queue {
    pending: PathBuf,
    failed: PathBuf,
}

/// 同じプロセスの中で、同じナノ秒に積んでも id が増え続けるようにする連番
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 失敗 `attempts` 回目のあとの待ち時間。使い切ったら `None` (failed へ回す)。
pub fn backoff(attempts: u32) -> Option<Duration> {
    match attempts {
        1 => Some(Duration::from_secs(10)),
        2 => Some(Duration::from_secs(60)),
        3 => Some(Duration::from_secs(300)),
        _ => None,
    }
}

fn io_err(what: &str, path: &Path, e: std::io::Error) -> Error {
    Error::Permanent(format!("{what}に失敗しました ({}): {e}", path.display()))
}

fn new_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{nanos:020}-{}-{seq:010}", std::process::id())
}

fn entry_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// 一時ファイルに書いてから rename する。途中で落ちても壊れたファイルを残さない。
fn write_entry(dir: &Path, entry: &Entry) -> Result<(), Error> {
    let path = entry_path(dir, &entry.id);
    let json = serde_json::to_vec_pretty(entry)
        .map_err(|e| Error::Permanent(format!("待ち行列の項目を JSON にできません: {e}")))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| io_err("一時ファイルの作成", dir, e))?;
    tmp.write_all(&json).map_err(|e| io_err("一時ファイルへの書き込み", tmp.path(), e))?;
    tmp.as_file().sync_all().map_err(|e| io_err("一時ファイルの同期", tmp.path(), e))?;
    tmp.persist(&path).map_err(|e| io_err("rename", &path, e.error))?;
    Ok(())
}

fn read_entry(path: &Path) -> Result<Entry, Error> {
    let bytes = fs::read(path).map_err(|e| io_err("読み込み", path, e))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| Error::Permanent(format!("待ち行列のファイルが壊れています ({}): {e}", path.display())))
}

/// 存在しなくてもよいファイルの削除
fn remove_file(path: &Path) -> Result<(), Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err("削除", path, e)),
    }
}

/// ディレクトリの中の項目を id の順 (積んだ順) で返す。一時ファイルなど `.json` 以外は無視する。
fn list(dir: &Path) -> Result<Vec<Entry>, Error> {
    let mut paths = Vec::new();
    for item in fs::read_dir(dir).map_err(|e| io_err("ディレクトリの読み込み", dir, e))? {
        let path = item.map_err(|e| io_err("ディレクトリの読み込み", dir, e))?.path();
        if path.extension().is_some_and(|x| x == "json") {
            paths.push(path);
        }
    }
    paths.sort();
    paths.iter().map(|p| read_entry(p)).collect()
}

impl Queue {
    pub fn open(state_dir: &Path) -> Result<Queue, Error> {
        let pending = state_dir.join("queue/pending");
        let failed = state_dir.join("queue/failed");
        for dir in [&pending, &failed] {
            fs::create_dir_all(dir).map_err(|e| io_err("ディレクトリの作成", dir, e))?;
        }
        Ok(Queue { pending, failed })
    }

    pub fn push(&self, job: Job) -> Result<String, Error> {
        let id = new_id();
        let entry = Entry { id: id.clone(), job, attempts: 0, not_before: SystemTime::now(), last_error: None };
        write_entry(&self.pending, &entry)?;
        Ok(id)
    }

    /// 積んだ順で、`not_before <= now` の最初の 1 件
    pub fn next_ready(&self, now: SystemTime) -> Result<Option<Entry>, Error> {
        Ok(list(&self.pending)?.into_iter().find(|e| e.not_before <= now))
    }

    /// pending の最小の `not_before`
    pub fn earliest_wait(&self) -> Result<Option<SystemTime>, Error> {
        Ok(list(&self.pending)?.into_iter().map(|e| e.not_before).min())
    }

    pub fn complete(&self, id: &str) -> Result<(), Error> {
        remove_file(&entry_path(&self.pending, id))
    }

    /// `attempts` を 1 増やして待たせる。`backoff` が `None` なら failed へ回す。
    pub fn retry_later(&self, mut entry: Entry, error: &str, now: SystemTime) -> Result<(), Error> {
        entry.attempts += 1;
        match backoff(entry.attempts) {
            Some(wait) => {
                entry.not_before = now + wait;
                entry.last_error = Some(error.to_string());
                write_entry(&self.pending, &entry)
            }
            None => self.fail(entry, error),
        }
    }

    /// failed へ移す。先に failed へ書いてから pending を消すので、途中で落ちても項目は失われない。
    pub fn fail(&self, mut entry: Entry, error: &str) -> Result<(), Error> {
        entry.last_error = Some(error.to_string());
        write_entry(&self.failed, &entry)?;
        remove_file(&entry_path(&self.pending, &entry.id))
    }

    pub fn failed(&self) -> Result<Vec<Entry>, Error> {
        list(&self.failed)
    }

    pub fn retry_failed(&self, id: &str) -> Result<(), Error> {
        let path = entry_path(&self.failed, id);
        if !path.exists() {
            return Err(Error::Permanent(format!("失敗の中に {id} はありません")));
        }
        self.requeue(read_entry(&path)?)
    }

    pub fn retry_all_failed(&self) -> Result<(), Error> {
        for entry in self.failed()? {
            self.requeue(entry)?;
        }
        Ok(())
    }

    pub fn discard_failed(&self, id: &str) -> Result<(), Error> {
        let path = entry_path(&self.failed, id);
        if !path.exists() {
            return Err(Error::Permanent(format!("失敗の中に {id} はありません")));
        }
        remove_file(&path)
    }

    /// failed から pending へ `attempts = 0` で戻す
    fn requeue(&self, mut entry: Entry) -> Result<(), Error> {
        entry.attempts = 0;
        entry.not_before = SystemTime::now();
        write_entry(&self.pending, &entry)?;
        remove_file(&entry_path(&self.failed, &entry.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn embed(slug: &str) -> Job {
        Job::Embed { kind: NoteKind::Fleeting, slug: slug.to_string() }
    }

    #[test]
    fn jobs_come_out_in_push_order() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        let a = q.push(embed("a")).unwrap();
        let b = q.push(embed("b")).unwrap();
        assert!(a < b);
        let now = SystemTime::now() + Duration::from_secs(1);
        let first = q.next_ready(now).unwrap().unwrap();
        assert_eq!(first.job, embed("a"));
        q.complete(&first.id).unwrap();
        let second = q.next_ready(now).unwrap().unwrap();
        assert_eq!(second.job, embed("b"));
        q.complete(&second.id).unwrap();
        assert!(q.next_ready(now).unwrap().is_none());
    }

    #[test]
    fn job_survives_reopen() {
        let dir = tempdir().unwrap();
        let job = Job::Backup { message: "テスト".to_string() };
        Queue::open(dir.path()).unwrap().push(job.clone()).unwrap();
        let q = Queue::open(dir.path()).unwrap();
        let e = q.next_ready(SystemTime::now() + Duration::from_secs(1)).unwrap().unwrap();
        assert_eq!(e.job, job);
        assert_eq!(e.attempts, 0);
    }

    #[test]
    fn backoff_is_10s_1m_5m_then_none() {
        assert_eq!(backoff(1), Some(Duration::from_secs(10)));
        assert_eq!(backoff(2), Some(Duration::from_secs(60)));
        assert_eq!(backoff(3), Some(Duration::from_secs(300)));
        assert_eq!(backoff(4), None);
    }

    #[test]
    fn retry_later_hides_job_until_not_before() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(embed("a")).unwrap();
        let now = SystemTime::now() + Duration::from_secs(1);
        let e = q.next_ready(now).unwrap().unwrap();
        q.retry_later(e, "届かない", now).unwrap();
        assert!(q.next_ready(now).unwrap().is_none());
        assert_eq!(q.earliest_wait().unwrap(), Some(now + Duration::from_secs(10)));
        let e = q.next_ready(now + Duration::from_secs(10)).unwrap().unwrap();
        assert_eq!(e.attempts, 1);
        assert_eq!(e.last_error.as_deref(), Some("届かない"));
    }

    #[test]
    fn fourth_transient_failure_goes_to_failed() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(embed("a")).unwrap();
        let mut now = SystemTime::now();
        for n in 1..=4 {
            now += Duration::from_secs(3600);
            let e = q.next_ready(now).unwrap().unwrap();
            q.retry_later(e, &format!("失敗 {n}"), now).unwrap();
        }
        assert!(q.next_ready(now + Duration::from_secs(3600)).unwrap().is_none());
        assert!(q.earliest_wait().unwrap().is_none());
        let failed = q.failed().unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].last_error.as_deref(), Some("失敗 4"));
        assert_eq!(failed[0].attempts, 4);
    }

    #[test]
    fn retry_failed_returns_to_pending_with_zero_attempts() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(embed("a")).unwrap();
        q.push(embed("b")).unwrap();
        let now = SystemTime::now() + Duration::from_secs(1);
        let a = q.next_ready(now).unwrap().unwrap();
        q.fail(a.clone(), "だめ").unwrap();
        let b = q.next_ready(now).unwrap().unwrap();
        q.fail(b, "だめ").unwrap();
        assert_eq!(q.failed().unwrap().len(), 2);

        q.retry_failed(&a.id).unwrap();
        assert_eq!(q.failed().unwrap().len(), 1);
        let e = q.next_ready(now).unwrap().unwrap();
        assert_eq!(e.id, a.id);
        assert_eq!(e.attempts, 0);

        q.retry_all_failed().unwrap();
        assert!(q.failed().unwrap().is_empty());
    }

    #[test]
    fn discard_failed_removes_it() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        q.push(embed("a")).unwrap();
        let e = q.next_ready(SystemTime::now() + Duration::from_secs(1)).unwrap().unwrap();
        q.fail(e.clone(), "だめ").unwrap();
        q.discard_failed(&e.id).unwrap();
        assert!(q.failed().unwrap().is_empty());
        assert!(q.next_ready(SystemTime::now() + Duration::from_secs(1)).unwrap().is_none());
    }
}
