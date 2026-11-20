pub mod config;
pub mod embed;
pub mod error;
pub mod notes;
pub mod omnigraph;
pub mod paths;
pub mod propose;
pub mod queue;
pub mod review;
pub mod source_ref;
pub mod worker;

use config::Config;
use embed::{Embedder, OllamaEmbedder};
use error::Error;
use omnigraph::Omnigraph;
use paths::Paths;
use queue::Queue;

/// コマンドが共有する、設定・場所・omnigraph・埋め込み・待ち行列。
pub struct Ctx {
    pub config: Config,
    pub paths: Paths,
    pub og: Omnigraph,
    pub embedder: Box<dyn Embedder>,
    pub queue: Queue,
}

impl Ctx {
    /// 環境変数と設定ファイルから作る。サーバーには接続しない。
    pub fn from_env() -> Result<Ctx, Error> {
        let config = Config::load()?;
        let paths = Paths::from_env()?;
        let og = Omnigraph::new(&config.omnigraph, &paths.state)?;
        let queue = Queue::open(&paths.state)?;
        let embedder = Box::new(OllamaEmbedder::new(&config.embed));
        Ok(Ctx { config, paths, og, embedder, queue })
    }
}
