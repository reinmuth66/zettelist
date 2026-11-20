//! エージェント向けの skill。`skill/SKILL.md` を埋め込み、`--skill-export` で書き出す。

use crate::error::Error;
use std::path::Path;

/// 埋め込んだ `skill/SKILL.md`。
pub const SKILL: &str = include_str!("../skill/SKILL.md");

/// `dir` に `SKILL.md` を書く。`dir` がなければ作る。
/// すでに `SKILL.md` があるときは、`force` がなければ書かずに入力の誤りにする。
pub fn export(dir: &Path, force: bool) -> Result<(), Error> {
    let file = dir.join("SKILL.md");
    if file.exists() && !force {
        return Err(Error::Invalid(format!(
            "{} にはすでに skill があります。上書きするなら --force",
            dir.display()
        )));
    }
    std::fs::create_dir_all(dir)
        .map_err(|e| Error::Permanent(format!("{} を作れません: {e}", dir.display())))?;
    std::fs::write(&file, SKILL)
        .map_err(|e| Error::Permanent(format!("{} に書けません: {e}", file.display())))?;
    Ok(())
}
