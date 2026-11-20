//! 本文のベクトル化。実装は Task 7 で書く。

use crate::error::Error;

pub trait Embedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error>;
}

/// Task 7 で `OllamaEmbedder` に置き換える仮の実装。
pub struct UnimplementedEmbedder;

impl Embedder for UnimplementedEmbedder {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, Error> {
        Err(Error::Transient("埋め込みは未実装です".to_string()))
    }
}
