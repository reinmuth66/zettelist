//! ファイルで持つ待ち行列。1 件 1 ファイル (`queue/pending/<id>.json`、`queue/failed/<id>.json`)。
//! id は `<UNIX 時刻のナノ秒 20 桁>-<pid>` で、名前の順が積んだ順になる。
//! 読めないファイルは `queue/corrupt/` に移して、残りの処理を続ける。

use crate::error::Error;
use crate::notes::{NewNote, NoteKind};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Job {
    WriteNote(NewNote),
    Embed { kind: NoteKind, slug: String },
    Promote { slug: String },
    Backup { message: String },
}

impl Job {
    /// JSON と一覧に出す、ジョブの種類の名前。
    pub fn name(&self) -> &'static str {
        match self {
            Job::WriteNote(_) => "write_note",
            Job::Embed { .. } => "embed",
            Job::Promote { .. } => "promote",
            Job::Backup { .. } => "backup",
        }
    }
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
    corrupt: PathBuf,
}

/// このプロセスで最後に使った id のナノ秒。同じナノ秒でも時計が戻っても id が増え続けるようにする
static LAST_NANOS: Mutex<u128> = Mutex::new(0);

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

/// 指定された id が failed に無い。指定の誤りなので入力の誤りとして返す。
fn missing_failed(id: &str) -> Error {
    Error::Invalid(format!("失敗の中に {id} はありません"))
}

/// 時計が止まっても戻っても、前に使った値より必ず大きくする
fn next_nanos(now: u128, last: u128) -> u128 {
    now.max(last + 1)
}

fn new_id() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut last = LAST_NANOS.lock().unwrap_or_else(|p| p.into_inner());
    let nanos = next_nanos(now, *last);
    *last = nanos;
    format!("{nanos:020}-{}", std::process::id())
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

/// 項目を読む。読む前に消えていたら `None` (別のプロセスが完了させた)。
/// JSON として読めない、または知らない Job の種類なら `corrupt` へ移して `None`。
fn read_entry(path: &Path, corrupt: &Path) -> Result<Option<Entry>, Error> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err("読み込み", path, e)),
    };
    match serde_json::from_slice(&bytes) {
        Ok(entry) => Ok(Some(entry)),
        Err(_) => {
            let name = path.file_name().unwrap_or_default();
            match fs::rename(path, corrupt.join(name)) {
                Ok(()) => Ok(None),
                // 移す前に別のプロセスが片付けた
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
                Err(e) => Err(io_err("壊れたファイルの退避", path, e)),
            }
        }
    }
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
/// 消えたファイルは飛ばし、壊れたファイルは `corrupt` へ移す。
fn list(dir: &Path, corrupt: &Path) -> Result<Vec<Entry>, Error> {
    let mut paths = Vec::new();
    for item in fs::read_dir(dir).map_err(|e| io_err("ディレクトリの読み込み", dir, e))? {
        let path = item.map_err(|e| io_err("ディレクトリの読み込み", dir, e))?.path();
        if path.extension().is_some_and(|x| x == "json") {
            paths.push(path);
        }
    }
    paths.sort();
    let mut entries = Vec::new();
    for path in &paths {
        if let Some(entry) = read_entry(path, corrupt)? {
            entries.push(entry);
        }
    }
    Ok(entries)
}

impl Queue {
    pub fn open(state_dir: &Path) -> Result<Queue, Error> {
        let pending = state_dir.join("queue/pending");
        let failed = state_dir.join("queue/failed");
        let corrupt = state_dir.join("queue/corrupt");
        for dir in [&pending, &failed, &corrupt] {
            fs::create_dir_all(dir).map_err(|e| io_err("ディレクトリの作成", dir, e))?;
        }
        Ok(Queue { pending, failed, corrupt })
    }

    pub fn push(&self, job: Job) -> Result<String, Error> {
        let id = new_id();
        let entry = Entry { id: id.clone(), job, attempts: 0, not_before: SystemTime::now(), last_error: None };
        write_entry(&self.pending, &entry)?;
        Ok(id)
    }

    /// 積んだ順で、`not_before <= now` の最初の 1 件
    pub fn next_ready(&self, now: SystemTime) -> Result<Option<Entry>, Error> {
        Ok(list(&self.pending, &self.corrupt)?.into_iter().find(|e| e.not_before <= now))
    }

    /// pending の最小の `not_before`
    pub fn earliest_wait(&self) -> Result<Option<SystemTime>, Error> {
        Ok(list(&self.pending, &self.corrupt)?.into_iter().map(|e| e.not_before).min())
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
        list(&self.failed, &self.corrupt)
    }

