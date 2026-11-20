//! 本文のベクトル化。Ollama の `POST /v1/embeddings` を呼ぶ。

use crate::config::EmbedConfig;
use crate::error::Error;
use serde_json::{json, Value};
use std::time::Duration;

/// スキーマの `Vector(1024)` に合わせた次元。
const DIMENSION: usize = 1024;

pub trait Embedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error>;
}

/// ローカルの Ollama で埋め込みを作る。
pub struct OllamaEmbedder {
    base: String,
    model: String,
}

impl OllamaEmbedder {
    pub fn new(cfg: &EmbedConfig) -> OllamaEmbedder {
        OllamaEmbedder { base: cfg.ollama.trim_end_matches('/').to_string(), model: cfg.model.clone() }
    }
}

impl Embedder for OllamaEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            // HTTP_PROXY / ALL_PROXY を読まない。ローカルの Ollama への通信をプロキシに回さない
            .proxy(None)
            .build()
            .into();
        let url = format!("{}/v1/embeddings", self.base);
        let mut resp = agent
            .post(&url)
            .send_json(json!({ "model": self.model, "input": text }))
            .map_err(|e| Error::Transient(format!("Ollama ({url}) に届きません: {e}")))?;
        let status = resp.status().as_u16();
        if status >= 500 {
            return Err(Error::Transient(format!("Ollama が {status} を返しました")));
        }
        if status >= 400 {
            let detail = resp.body_mut().read_to_string().unwrap_or_default();
            let message = format!("Ollama が {status} を返しました: {detail}");
            // 408 (タイムアウト) と 429 (制限) は待てば通る。モデル未取得の 404 などは直すまで通らない。
            return Err(if status == 408 || status == 429 { Error::Transient(message) } else { Error::Permanent(message) });
        }
        let body: Value = resp.body_mut().read_json().map_err(|e| match e {
            // 中身が JSON でないのは Ollama 側の仕様のずれ。読み込みの途中で切れた、時間切れなどは再試行で通りうる。
            ureq::Error::Json(_) => Error::Permanent(format!("Ollama の応答を読めません: {e}")),
            _ => Error::Transient(format!("Ollama の応答を読めません: {e}")),
        })?;
        let vector = body
            .pointer("/data/0/embedding")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Permanent(format!("Ollama の応答に embedding がありません: {body}")))?
            .iter()
            .map(|x| x.as_f64().map(|f| f as f32))
            .collect::<Option<Vec<f32>>>()
            .ok_or_else(|| Error::Permanent("Ollama の embedding に数値でない要素があります".to_string()))?;
        if vector.len() != DIMENSION {
            return Err(Error::Permanent(format!(
                "埋め込みの次元が {DIMENSION} ではありません ({})。embed.model を確かめてください",
                vector.len()
            )));
        }
        Ok(vector)
    }
}

