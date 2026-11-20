//! omnigraph CLI を呼ぶ唯一の場所。operator 設定の server target `zl` を使い、
//! `--json` の出力を読んで Transient / Permanent に分ける。

use crate::config::OmnigraphConfig;
use crate::error::Error;
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TARGET: &str = "zl";

pub struct Omnigraph {
    operator_config: PathBuf,
    state_dir: PathBuf,
    /// `OMNIGRAPH_HOME`。テストで利用者の `~/.omnigraph` に触れないために使う。
    home: Option<PathBuf>,
}

impl Omnigraph {
    /// `state_dir/operator.yaml` に target `zl` を書く。
    pub fn new(cfg: &OmnigraphConfig, state_dir: &Path) -> Result<Omnigraph, Error> {
        std::fs::create_dir_all(state_dir).map_err(|e| Error::Permanent(format!("{} を作れません: {e}", state_dir.display())))?;
        let operator_config = state_dir.join("operator.yaml");
        // JSON の文字列は YAML としても正しい引用になる。
        let yaml = format!(
            "version: 1\ntargets:\n  {TARGET}:\n    kind: server\n    endpoint: {}\n    auth: {{provider: anonymous}}\n    default_graph: {}\n",
            Value::String(cfg.server.clone()),
            Value::String(cfg.graph.clone()),
        );
        // worker と CLI が同時に書いても途中の内容が見えないよう、置き換えで書く。
        let io = |e: std::io::Error| Error::Permanent(format!("{} を書けません: {e}", operator_config.display()));
        let mut tmp = tempfile::NamedTempFile::new_in(state_dir).map_err(io)?;
        tmp.write_all(yaml.as_bytes()).map_err(io)?;
        tmp.persist(&operator_config).map_err(|e| io(e.error))?;
        Ok(Omnigraph { operator_config, state_dir: state_dir.to_path_buf(), home: None })
    }

    /// omnigraph に渡す `OMNIGRAPH_HOME` を指定する (テスト用)。
    pub fn with_home(mut self, home: &Path) -> Omnigraph {
        self.home = Some(home.to_path_buf());
        self
    }

    /// `omnigraph <sub> --operator-config ... --target zl <args>` を実行し、成功時の標準出力を返す。
    fn run(&self, sub: &str, args: &[&str]) -> Result<String, Error> {
        let mut cmd = Command::new("omnigraph");
        cmd.arg(sub).arg("--operator-config").arg(&self.operator_config).args(["--target", TARGET]).args(args);
        if let Some(home) = &self.home {
            cmd.env("OMNIGRAPH_HOME", home);
        }
        let out = cmd.output().map_err(|e| Error::Transient(format!("omnigraph を起動できません: {e}")))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(classify(&out))
        }
    }

    fn temp_file(&self, contents: &str) -> Result<tempfile::NamedTempFile, Error> {
        let io = |e: std::io::Error| Error::Permanent(format!("一時ファイルを書けません: {e}"));
        let mut f = tempfile::NamedTempFile::new_in(&self.state_dir).map_err(io)?;
        f.write_all(contents.as_bytes()).map_err(io)?;
        f.flush().map_err(io)?;
        Ok(f)
    }

    /// JSONL の行を `--mode merge` で書き込む。
    pub fn load(&self, branch: &str, lines: &[Value]) -> Result<(), Error> {
        if lines.is_empty() {
            return Ok(());
        }
        let mut jsonl = String::new();
        for l in lines {
            jsonl.push_str(&l.to_string());
            jsonl.push('\n');
        }
        let f = self.temp_file(&jsonl)?;
        let path = f.path().to_string_lossy().into_owned();
        self.run("load", &["--data", &path, "--branch", branch, "--mode", "merge", "--json"])?;
        Ok(())
    }

    /// 読み取りクエリ。行は `rows` に入る (キーは return の式か `as` の別名、null の列は出ない)。
    pub fn query(&self, branch: &str, source: &str, params: Value) -> Result<Vec<Map<String, Value>>, Error> {
        let params = self.temp_file(&params_json(&params))?;
        let params_path = params.path().to_string_lossy().into_owned();
        let out = self.run("query", &["--branch", branch, "-e", source, "--params-file", &params_path, "--json"])?;
        rows_of(&out)
    }

    /// 書き込みのクエリ (insert / delete)。
    pub fn mutate(&self, branch: &str, source: &str, params: Value) -> Result<(), Error> {
        let params = self.temp_file(&params_json(&params))?;
        let params_path = params.path().to_string_lossy().into_owned();
        self.run("mutate", &["--branch", branch, "-e", source, "--params-file", &params_path, "--json"])?;
        Ok(())
    }

    /// `branch create ...`、`branch merge ...`、`branch delete ...` の 1 文。
    pub fn statement(&self, stmt: &str) -> Result<(), Error> {
        let mut args = vec!["-e", stmt, "--json"];
        if stmt.trim_start().starts_with("branch delete") {
            // server target では確認なしの削除を拒否されるので、明示する。
            args.push("--yes");
        }
        self.run("mutate", &args)?;
        Ok(())
    }

    pub fn branches(&self) -> Result<Vec<String>, Error> {
        let out = self.run("query", &["-e", "branch list", "--json"])?;
        Ok(rows_of(&out)?.into_iter().filter_map(|r| r.get("name").and_then(Value::as_str).map(String::from)).collect())
    }

    /// ブランチ全体を JSONL で書き出す (ノードとエッジが 1 行ずつ)。
    pub fn export(&self, branch: &str) -> Result<String, Error> {
        self.run("export", &["--branch", branch])
    }
}

fn params_json(params: &Value) -> String {
    if params.is_null() { json!({}).to_string() } else { params.to_string() }
}

fn rows_of(stdout: &str) -> Result<Vec<Map<String, Value>>, Error> {
    let v: Value = serde_json::from_str(stdout)
        .map_err(|e| Error::Permanent(format!("omnigraph の出力を読めません: {e}: {}", truncate(stdout))))?;
    let rows = v
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Permanent(format!("omnigraph の出力に rows がありません: {}", truncate(stdout))))?;
    rows.iter()
        .map(|r| {
            r.as_object()
                .cloned()
                .ok_or_else(|| Error::Permanent(format!("rows の要素がオブジェクトではありません: {r}")))
        })
        .collect()
}

fn truncate(s: &str) -> String {
    s.chars().take(500).collect()
}

/// 失敗した omnigraph の出力を Transient / Permanent に分ける。
/// 終了コードや `code` (接続失敗でも `api_contract_mismatch` になる) では見分けず、
/// `request_dispatched` と `http_status` を見る。
fn classify(out: &Output) -> Error {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if let Ok(v) = serde_json::from_str::<Value>(&stdout) {
        let message = v.get("error").and_then(Value::as_str).unwrap_or(&stdout).to_string();
        let status = v.get("http_status").and_then(Value::as_u64);
        let not_dispatched = v.get("request_dispatched").and_then(Value::as_bool) == Some(false);
        let transient = match status {
            Some(s) => s >= 500,
            None => not_dispatched,
        };
        return if transient { Error::Transient(message) } else { Error::Permanent(message) };
    }
    // `--json` のない呼び出し (export) は、標準エラーの文で見分ける。
    let text = stderr.trim().to_string();
    let message = if text.is_empty() { format!("omnigraph が失敗しました ({})", out.status) } else { text };
    let transient = message.contains("this data request was not sent")
        || message.contains("connection failed")
        || (500..600).any(|s| message.contains(&format!("HTTP {s}")));
    if transient { Error::Transient(message) } else { Error::Permanent(message) }
}
