//! 設定ファイルと、手元の状態を置くディレクトリの場所。

use crate::error::Error;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    /// 設定ファイル。`ZETTELIST_CONFIG` があればそのパス、なければ `~/.config/zettelist/config.toml`
    pub config: PathBuf,
    /// 状態のディレクトリ。`ZETTELIST_STATE_DIR` があればそれ、なければ `~/.local/share/zettelist`。
    /// 下に `queue/pending/`、`queue/failed/`、`worker.lock`、`worker.log`、`write.lock`、`operator.yaml`、`review.json` を置く
    pub state: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Paths, Error> {
        let home = || {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| Error::Permanent("cannot find the home directory (HOME is not set)".to_string()))
        };
        let config = match std::env::var_os("ZETTELIST_CONFIG") {
            Some(p) => PathBuf::from(p),
            None => home()?.join(".config/zettelist/config.toml"),
        };
        let state = match std::env::var_os("ZETTELIST_STATE_DIR") {
            Some(p) => PathBuf::from(p),
            None => home()?.join(".local/share/zettelist"),
        };
        Ok(Paths { config, state })
    }

    pub fn queue_pending(&self) -> PathBuf {
        self.state.join("queue/pending")
    }
    pub fn queue_failed(&self) -> PathBuf {
        self.state.join("queue/failed")
    }
    pub fn worker_lock(&self) -> PathBuf {
        self.state.join("worker.lock")
    }
    pub fn worker_log(&self) -> PathBuf {
        self.state.join("worker.log")
    }
    pub fn operator_yaml(&self) -> PathBuf {
        self.state.join("operator.yaml")
    }
    pub fn review_json(&self) -> PathBuf {
        self.state.join("review.json")
    }
    /// `review::apply` が review.json の隣に取るロックと同じ場所
    pub fn review_lock(&self) -> PathBuf {
        self.state.join("review.lock")
    }
    /// `notes::write_note` と `notes::promote` が slug の探索から load まで持つロック。
    /// `Omnigraph::write_lock` (`state_dir/write.lock`) と同じ場所
    pub fn write_lock(&self) -> PathBuf {
        self.state.join("write.lock")
    }
}

/// ロックファイルを開いて、排他ロックを取る (ほかが持っていれば待つ)。
/// ロックはファイルを閉じる (返した `File` を落とす) と外れる。
pub fn lock_file(path: &std::path::Path) -> Result<std::fs::File, Error> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let io = |e: std::io::Error| Error::Permanent(format!("cannot acquire {name} ({}): {e}", path.display()));
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).map_err(io)?;
    file.lock().map_err(io)?;
    Ok(file)
}