    /// failed の項目のパス。id が単なるファイル名でなければ、探しても無いものとして入力の誤りにする。
    fn failed_path(&self, id: &str) -> Result<PathBuf, Error> {
        let plain = !id.is_empty() && !id.starts_with('.') && !id.contains(['/', '\\']);
        if plain {
            Ok(entry_path(&self.failed, id))
        } else {
            Err(missing_failed(id))
        }
    }

    pub fn retry_failed(&self, id: &str) -> Result<(), Error> {
        match read_entry(&self.failed_path(id)?, &self.corrupt)? {
            Some(entry) => self.requeue(entry),
            None => Err(missing_failed(id)),
        }
    }

    pub fn retry_all_failed(&self) -> Result<(), Error> {
        for entry in self.failed()? {
            self.requeue(entry)?;
        }
        Ok(())
    }

    pub fn discard_failed(&self, id: &str) -> Result<(), Error> {
        let path = self.failed_path(id)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Err(missing_failed(id)),
            Err(e) => Err(io_err("削除", &path, e)),
        }
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

    #[test]
    fn next_nanos_is_strictly_increasing_even_if_clock_goes_back() {
        // 時計が進んでいれば now をそのまま使う
        assert_eq!(next_nanos(100, 50), 100);
        // 同じ値なら 1 進める
        assert_eq!(next_nanos(100, 100), 101);
        // 時計が戻っても last より後ろになる
        assert_eq!(next_nanos(10, 100), 101);
        // 初回 (last = 0)
        assert_eq!(next_nanos(0, 0), 1);
    }

    #[test]
    fn ids_are_strictly_increasing_in_one_process() {
        let ids: Vec<String> = (0..1000).map(|_| new_id()).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn read_entry_of_vanished_file_is_none() {
        let dir = tempdir().unwrap();
        let corrupt = dir.path().join("corrupt");
        fs::create_dir_all(&corrupt).unwrap();
        // 一覧に載ったあと、別のプロセスが消したファイル
        let gone = dir.path().join("00000000000000000001-1.json");
        assert!(read_entry(&gone, &corrupt).unwrap().is_none());
    }

    #[test]
    fn retry_and_discard_of_missing_id_are_errors() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        assert!(matches!(q.retry_failed("nothing"), Err(Error::Invalid(m)) if m.contains("ありません")));
        assert!(matches!(q.discard_failed("nothing"), Err(Error::Invalid(m)) if m.contains("ありません")));
    }

    #[test]
    fn path_like_ids_are_invalid_and_touch_nothing() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        let id = q.push(embed("a")).unwrap();
        // pending にある項目を、failed からの相対で指しても触れない
        let sneaky = format!("../pending/{id}");
        assert!(matches!(q.retry_failed(&sneaky), Err(Error::Invalid(_))));
        assert!(matches!(q.discard_failed(&sneaky), Err(Error::Invalid(_))));
        assert!(matches!(q.discard_failed(""), Err(Error::Invalid(_))));
        assert!(q.next_ready(SystemTime::now() + Duration::from_secs(1)).unwrap().is_some());
    }

    #[test]
    fn corrupt_file_is_moved_aside_and_queue_keeps_working() {
        let dir = tempdir().unwrap();
        let q = Queue::open(dir.path()).unwrap();
        let pending = dir.path().join("queue/pending");
        // 有効な項目より前の名前に壊れたファイルを置く
        fs::write(pending.join("00000000000000000000-1-bad.json"), "これは JSON ではない").unwrap();
        // 知らない Job の種類も壊れたものとして扱う
        fs::write(
            pending.join("00000000000000000000-2-unknown.json"),
            r#"{"id":"x","job":{"Nope":{}},"attempts":0,"not_before":{"secs_since_epoch":0,"nanos_since_epoch":0},"last_error":null}"#,
        )
        .unwrap();
        q.push(embed("ok")).unwrap();

        let e = q.next_ready(SystemTime::now() + Duration::from_secs(1)).unwrap().unwrap();
        assert_eq!(e.job, embed("ok"));
        assert!(!pending.join("00000000000000000000-1-bad.json").exists());
        assert!(!pending.join("00000000000000000000-2-unknown.json").exists());
        let corrupt = dir.path().join("queue/corrupt");
        assert!(corrupt.join("00000000000000000000-1-bad.json").exists());
        assert!(corrupt.join("00000000000000000000-2-unknown.json").exists());
        assert!(q.earliest_wait().unwrap().is_some());
        assert!(q.failed().unwrap().is_empty());
    }
}