/// `nearest` の距離 (正規化したベクトルの 2 乗ユークリッド距離、0 から 4) をコサイン類似度にする。
pub fn similarity(distance: f32) -> f32 {
    1.0 - distance / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EmbedConfig;
    use crate::error::Error;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// 偽の Ollama。受けた要求の 1 行目と本文は `finish` で取り出す。
    struct Fake {
        base: String,
        handle: std::thread::JoinHandle<()>,
        rx: std::sync::mpsc::Receiver<(String, String)>,
    }

    impl Fake {
        /// スレッドの終了を待つ (中で panic していればここで落ちる)。受けた要求の 1 行目と本文を返す。
        fn finish(self) -> (String, String) {
            self.handle.join().expect("偽 Ollama のスレッドが panic しました");
            self.rx.recv().expect("要求が届いていません")
        }
    }

    /// 1 回だけ、`response` をそのまま返す偽の Ollama を立てる。
    fn fake_raw(response: String) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            // ヘッダーと Content-Length 分の本文を読み切ってから返す
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let (head, body) = loop {
                let n = s.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(pos) = text.find("\r\n\r\n") {
                    let len = text[..pos]
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                        .unwrap_or(0);
                    if buf.len() >= pos + 4 + len {
                        break (text[..pos].to_string(), String::from_utf8_lossy(&buf[pos + 4..pos + 4 + len]).to_string());
                    }
                }
                assert!(n > 0, "要求を読み切る前に切れました");
            };
            tx.send((head.lines().next().unwrap_or_default().to_string(), body)).unwrap();
            s.write_all(response.as_bytes()).unwrap();
        });
        Fake { base, handle, rx }
    }

    fn fake_ollama(status: &str, body: String) -> Fake {
        fake_raw(format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ))
    }

    fn embedder(base: &str) -> OllamaEmbedder {
        OllamaEmbedder::new(&EmbedConfig { ollama: base.to_string(), model: "m".to_string() })
    }

    fn body_with(n: usize) -> String {
        let v = vec![0.0f32; n];
        serde_json::json!({ "data": [{ "embedding": v }] }).to_string()
    }

    #[test]
    fn ollama_returns_vector() {
        let fake = fake_ollama("200 OK", body_with(1024));
        let v = embedder(&fake.base).embed("こんにちは").unwrap();
        assert_eq!(v.len(), 1024);
        let (line, body) = fake.finish();
        assert!(line.starts_with("POST /v1/embeddings"), "{line}");
        let sent: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(sent, json!({ "model": "m", "input": "こんにちは" }));
    }

    #[test]
    fn wrong_dimension_is_permanent() {
        let fake = fake_ollama("200 OK", body_with(3));
        let e = embedder(&fake.base).embed("x").unwrap_err();
        assert!(matches!(e, Error::Permanent(_)), "{e:?}");
        assert!(e.to_string().contains("1024"), "{e}");
        assert_eq!(e.to_string(), "埋め込みの次元が 1024 ではありません (3)。embed.model を確かめてください");
        fake.finish();
    }

    #[test]
    fn unreachable_ollama_is_transient() {
        let e = embedder("http://127.0.0.1:1").embed("x").unwrap_err();
        assert!(e.is_transient(), "{e:?}");
    }

    #[test]
    fn server_error_is_transient() {
        let fake = fake_ollama("503 Service Unavailable", "{}".to_string());
        let e = embedder(&fake.base).embed("x").unwrap_err();
        assert!(e.is_transient(), "{e:?}");
        fake.finish();
    }

    #[test]
    fn rate_limit_and_request_timeout_are_transient() {
        for status in ["429 Too Many Requests", "408 Request Timeout"] {
            let fake = fake_ollama(status, "{}".to_string());
            let e = embedder(&fake.base).embed("x").unwrap_err();
            assert!(e.is_transient(), "{status}: {e:?}");
            fake.finish();
        }
    }

    #[test]
    fn client_error_is_permanent() {
        let fake = fake_ollama("404 Not Found", "{\"error\":\"model not found\"}".to_string());
        let e = embedder(&fake.base).embed("x").unwrap_err();
        assert!(matches!(e, Error::Permanent(_)), "{e:?}");
        fake.finish();
    }

    #[test]
    fn non_json_body_is_permanent() {
        let fake = fake_ollama("200 OK", "これは JSON ではありません".to_string());
        let e = embedder(&fake.base).embed("x").unwrap_err();
        assert!(matches!(e, Error::Permanent(_)), "{e:?}");
        fake.finish();
    }

    #[test]
    fn body_cut_off_midway_is_transient() {
        // Content-Length より短い本文で接続を閉じる
        let fake = fake_raw(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 5000\r\nConnection: close\r\n\r\n{\"data\":".to_string(),
        );
        let e = embedder(&fake.base).embed("x").unwrap_err();
        assert!(e.is_transient(), "{e:?}");
        fake.finish();
    }

    #[test]
    fn similarity_from_distance() {
        assert_eq!(similarity(0.0), 1.0);
        assert_eq!(similarity(2.0), 0.0);
        assert_eq!(similarity(4.0), -1.0);
    }

    #[test]
    fn embedder_is_object_safe() {
        let b: Box<dyn Embedder> = Box::new(embedder("http://127.0.0.1:1"));
        let r: &dyn Embedder = b.as_ref();
        assert!(r.embed("x").is_err());
    }
}
