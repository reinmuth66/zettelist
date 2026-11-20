//! main ブランチを JSONL に書き出して、データのリポジトリへ commit する。

use crate::error::Error;
use crate::Ctx;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const BRANCH: &str = "main";

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// commit を作った
    Committed,
    /// 前回の書き出しと同じだった
    NoChanges,
    /// `backup.repo` が設定されていない
    NotConfigured,
}

/// `repo` の中の `file_name` に `contents` を書き、そのファイルだけを commit する。
/// 変更がなければ `NoChanges`。`push` なら続けて `git push` する。
/// push の失敗は Transient (commit は残る)、ほかの git の失敗は Permanent。
pub fn commit_export(
    repo: &Path,
    file_name: &str,
    contents: &str,
    message: &str,
    push: bool,
) -> Result<Outcome, Error> {
    if !repo.is_dir() {
        return Err(Error::Permanent(format!("backup.repo のディレクトリがありません: {}", repo.display())));
    }
    // 通常のリポジトリは `.git` ディレクトリ、worktree やサブモジュールは `.git` ファイル
    if !repo.join(".git").exists() {
        return Err(Error::Permanent(format!("backup.repo が git リポジトリではありません: {}", repo.display())));
    }
    fs::write(repo.join(file_name), contents)
        .map_err(|e| Error::Permanent(format!("{file_name} を書けません ({}): {e}", repo.display())))?;

    git_ok(repo, &["add", "--", file_name])?;
    // 0 なら、書き出したファイルに commit するほどの差がない
    let outcome = if git(repo, &["diff", "--cached", "--quiet", "--", file_name])?.status.success() {
        Outcome::NoChanges
    } else {
        // 書き出したファイルだけを commit する (ほかに stage 済みのものを巻き込まない)
        git_ok(repo, &["commit", "-m", message, "--", file_name])?;
        Outcome::Committed
    };

    // 変更がなくても push する。前回 push に失敗した commit が残っていれば、再試行でここから届く
    // (送るものがなければ push は何もせず成功する)
    if push {
        let out = git(repo, &["push"])?;
        if !out.status.success() {
            return Err(Error::Transient(format!("git push に失敗しました: {}", describe(&out))));
        }
    }
    Ok(outcome)
}

/// main を `<graph>.jsonl` に書き出して commit する。`backup.repo` がなければ何もしない。
pub fn run(ctx: &Ctx, message: &str) -> Result<Outcome, Error> {
    let Some(repo) = &ctx.config.backup.repo else {
        return Ok(Outcome::NotConfigured);
    };
    let contents = ctx.og.export(BRANCH)?;
    let file_name = format!("{}.jsonl", ctx.config.omnigraph.graph);
    commit_export(repo, &file_name, &contents, message, ctx.config.backup.push)
}

fn git(repo: &Path, args: &[&str]) -> Result<Output, Error> {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| Error::Permanent(format!("git を起動できません: {e}")))
}

/// 成功を求める git。失敗したら標準エラーを添えて Permanent にする。
fn git_ok(repo: &Path, args: &[&str]) -> Result<Output, Error> {
    let out = git(repo, args)?;
    if out.status.success() {
        Ok(out)
    } else {
        Err(Error::Permanent(format!("git {} に失敗しました: {}", args[0], describe(&out))))
    }
}

