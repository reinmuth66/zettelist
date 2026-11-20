//! 設定ファイルと、手元の状態を置くディレクトリの場所。

use crate::error::Error;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    /// 設定ファイル。`ZETTELIST_CONFIG` があればそのパス、なければ `~/.config/zettelist/config.toml`
    pub config: PathBuf,
    /// 状態のディレクトリ。`ZETTELIST_STATE_DIR` があればそれ、なければ `~/.local/share/zettelist`。
    /// 下に `queue/pending/`、`queue/failed/`、`worker.lock`、`worker.log`、`operator.yaml`、`review.json` を置く
    pub state: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Paths, Error> {
        let home = || {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| Error::Permanent("ホームディレクトリが分かりません (HOME が未設定)".to_string()))
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
}
