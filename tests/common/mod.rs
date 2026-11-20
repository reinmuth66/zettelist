//! 統合テスト用の、一時的な omnigraph-server。
#![allow(dead_code)] // テストのファイルごとに使うものが違う

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use zettelist::config::Config;
use zettelist::omnigraph::Omnigraph;

pub struct TestServer {
    _dir: tempfile::TempDir,
    child: Child,
    state: PathBuf,
    home: PathBuf,
    url: String,
}

fn run_ok(cmd: &mut Command, what: &str) -> String {
    let out = cmd.output().unwrap_or_else(|e| panic!("{what}: 起動できません: {e}"));
    assert!(
        out.status.success(),
        "{what} が失敗 ({}):\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

impl TestServer {
    pub fn start() -> TestServer {
        let dir = tempfile::tempdir().unwrap();
        // macOS の /tmp などのシンボリックリンクを解いておく (file:// の URI に使うため)。
        let root = &dir.path().canonicalize().unwrap();
        let home = root.join("home");
        let data = root.join("data");
        let config = root.join("schema");
        let state = root.join("state");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("schema"), &config);

        // 手順 1 と 2: 検証してグラフを作る。
        let omnigraph = |args: &[&str]| {
            let mut c = Command::new("omnigraph");
            c.args(args).env("OMNIGRAPH_HOME", &home).env("OLLAMA_API_KEY", "ollama");
            c
        };
        let data_s = data.to_str().unwrap();
        let config_s = config.to_str().unwrap();
        run_ok(&mut omnigraph(&["cluster", "validate", "--config", config_s]), "cluster validate");
        run_ok(
            &mut omnigraph(&["cluster", "apply", "--target", data_s, "--config", config_s, "--as", "test", "--json"]),
            "cluster apply",
        );

        // 手順 3: apply が持ったままのロックを外す。
        let status = run_ok(&mut omnigraph(&["cluster", "status", "--target", data_s, "--json"]), "cluster status");
        let status: serde_json::Value = serde_json::from_str(&status).expect("cluster status の JSON");
        if let Some(lock_id) = status.get("lock_id").and_then(|v| v.as_str()) {
            run_ok(&mut omnigraph(&["cluster", "force-unlock", lock_id, "--target", data_s]), "cluster force-unlock");
        }

        // 手順 4: サーバーを起動する。
        let port = free_port();
        let bind = format!("127.0.0.1:{port}");
        let log = std::fs::File::create(root.join("server.log")).unwrap();
        let child = Command::new("omnigraph-server")
            .args(["--cluster", &format!("file://{data_s}"), "--bind", &bind, "--unauthenticated"])
            .env("OMNIGRAPH_HOME", &home)
            .env("OLLAMA_API_KEY", "ollama")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .expect("omnigraph-server を起動できません");
        let url = format!("http://{bind}");
        let mut server = TestServer { _dir: dir, child, state, home, url };
        server.wait_healthy();
        server
    }

    fn wait_healthy(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(2)))
            .http_status_as_error(false)
            .build()
            .into();
        loop {
            if let Ok(r) = agent.get(&format!("{}/healthz", self.url)).call() {
                if r.status() == 200 {
                    return;
                }
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("omnigraph-server が先に終了しました: {status}");
            }
            if Instant::now() > deadline {
                panic!("omnigraph-server が 30 秒以内に healthz に答えません");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn config(&self) -> Config {
        let mut c = Config::default();
        c.omnigraph.server = self.url.clone();
        c
    }

    pub fn state_dir(&self) -> &Path {
        &self.state
    }

    pub fn og(&self) -> Omnigraph {
        Omnigraph::new(&self.config().omnigraph, &self.state).unwrap().with_home(&self.home)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 決まった文に決まったベクトルを返す偽の埋め込み。知らない文は Permanent エラー。
pub struct FixedEmbedder(pub std::collections::HashMap<String, Vec<f32>>);

impl zettelist::embed::Embedder for FixedEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, zettelist::error::Error> {
        self.0
            .get(text)
            .cloned()
            .ok_or_else(|| zettelist::error::Error::Permanent(format!("FixedEmbedder が知らない文です: {text}")))
    }
}

/// 1024 次元で、`(位置, 値)` の成分だけを持つ正規化したベクトル。
pub fn unit(parts: &[(usize, f32)]) -> Vec<f32> {
    let mut v = vec![0.0f32; 1024];
    for (i, x) in parts {
        v[*i] = *x;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(norm > 0.0, "ゼロベクトルは正規化できません");
    v.iter().map(|x| x / norm).collect()
}