fn describe(out: &Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let text = stderr.trim();
    if text.is_empty() {
        format!("{}", out.status)
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{tempdir, TempDir};

    fn git_in(repo: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }

    /// 識別情報はこのリポジトリの設定に書く。利用者の global 設定には頼らない。
    fn init_repo() -> TempDir {
        let dir = tempdir().unwrap();
        git_in(dir.path(), &["init", "-q"]);
        git_in(dir.path(), &["config", "user.name", "テスト"]);
        git_in(dir.path(), &["config", "user.email", "test@example.com"]);
        git_in(dir.path(), &["config", "commit.gpgsign", "false"]);
        dir
    }

    #[test]
    fn first_export_is_committed() {
        let repo = init_repo();
        let r = commit_export(repo.path(), "g.jsonl", "{\"a\":1}\n", "merge: links 3", false).unwrap();
        assert_eq!(r, Outcome::Committed);
        assert_eq!(git_in(repo.path(), &["log", "-1", "--format=%s"]).trim(), "merge: links 3");
        assert_eq!(fs::read_to_string(repo.path().join("g.jsonl")).unwrap(), "{\"a\":1}\n");
    }

    #[test]
    fn same_contents_gives_no_changes() {
        let repo = init_repo();
        commit_export(repo.path(), "g.jsonl", "x\n", "first", false).unwrap();
        let r = commit_export(repo.path(), "g.jsonl", "x\n", "second", false).unwrap();
        assert_eq!(r, Outcome::NoChanges);
        assert_eq!(git_in(repo.path(), &["rev-list", "--count", "HEAD"]).trim(), "1");
    }

    #[test]
    fn changed_contents_make_a_second_commit() {
        let repo = init_repo();
        commit_export(repo.path(), "g.jsonl", "x\n", "first", false).unwrap();
        let r = commit_export(repo.path(), "g.jsonl", "y\n", "second", false).unwrap();
        assert_eq!(r, Outcome::Committed);
        assert_eq!(git_in(repo.path(), &["rev-list", "--count", "HEAD"]).trim(), "2");
    }

    #[test]
    fn push_failure_is_transient() {
        let repo = init_repo();
        let err = commit_export(repo.path(), "g.jsonl", "x\n", "msg", true).unwrap_err();
        assert!(matches!(err, Error::Transient(_)), "{err:?}");
        // commit は残る
        assert_eq!(git_in(repo.path(), &["log", "-1", "--format=%s"]).trim(), "msg");
    }

    #[test]
    fn retry_pushes_commit_left_by_failed_push() {
        let repo = init_repo();
        // remote がないので push は失敗し、commit だけが残る
        let err = commit_export(repo.path(), "g.jsonl", "x\n", "msg", true).unwrap_err();
        assert!(matches!(err, Error::Transient(_)), "{err:?}");
        assert_eq!(git_in(repo.path(), &["rev-list", "--count", "HEAD"]).trim(), "1");

        // あとから remote と upstream を用意する (push はしない)
        let remote = tempdir().unwrap();
        git_in(remote.path(), &["init", "-q", "--bare"]);
        let remote_path = remote.path().to_str().unwrap();
        let branch = git_in(repo.path(), &["branch", "--show-current"]).trim().to_string();
        git_in(repo.path(), &["remote", "add", "origin", remote_path]);
        git_in(repo.path(), &["config", &format!("branch.{branch}.remote"), "origin"]);
        git_in(repo.path(), &["config", &format!("branch.{branch}.merge"), &format!("refs/heads/{branch}")]);

        // 同じ中身の再試行は commit するものがなくても、残っている commit を push する
        let r = commit_export(repo.path(), "g.jsonl", "x\n", "msg", true).unwrap();
        assert_eq!(r, Outcome::NoChanges);
        assert_eq!(git_in(repo.path(), &["rev-list", "--count", "HEAD"]).trim(), "1");
        assert_eq!(
            git_in(remote.path(), &["rev-parse", "HEAD"]).trim(),
            git_in(repo.path(), &["rev-parse", "HEAD"]).trim()
        );
    }

    #[test]
    fn commit_message_has_no_attribution() {
        let repo = init_repo();
        commit_export(repo.path(), "g.jsonl", "x\n", "merge: links 1", false).unwrap();
        let body = git_in(repo.path(), &["log", "-1", "--format=%B"]);
        assert!(!body.contains("Co-Authored-By"), "{body}");
        assert_eq!(body.trim(), "merge: links 1");
    }

    #[test]
    fn other_staged_files_are_not_committed() {
        let repo = init_repo();
        fs::write(repo.path().join("other.txt"), "keep\n").unwrap();
        git_in(repo.path(), &["add", "other.txt"]);
        commit_export(repo.path(), "g.jsonl", "x\n", "msg", false).unwrap();
        let files = git_in(repo.path(), &["show", "--name-only", "--format=", "HEAD"]);
        assert_eq!(files.trim(), "g.jsonl");
    }

    #[test]
    fn missing_repo_is_permanent() {
        let dir = tempdir().unwrap();
        let err = commit_export(&dir.path().join("none"), "g.jsonl", "x\n", "m", false).unwrap_err();
        assert!(matches!(err, Error::Permanent(_)), "{err:?}");
    }

    #[test]
    fn non_git_directory_is_permanent() {
        let dir = tempdir().unwrap();
        let err = commit_export(dir.path(), "g.jsonl", "x\n", "m", false).unwrap_err();
        assert!(matches!(&err, Error::Permanent(m) if m.contains("git リポジトリではありません")), "{err:?}");
        assert!(!dir.path().join("g.jsonl").exists());
    }
}
