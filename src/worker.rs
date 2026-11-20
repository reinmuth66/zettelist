//! 待ち行列を処理する worker。起動の中身は Task 9 で書く。

use crate::error::Error;
use crate::paths::Paths;

/// worker を起動する。Task 9 まで何もしない。`ZETTELIST_NO_SPAWN` があっても何も起こらない。
pub fn spawn_worker(_paths: &Paths) -> Result<(), Error> {
    Ok(())
}
