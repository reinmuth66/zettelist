# zettelist 本体 (TUI を除く) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `zl "<本文>"` での書き込み、待ち行列と worker、埋め込み、リンクと Hub の提案、レビューの適用、バックアップを、TUI なしで動く形で作る。

**Architecture:** ライブラリ crate `zettelist` に処理を置き、`zl` バイナリは引数を振り分けるだけにする。Omnigraph には CLI (`omnigraph --operator-config <file> --target zl`) をサブプロセスで呼び、呼び出しは `src/omnigraph.rs` の 1 か所にまとめる。TUI (inbox、昇格、レビューの画面、失敗の一覧、`[theme]`、`[keys]`) は計画 3、出典の登録 (外部 API と辞書) は計画 4 で、この計画が作る関数を呼ぶ。

**Tech Stack:** Rust 1.99 (devShell)、clap、serde / serde_json、toml、chrono、ureq 3、omnigraph 0.15.0 / omnigraph-server、Ollama (`qwen3-embedding:0.6b`)

**Spec:** `docs/superpowers/specs/2026-11-19-zettelist-design.md` (確認の記録: `docs/superpowers/notes/2026-11-19-omnigraph-check.md`)

## Global Constraints

- コマンド名は `zl`、パッケージ名は `zettelist`
- 設定ファイルは `~/.config/zettelist/config.toml`。ファイルがなくても既定値で動く。環境変数 `ZETTELIST_CONFIG` があればそのパスを読む (テスト用)
- 手元の状態は `~/.local/share/zettelist/` に置く。環境変数 `ZETTELIST_STATE_DIR` があればそこを使う (テスト用)
- review は常にある 1 本のブランチで、名前は `review`。提案を書く前に毎回 `branch merge main into review` で main を取り込む
- 近さは `similarity = 1 - distance / 2` (`nearest` の距離は 2 乗ユークリッド距離で 0 から 4)。`min_similarity`、`inbox_similarity`、Links の `similarity` はこの値
- 埋め込みは zl が Ollama の `POST {embed.ollama}/v1/embeddings` で作る (設計書の方法 a)。次元は 1024。検索でも zl がベクトルを渡す (`$v: Vector(1024)`)
- 書き込みは Ollama を待たない。埋め込みは待ち行列に積み、worker が後から書き足す
- 一時的な失敗は 10 秒、1 分、5 分の間を空けて 3 回やり直す。何度やっても同じ失敗はやり直さずに「失敗」に回す
- メモは裏の処理の成否に関係なく必ず保存する
- LLM は使わない。Python と Python ライブラリは使わない。ファイル編集に `sed -i` を使わない
- commit メッセージに Claude の関与 (Co-Authored-By など) を書かない。バックアップの commit も同じ
- 文書とメッセージは日本語。絵文字を使わない
- テストは `nix develop -c cargo test` で動かす。結合テストは devShell の `omnigraph` / `omnigraph-server` で一時的なサーバーを立てる。Ollama は使わない (テストでは固定のベクトルを返す `Embedder` を使う)

## Review Focus

1. 同じ秒に `zl` を 2 回実行する → 2 件目は別の slug (1 秒後の時刻) になり、1 件目を上書きしない (`@key` の insert は upsert なので、確かめないと黙って消える)
2. omnigraph-server が止まっているときに `zl "<本文>"` → メモを待ち行列に保存し、そう表示して終了コード 0 で終わる。worker が後で書き込む
3. worker が終わる直前に新しい処理が積まれる → 取りこぼさない (ロックを外したあとで待ち行列をもう一度見る)
4. 通信が切れて成功が見えないまま、worker が同じ提案をもう一度書く → エッジが重複しない (`Links` と `Includes` に `@key(@src, @dst)`)
5. `m` (レビューの適用) の途中で落ちる → 未判断の提案が消えない (再び review に入れる分を、消す前に手元に書いておく)

---

## File Structure

| ファイル | 役割 |
|---|---|
| `schema/schema.pg`, `schema/cluster.yaml` | スキーマと cluster 設定の正本。テストのサーバーも、運用のサーバーもこれを使う |
| `src/lib.rs` | モジュールの宣言と `Ctx` (設定、パス、Omnigraph、Embedder、Queue をまとめたもの) |
| `src/main.rs` | 引数の振り分けだけ |
| `src/paths.rs` | 設定ファイルと状態のディレクトリの場所 |
| `src/error.rs` | `Error::{Transient, Permanent}`。worker のやり直しの判断に使う |
| `src/omnigraph.rs` | omnigraph CLI の呼び出し。ほかのファイルはサブプロセスを直接呼ばない |
| `src/source_ref.rs` | `-s` の値を ID (`isbn:` / `doi:` / `arxiv:` / `url:`) か slug に分ける |
| `src/notes.rs` | ノートの slug、書き込み、inbox、昇格、削除、埋め込みの読み書き、近いノートの検索 |
| `src/embed.rs` | `Embedder` と Ollama の実装 |
| `src/queue.rs` | 待ち行列 (ファイル)、失敗の一覧、やり直しの間隔 |
| `src/worker.rs` | 処理の実行、やり直し、worker の起動 |
| `src/propose.rs` | リンクの候補と Hub の振り分けを review に書く |
| `src/review.rs` | 提案の一覧、採否の記録、`m` での適用 |
| `src/backup.rs` | export と git commit / push |
| `tests/common/mod.rs` | 一時的な omnigraph-server と、固定のベクトルを返す Embedder |

---

### Task 1: 計画 2 の最初の確認

設計書の「まだ確かめていないこと」と、この計画が前提にすることを確かめる。使い捨ての確認なので TDD は行わない。

**Files:**
- Create: `spike/check2.sh`
- Modify: `spike/cluster/schema.pg` (`Links` と `Includes` に `@key(@src, @dst)`)、`docs/superpowers/notes/2026-11-19-omnigraph-check.md` (「計画 2 の確認」の節を足す)、`docs/superpowers/specs/2026-11-19-zettelist-design.md` (「まだ確かめていないこと」を結果に置き換える)

**Interfaces:**
- Produces: 確認の記録。Task 2 以降はこれを前提にする。前提と違う結果が出たら、直す前に止めて利用者に聞く

- [ ] **Step 1: スキーマに `@key` を足す**

`edge Links` と `edge Includes` の本体の先頭に `@key(@src, @dst)` を書く。`spike/data` と `spike/.home` を消してから、確認の記録の手順 1 から 5 でグラフを作り直し、サーバーを起動する。

- [ ] **Step 2: `spike/check2.sh` で次の 6 項目を確かめる**

`spike/check.sh` と同じ書き方 (`$OG` と `say`) で書く。

1. 2 回目の往復: review を main にマージしたあと、main に Permanent を足し、`branch merge main into review` してから、その Permanent へのリンクを review に書けるか
2. 却下: review にリンクを 2 本書き、1 本を `delete Links where @src = $a and @dst = $b` (`mutate --branch review`) で消してから review を main にマージすると、main に 1 本だけ入るか。`where` で `@src` と `@dst` を `and` でつなげるか
3. 重複: 同じ組のリンクを `load --mode merge` で 2 回書いても 1 本か。2 回目に `reason` を付けると、1 本のまま `reason` が入るか
4. 削除: Cites を持つ Literature を `delete Literature where slug = $s` で消すと、Cites はどうなるか (一緒に消えるか、拒否されるか、残るか)。DerivedFrom のない Fleeting は消せるか
5. 停止: サーバーを止めた状態で `load` と `query` を実行したときの終了コードと出力。`@card` 違反 (確認 1) の出力と見分けられるか
6. 出力の形: `query --json` の行がどのキーに入るか (`rows` など)。`query -e 'branch list' --json` の出力の形

Run: `nix develop -c sh spike/check2.sh > spike/check2.out 2>&1`
Expected: 6 項目それぞれに、できた / できないの根拠になる出力が出る

- [ ] **Step 3: 結果を記録し、設計書を直す**

確認の記録に「計画 2 の確認 (日付)」の節を足し、6 項目の結果を書く。設計書の「まだ確かめていないこと」の 3 点を結果に置き換え、`Links` の `@key` を決めた形で書く。項目 4 で Literature を消すと Cites が残る場合は、削除は「Cites を消してから Literature を消す」(2 回の mutate) とし、Task 6 の `delete_notes` にそう書き足す。項目 1 から 3 のどれかができない場合は、ここで止めて利用者に聞く (review の作りが変わるため)。

- [ ] **Step 4: Commit**

```bash
git add spike docs/superpowers/notes docs/superpowers/specs/2026-11-19-zettelist-design.md
git commit -m "docs: Record plan 2 omnigraph checks"
```

### Task 2: ライブラリ化、スキーマ、Omnigraph の呼び出し、テストのサーバー

**Files:**
- Create: `src/lib.rs`, `src/paths.rs`, `src/error.rs`, `src/omnigraph.rs`, `schema/schema.pg`, `schema/cluster.yaml`, `tests/common/mod.rs`, `tests/omnigraph.rs`
- Modify: `src/main.rs` (`mod config` をやめて `zettelist::config` を使う)、`src/config.rs` (`load` の `allow(dead_code)` を外し、`ZETTELIST_CONFIG` を読む)、`Cargo.toml`、`flake.nix` (`packages.default` に `cargoTestFlags = [ "--lib" ]`。nix のビルドの中にはサーバーがないため)

**Interfaces:**
- Consumes: `config::{Config, OmnigraphConfig}` (計画 1)
- Produces:
  - `pub struct Paths { pub config: PathBuf, pub state: PathBuf }`、`pub fn Paths::from_env() -> Result<Paths, Error>`。`state` の下に `queue/pending/`、`queue/failed/`、`worker.lock`、`worker.log`、`operator.yaml`、`review.json` を置く
  - `pub enum Error { Transient(String), Permanent(String) }` (`Display` と `std::error::Error`)。`pub fn Error::is_transient(&self) -> bool`
  - `pub struct Omnigraph`。`pub fn Omnigraph::new(cfg: &OmnigraphConfig, state_dir: &Path) -> Result<Omnigraph, Error>` は `state_dir/operator.yaml` に target `zl` (`kind: server`、`endpoint: cfg.server`、`auth: {provider: anonymous}`、`default_graph: cfg.graph`) を書く
  - `pub fn load(&self, branch: &str, lines: &[serde_json::Value]) -> Result<(), Error>` (`--mode merge --json`、一時ファイル経由)
  - `pub fn query(&self, branch: &str, source: &str, params: serde_json::Value) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, Error>`
  - `pub fn mutate(&self, branch: &str, source: &str, params: serde_json::Value) -> Result<(), Error>`
  - `pub fn statement(&self, stmt: &str) -> Result<(), Error>` (`branch create|merge|delete`)
  - `pub fn branches(&self) -> Result<Vec<String>, Error>`
  - `pub fn export(&self, branch: &str) -> Result<String, Error>`
  - テスト用: `common::TestServer::start() -> TestServer`、`fn og(&self) -> Omnigraph`、`fn config(&self) -> Config`、`fn state_dir(&self) -> &Path`。`Drop` でサーバーを止める

- [ ] **Step 1: スキーマを置く**

`schema/schema.pg` は Task 1 で確かめた `spike/cluster/schema.pg` (`@key` つき) の写し、`schema/cluster.yaml` は `spike/cluster/cluster.yaml` の写し。冒頭のコメントを「スキーマの正本。運用のサーバーもこれを使う」に直す。

- [ ] **Step 2: 失敗するテストを書く**

`tests/common/mod.rs` の `TestServer::start` は、一時ディレクトリに `schema/` を写し、確認の記録の手順 1 から 4 (`OMNIGRAPH_HOME` と `OLLAMA_API_KEY=ollama` を一時的な値にする) を行い、空いているポート (`127.0.0.1:0` で bind して得る) で `omnigraph-server` を起動し、`/healthz` が 200 を返すまで最大 30 秒待つ。

`tests/omnigraph.rs`:

```rust
#[test] fn load_then_query_returns_rows() {
    let s = TestServer::start(); let og = s.og();
    og.load("main", &[json!({"type":"Fleeting","data":{"slug":"20261120100000","body":"a","created":"2026-11-20T01:00:00.000Z"}})]).unwrap();
    let rows = og.query("main", "query q() { match { $f: Fleeting } return { $f.slug } }", json!({})).unwrap();
    assert_eq!(rows[0]["slug"], "20261120100000");
}
#[test] fn card_violation_is_permanent() {
    let s = TestServer::start();
    let e = s.og().load("main", &[json!({"type":"Literature","data":{"slug":"x","body":"b","created":"2026-11-20T01:00:00.000Z"}})]).unwrap_err();
    assert!(!e.is_transient(), "{e}");
}
#[test] fn stopped_server_is_transient() {
    let cfg = OmnigraphConfig { server: "http://127.0.0.1:1".into(), graph: "zettelkasten".into() };
    let dir = tempfile::tempdir().unwrap();
    let e = Omnigraph::new(&cfg, dir.path()).unwrap().query("main", "query q() { match { $f: Fleeting } return { $f.slug } }", json!({})).unwrap_err();
    assert!(e.is_transient(), "{e}");
}
#[test] fn branch_create_shows_in_branches() {
    let s = TestServer::start(); let og = s.og();
    og.statement("branch create review from main").unwrap();
    assert!(og.branches().unwrap().contains(&"review".to_string()));
}
```

- [ ] **Step 3: テストが失敗することを確かめる**

Run: `nix develop -c cargo test --test omnigraph`
Expected: FAIL (`zettelist::omnigraph` がない)

- [ ] **Step 4: `src/lib.rs`、`src/paths.rs`、`src/error.rs`、`src/omnigraph.rs` を書く**

依存は `cargo add serde_json chrono`、`cargo add ureq@3 --features json`、`cargo add --dev tempfile`。Transient と Permanent は Task 1 の項目 5 の出力で分ける (サーバーに届かない、5xx、プロセスを起動できない → Transient。400 番台の拒否 → Permanent)。`query` の行の取り出し方は項目 6 に従う。

- [ ] **Step 5: テストが通ることを確かめる**

Run: `nix develop -c cargo test` と `nix build`
Expected: すべて PASS (計画 1 のテストも)。`nix build` が通る

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock flake.nix schema src tests
git commit -m "feat: Call omnigraph through one client"
```

### Task 3: 出典の値の読み分け

**Files:**
- Create: `src/source_ref.rs` (テストは同じファイルの `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `pub enum SourceRef { Id(String), Slug(String) }`
  - `pub fn SourceRef::parse(input: &str) -> SourceRef`。`Id` の値は `isbn:<13桁>` / `doi:<小文字>` / `arxiv:<版なし>` / `url:<正規化した URL>`

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test] fn isbn13_with_hyphens() { assert_eq!(SourceRef::parse("978-4-480-06899-8"), Id("isbn:9784480068998".into())); }
#[test] fn isbn10_becomes_13() { assert_eq!(SourceRef::parse("0-306-40615-2"), Id("isbn:9780306406157".into())); }
#[test] fn doi_is_lowercased() { assert_eq!(SourceRef::parse("10.1145/ABC.123"), Id("doi:10.1145/abc.123".into())); }
#[test] fn doi_org_url_becomes_doi() { assert_eq!(SourceRef::parse("https://doi.org/10.1145/ABC"), Id("doi:10.1145/abc".into())); }
#[test] fn arxiv_drops_version() { assert_eq!(SourceRef::parse("2401.01234v2"), Id("arxiv:2401.01234".into())); }
#[test] fn arxiv_url_becomes_arxiv() { assert_eq!(SourceRef::parse("https://arxiv.org/abs/2401.01234v3"), Id("arxiv:2401.01234".into())); }
#[test] fn url_drops_fragment_and_lowercases_host() { assert_eq!(SourceRef::parse("https://Example.COM/a?b=1#c"), Id("url:https://example.com/a?b=1".into())); }
#[test] fn other_text_is_slug() { assert_eq!(SourceRef::parse("ahrens2017"), Slug("ahrens2017".into())); }
#[test] fn surrounding_spaces_are_trimmed() { assert_eq!(SourceRef::parse("  ahrens2017 "), Slug("ahrens2017".into())); }
```

ISBN-10 は先頭に `978` を付け、検査数字を計算し直す (`0-306-40615-2` → `9780306406157`)。テストは 9 件。

- [ ] **Step 2: テストが失敗することを確かめる**

Run: `nix develop -c cargo test --lib source_ref`
Expected: FAIL

- [ ] **Step 3: `SourceRef::parse` を書く**

URL の正規化はこの計画で決める: 前後の空白を除く、`#` 以降を除く、scheme とホストを小文字にする。それ以外 (末尾の `/`、クエリ) は変えない。正規表現の crate は入れず、文字列の操作で書く。

- [ ] **Step 4: テストが通ることを確かめる**

Run: `nix develop -c cargo test --lib source_ref`
Expected: PASS (9 件)

- [ ] **Step 5: Commit**

```bash
git add src/source_ref.rs src/lib.rs
git commit -m "feat: Parse source values into ids"
```

### Task 4: 待ち行列

**Files:**
- Create: `src/queue.rs` (テストは同じファイル)

**Interfaces:**
- Consumes: `NoteKind` (Task 5 で定義。このタスクで `src/notes.rs` に `#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)] pub enum NoteKind { Fleeting, Literature, Permanent }` と `pub struct NewNote { pub body: String, pub source: Option<String>, pub locator: Option<String>, pub created: chrono::DateTime<chrono::Local> }` (Serialize / Deserialize) だけ先に置く)
- Produces:
  - `pub enum Job { WriteNote(NewNote), Embed { kind: NoteKind, slug: String }, Promote { slug: String }, Backup { message: String } }` (serde で JSON にする)
  - `pub struct Entry { pub id: String, pub job: Job, pub attempts: u32, pub not_before: SystemTime, pub last_error: Option<String> }`
  - `pub struct Queue`。`pub fn Queue::open(state_dir: &Path) -> Result<Queue, Error>`
  - `pub fn push(&self, job: Job) -> Result<String, Error>` (id を返す)
  - `pub fn next_ready(&self, now: SystemTime) -> Result<Option<Entry>, Error>` (積んだ順で、`not_before <= now` の最初の 1 件)
  - `pub fn earliest_wait(&self) -> Result<Option<SystemTime>, Error>` (pending の最小の `not_before`)
  - `pub fn complete(&self, id: &str) -> Result<(), Error>`
  - `pub fn retry_later(&self, entry: Entry, error: &str, now: SystemTime) -> Result<(), Error>` (`attempts` を 1 増やし、`backoff` が `None` なら failed へ)
  - `pub fn fail(&self, entry: Entry, error: &str) -> Result<(), Error>`
  - `pub fn failed(&self) -> Result<Vec<Entry>, Error>`、`pub fn retry_failed(&self, id: &str)`、`pub fn retry_all_failed(&self)`、`pub fn discard_failed(&self, id: &str)` (どれも `Result<(), Error>`。やり直すと `attempts = 0` で pending に戻る)
  - `pub fn backoff(attempts: u32) -> Option<Duration>`: 1 → 10 秒、2 → 60 秒、3 → 300 秒、4 以上 → `None`

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test] fn jobs_come_out_in_push_order() { /* push Embed a, Embed b → next_ready は a、complete(a) のあと b */ }
#[test] fn job_survives_reopen() { /* push → Queue::open をやり直しても next_ready で同じ Job が出る */ }
#[test] fn backoff_is_10s_1m_5m_then_none() {
    assert_eq!(backoff(1), Some(Duration::from_secs(10)));
    assert_eq!(backoff(2), Some(Duration::from_secs(60)));
    assert_eq!(backoff(3), Some(Duration::from_secs(300)));
    assert_eq!(backoff(4), None);
}
#[test] fn retry_later_hides_job_until_not_before() { /* retry_later(now) → next_ready(now) は None、next_ready(now + 10s) で出る */ }
#[test] fn fourth_transient_failure_goes_to_failed() { /* retry_later を 4 回 → failed() に 1 件、last_error が最後の文言 */ }
#[test] fn retry_failed_returns_to_pending_with_zero_attempts() {}
#[test] fn discard_failed_removes_it() {}
```

コメントの中身をそのまま assert にする。

- [ ] **Step 2: テストが失敗することを確かめる**

Run: `nix develop -c cargo test --lib queue`
Expected: FAIL

- [ ] **Step 3: `src/queue.rs` を書く**

1 件 1 ファイル (`queue/pending/<id>.json`、`queue/failed/<id>.json`)。id は `<UNIX 時刻のナノ秒を 20 桁>-<pid>-<連番>` にして、名前の順が積んだ順になるようにする。書き込みは一時ファイルに書いてから rename する (途中で落ちても壊れたファイルを残さない)。

- [ ] **Step 4: テストが通ることを確かめる**

Run: `nix develop -c cargo test --lib queue`
Expected: PASS (7 件)

- [ ] **Step 5: Commit**

```bash
git add src/queue.rs src/notes.rs src/lib.rs
git commit -m "feat: Add file-based job queue"
```

### Task 5: メモの書き込みと `zl "<本文>"`

**Files:**
- Modify: `src/notes.rs`, `src/main.rs`, `tests/cli.rs`
- Create: `tests/notes.rs`

**Interfaces:**
- Consumes: `Omnigraph` (Task 2)、`SourceRef` (Task 3)、`Queue`, `Job`, `NewNote`, `NoteKind` (Task 4)
- Produces:
  - `pub fn note_slug(t: chrono::NaiveDateTime) -> String` (`%Y%m%d%H%M%S`)
  - `pub struct Written { pub slug: String, pub kind: NoteKind }`
  - `pub fn write_note(og: &Omnigraph, note: &NewNote) -> Result<Written, Error>`
  - `pub struct Ctx { pub config: Config, pub paths: Paths, pub og: Omnigraph, pub embedder: Box<dyn Embedder>, pub queue: Queue }` (`src/lib.rs`)。`pub fn Ctx::from_env() -> Result<Ctx, Error>`。`Embedder` は Task 7 で中身を書く。このタスクでは `src/embed.rs` に `pub trait Embedder { fn embed(&self, text: &str) -> Result<Vec<f32>, Error>; }` だけ置く
  - CLI: `zl [-s <出典>] [-p <位置>] [--] <本文>`。成功したら `Embed` を積んで worker を起動し (Task 9 の `spawn_worker`。このタスクでは何もしない関数として置く)、`保存しました <slug>` を表示する

`write_note` の決まり:

- 本文が空白だけなら `Error::Permanent("本文が空です")`
- `created` の秒から slug を作り、Fleeting と Literature に同じ slug があれば 1 秒ずつ進める (Review Focus 1)
- 出典なしは Fleeting。出典ありは Literature と Cites (`locator` は `-p` の値)
- `SourceRef::Slug` は Source を `slug` で探す。なければ `Error::Permanent("出典 '<値>' は登録されていません")`
- `SourceRef::Id` は Source を `id` で探す。なければ仮の Source (`id`、`slug: ""`、`title: <元の値>`、`kind: other`、`input: <元の値>`、`provenance: "{}"`) を同じ `load` に入れる
- `created` は UTC の ISO 文字列 (ミリ秒まで、`Z` つき)

- [ ] **Step 1: 失敗するテストを書く**

`tests/notes.rs` (`TestServer` を使う):

```rust
#[test] fn plain_text_becomes_fleeting() { /* write_note(本文のみ) → kind == Fleeting、Fleeting を slug で引ける */ }
#[test] fn same_second_gets_next_slug() {
    // created を同じにして 2 回書く
    assert_eq!(a.slug, "20261120100000"); assert_eq!(b.slug, "20261120100001");
    // 1 件目の本文が残っている
}
#[test] fn unknown_isbn_creates_provisional_source() { /* -s 978-4-480-06899-8 -p p.42 → Literature、Source id "isbn:9784480068998" の slug は ""、kind は "other"、Cites の locator は "p.42" */ }
#[test] fn same_isbn_twice_reuses_source() { /* 2 回書いても Source は 1 件 */ }
#[test] fn unknown_slug_is_rejected_and_writes_nothing() { /* -s nosuch → Permanent エラー、Fleeting も Literature も 0 件 */ }
#[test] fn blank_body_is_rejected() {}
```

`tests/cli.rs` に足す (サーバーなし。`ZETTELIST_CONFIG` で `server = "http://127.0.0.1:1"`、`ZETTELIST_STATE_DIR` で一時ディレクトリ、`ZETTELIST_NO_SPAWN=1`):

```rust
#[test] fn blank_body_exits_nonzero() { /* zl "  " → 失敗、stderr に "本文が空です" */ }
#[test] fn stopped_server_queues_the_note() {
    // zl "メモ" → 成功 (終了コード 0)、stdout に "待ち行列に保存しました"
    // state/queue/pending に WriteNote が 1 件、本文は "メモ"
}
#[test] fn dash_body_after_double_dash() { /* zl -- "-x" → 本文 "-x" の WriteNote が積まれる */ }
```

- [ ] **Step 2: テストが失敗することを確かめる**

Run: `nix develop -c cargo test --test notes --test cli`
Expected: FAIL

- [ ] **Step 3: `note_slug`、`write_note`、`Ctx::from_env`、CLI を書く**

CLI で `write_note` が `Transient` を返したら、`Job::WriteNote(note)` を積み (`created` は書こうとした時刻のまま)、`omnigraph-server に届かないので待ち行列に保存しました` を表示して 0 で終わる。`Permanent` は stderr に出して 1 で終わる。`zl` だけ (本文なし) は `TUI は計画 3 で作ります` を表示して 0 で終わる。環境変数 `ZETTELIST_NO_SPAWN` があれば worker を起動しない。

- [ ] **Step 4: テストが通ることを確かめる**

Run: `nix develop -c cargo test --test notes --test cli`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src tests
git commit -m "feat: Write notes from the command line"
```

### Task 6: inbox、昇格、削除

**Files:**
- Modify: `src/notes.rs`, `tests/notes.rs`

**Interfaces:**
- Consumes: Task 5 の `write_note`、`Queue`
- Produces (計画 3 の TUI が呼ぶ):
  - `pub struct InboxItem { pub kind: NoteKind, pub slug: String, pub body: String, pub created: DateTime<Utc>, pub source_slug: Option<String>, pub source_id: Option<String>, pub locator: Option<String> }`
  - `pub fn InboxItem::is_old(&self, now: DateTime<Utc>) -> bool` (30 日を超えたら true)
  - `pub fn inbox(og: &Omnigraph) -> Result<Vec<InboxItem>, Error>` (DerivedFrom / DerivedFromLit のない Fleeting と Literature、古い順)
  - `pub fn promote(og: &Omnigraph, from: &[(NoteKind, String)], title: &str, body: &str, created: DateTime<Local>) -> Result<String, Error>` (Permanent の slug を返す)。成功後に呼ぶ側が `Job::Promote { slug }` を積む
  - `pub fn delete_notes(og: &Omnigraph, notes: &[(NoteKind, String)]) -> Result<(), Error>`

`promote` の決まり: Permanent と DerivedFrom (Fleeting から) / DerivedFromLit (Literature から) を 1 回の `load` で入れる。`title` か `body` が空白だけなら `Error::Permanent`。`from` が空なら `Error::Permanent`。slug は `write_note` と同じく、Permanent に同じ slug があれば 1 秒進める。

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test] fn inbox_lists_unprocessed_oldest_first() { /* 3 件書く (created を変える) → 古い順 */ }
#[test] fn inbox_shows_source_of_literature() { /* Literature の item の source_id が Some("isbn:..."), locator が Some("p.42") */ }
#[test] fn promoted_notes_leave_inbox() { /* Fleeting 1 件と Literature 1 件を promote → inbox は空、Permanent が 1 件 */ }
#[test] fn promote_with_blank_body_writes_nothing() {}
#[test] fn delete_removes_from_inbox() { /* Fleeting と Literature を 1 件ずつ消す → inbox は空 */ }
#[test] fn is_old_after_30_days() {
    // created から 30 日ちょうどは false、30 日と 1 秒は true
}
```

- [ ] **Step 2: テストが失敗することを確かめる**

Run: `nix develop -c cargo test --test notes`
Expected: 新しい 6 件が FAIL

- [ ] **Step 3: `inbox`、`promote`、`delete_notes`、`is_old` を書く**

「処理済みでない」は `not { $p derivedFrom $f }` で書く。`delete_notes` は Task 1 の項目 4 の結果に従う (Cites を先に消す必要があれば、Cites の削除と Literature の削除を別の mutate にする)。

- [ ] **Step 4: テストが通ることを確かめる**

Run: `nix develop -c cargo test --test notes`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/notes.rs tests/notes.rs
git commit -m "feat: List inbox, promote and delete notes"
```

### Task 7: 埋め込みと近いノート

**Files:**
- Modify: `src/embed.rs`, `src/notes.rs`, `tests/notes.rs`, `tests/common/mod.rs`

**Interfaces:**
- Consumes: Task 5 の `Embedder` トレイト
- Produces:
  - `pub struct OllamaEmbedder { base: String, model: String }`、`pub fn OllamaEmbedder::new(cfg: &EmbedConfig) -> OllamaEmbedder`。`POST {base}/v1/embeddings` に `{"model": .., "input": text}` を送り、`data[0].embedding` を返す。届かない、5xx → Transient。返ったベクトルが 1024 次元でない → Permanent (`埋め込みの次元が 1024 ではありません (<n>)。embed.model を確かめてください`)
  - `pub fn similarity(distance: f32) -> f32` (`1 - distance / 2`)
  - `pub fn set_embedding(og: &Omnigraph, kind: NoteKind, slug: &str, v: &[f32]) -> Result<(), Error>`
  - `pub fn note_text(og: &Omnigraph, kind: NoteKind, slug: &str) -> Result<Option<String>, Error>` (Permanent は `title` と `body` を改行でつないだもの、ほかは `body`。なければ None)
  - `pub fn get_embedding(og: &Omnigraph, kind: NoteKind, slug: &str) -> Result<Option<Vec<f32>>, Error>`
  - `pub struct Near { pub kind: NoteKind, pub slug: String, pub title: Option<String>, pub body: String, pub similarity: f32 }`
  - `pub fn nearest(og: &Omnigraph, branch: &str, kinds: &[NoteKind], v: &[f32], limit: usize, min_similarity: f32) -> Result<Vec<Near>, Error>` (種類ごとに `nearest` で引き、近い順に並べ直して `limit` 件。埋め込みのないノートは出ない)
  - テスト用: `common::FixedEmbedder` (`HashMap<String, Vec<f32>>`、知らない文は Permanent エラー) と `common::unit(parts: &[(usize, f32)]) -> Vec<f32>` (1024 次元、正規化)

- [ ] **Step 1: 失敗するテストを書く**

`src/embed.rs` の単体テスト (標準ライブラリの `TcpListener` で 1 回だけ応答する偽の Ollama を立てる):

```rust
#[test] fn ollama_returns_vector() { /* 1024 個の 0.0 を返す偽 → embed は長さ 1024 */ }
#[test] fn wrong_dimension_is_permanent() { /* 3 個返す → Permanent、文言に "1024" */ }
#[test] fn unreachable_ollama_is_transient() { /* base を http://127.0.0.1:1 → Transient */ }
#[test] fn similarity_from_distance() { assert_eq!(similarity(0.0), 1.0); assert_eq!(similarity(2.0), 0.0); assert_eq!(similarity(4.0), -1.0); }
```

`tests/notes.rs`:

```rust
#[test] fn nearest_orders_by_similarity_and_skips_missing() {
    // Permanent a = unit(&[(0,1.0)]), b = unit(&[(0,1.0),(1,1.0)]), c = 埋め込みなし
    // nearest(main, [Permanent], unit(&[(0,1.0)]), 5, 0.0) → [a, b] (c は出ない)、a.similarity は 1.0 に近い (差 1e-4 未満)
}
#[test] fn nearest_drops_below_min_similarity() { /* 直交するベクトル (similarity 0) は min 0.5 で出ない */ }
#[test] fn get_embedding_roundtrip() {}
```

- [ ] **Step 2: テストが失敗することを確かめる**

Run: `nix develop -c cargo test --lib embed` と `nix develop -c cargo test --test notes`
Expected: FAIL

- [ ] **Step 3: `OllamaEmbedder`、`similarity`、`set_embedding`、`note_text`、`get_embedding`、`nearest` を書く**

型名 (`Fleeting` など) は `NoteKind` から作り、利用者の値は必ず `--params` で渡す。`set_embedding` は `update <型> set { embedding: $v } where slug = $slug`。

- [ ] **Step 4: テストが通ることを確かめる**

Run: `nix develop -c cargo test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src tests
git commit -m "feat: Embed notes with Ollama and find near notes"
```

### Task 8: リンクと Hub の提案

**Files:**
- Create: `src/propose.rs`, `tests/propose.rs`

**Interfaces:**
- Consumes: `nearest`, `get_embedding`, `similarity` (Task 7)、`Embedder`
- Produces:
  - `pub fn ensure_review(og: &Omnigraph) -> Result<(), Error>` (review がなければ main から作り、あれば `branch merge main into review`)
  - `pub struct Proposed { pub links: usize, pub hub: bool }`
  - `pub fn propose_for(og: &Omnigraph, embedder: &dyn Embedder, cfg: &LinkConfig, slug: &str) -> Result<Proposed, Error>`

`propose_for` の決まり:

1. Permanent `slug` の埋め込みを読む。なければ `Error::Transient("埋め込みがまだありません")` (worker の `Promote` が先に埋め込みを作るので、普通は起きない)
2. `ensure_review`
3. main の Permanent から近いものを引く (自分を除く。`min_similarity` 以上、上位 `candidates` 件)。main か review に、どちら向きでもすでに Links がある組は除く
4. 残りを Links (`from: slug`、`to: 候補`、`data: {similarity}`) として review に 1 回の `load` で書く。0 件なら書かない
5. Hub: すべての Hub の `title` をその場で `embedder` で埋め込みにし、いちばん近い 1 つが `min_similarity` 以上なら、Includes (`from: hub`、`to: slug`) を review に書く。Hub が 0 件なら何もしない。すでに main か review でその Hub に入っていれば書かない

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test] fn proposes_top_candidates_to_review_only() { /* Permanent 4 件 (近さを変える)、candidates = 2 → review に Links 2 本 (近い 2 件)、main には 0 本、similarity が入っている */ }
#[test] fn nothing_above_threshold_proposes_nothing() { /* 直交するベクトルだけ、min 0.5 → links 0、review に Links なし */ }
#[test] fn existing_link_is_not_proposed_again() { /* main に b -> a の Links がある → a の提案に b は出ない */ }
#[test] fn proposing_twice_does_not_duplicate() { /* propose_for を 2 回 → review の Links は 1 回分の本数 (Review Focus 4) */ }
#[test] fn sees_permanent_added_to_main_after_review_was_created() { /* review を作る → main に Permanent を足す → propose_for で、その Permanent への提案が review に書ける */ }
#[test] fn proposes_nearest_hub() { /* Hub 2 つ、近いほうに Includes が review に入る */ }
```

- [ ] **Step 2: テストが失敗することを確かめる**

Run: `nix develop -c cargo test --test propose`
Expected: FAIL

- [ ] **Step 3: `ensure_review`、`propose_for` を書く**

- [ ] **Step 4: テストが通ることを確かめる**

Run: `nix develop -c cargo test --test propose`
Expected: PASS (6 件)

- [ ] **Step 5: Commit**

```bash
git add src/propose.rs src/lib.rs tests/propose.rs
git commit -m "feat: Propose links and hubs on the review branch"
```

### Task 9: worker

**Files:**
- Create: `src/worker.rs`
- Modify: `src/main.rs`, `tests/cli.rs`

**Interfaces:**
- Consumes: `Queue`, `Job`, `backoff` (Task 4)、`write_note` (Task 5)、`note_text`, `set_embedding` (Task 7)、`propose_for` (Task 8)、`backup::run` (Task 11。このタスクでは `Job::Backup` を `Ok(())` で済ませ、Task 11 で差し替える)
- Produces:
  - `pub fn execute(ctx: &Ctx, job: &Job) -> Result<(), Error>`
    - `WriteNote(n)` → `write_note`、成功したら `Embed` を積む
    - `Embed { kind, slug }` → `note_text`、`embedder.embed`、`set_embedding`。ノートがもう消えていたら `Ok(())`
    - `Promote { slug }` → Permanent を埋め込み、`propose_for`
  - `pub fn run_queue(queue: &Queue, lock_path: &Path, exec: &mut dyn FnMut(&Job) -> Result<(), Error>, now: &dyn Fn() -> SystemTime, sleep: &mut dyn FnMut(Duration)) -> Result<(), Error>`
  - `pub fn run_worker(ctx: &Ctx) -> Result<(), Error>` (`run_queue` に `execute` と実時間を渡す)
  - `pub fn spawn_worker(paths: &Paths) -> Result<(), Error>` (`current_exe() --worker` を `process_group(0)` で起動し、stdout / stderr を `worker.log` に追記する。待たない。`ZETTELIST_NO_SPAWN` があれば何もしない)
  - `pub fn reembed_all(ctx: &Ctx) -> Result<usize, Error>` (すべての Fleeting / Literature / Permanent に `Embed` を積み、件数を返す)
  - CLI: `zl --worker`、`zl --reembed` (積んだ件数を表示し、worker を起動する)

`run_queue` の決まり:

1. `worker.lock` を `File::try_lock` で取る。取れなければ (ほかの worker が動いている) `Ok(())`
2. `next_ready` があれば実行する。成功 → `complete`。Transient → `retry_later`。Permanent → `fail`
3. ready がなく、待っているもの (`earliest_wait`) があれば、その時刻まで `sleep` して 2 へ
4. 空になったらロックを外し、待ち行列をもう一度見る。積まれていれば 1 へ (Review Focus 3)。空なら終わる

- [ ] **Step 1: 失敗するテストを書く**

`src/worker.rs` の単体テスト (`run_queue` に偽の `exec`、`now`、`sleep` を渡す。`sleep` は `now` の時計を進める):

```rust
#[test] fn successful_jobs_empty_the_queue() {}
#[test] fn permanent_error_goes_to_failed_without_retry() { /* exec の呼び出しは 1 回、failed に 1 件 */ }
#[test] fn transient_error_retries_three_times_then_fails() { /* exec の呼び出しは 4 回、sleep は 10s, 60s, 300s の順、failed に 1 件 */ }
#[test] fn transient_then_success_completes() { /* 1 回目 Transient、2 回目 Ok → pending も failed も 0 件 */ }
#[test] fn locked_queue_returns_immediately() { /* 先に lock を取っておく → exec は呼ばれない */ }
#[test] fn job_pushed_during_last_job_is_run() { /* exec の中で push → その Job も実行される */ }
```

`tests/cli.rs`:

```rust
#[test] fn worker_writes_queued_note_when_server_is_back() {
    // TestServer を指す設定 (embed.ollama は http://127.0.0.1:1) で、WriteNote を 1 件積んでおく
    // zl --worker --once → 成功、inbox に 1 件
    // Embed は Ollama に届かず Transient → pending に Embed が 1 件、attempts == 1
}
```

`--once` は隠しオプション。ready のものを 1 巡だけ実行し、待ちのあるものは待たずに終わる (テストで最大 6 分待たないため)。

- [ ] **Step 2: テストが失敗することを確かめる**

Run: `nix develop -c cargo test --lib worker` と `nix develop -c cargo test --test cli`
Expected: FAIL

- [ ] **Step 3: `execute`、`run_queue`、`run_worker`、`spawn_worker`、`reembed_all`、CLI を書く**

Task 5 の何もしない `spawn_worker` を差し替える。

- [ ] **Step 4: テストが通ることを確かめる**

Run: `nix develop -c cargo test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src tests
git commit -m "feat: Run queued jobs in a worker"
```

### Task 10: レビューの一覧と適用

**Files:**
- Create: `src/review.rs`, `tests/review.rs`

**Interfaces:**
- Consumes: `ensure_review` (Task 8)、`Queue`, `Job::Backup` (Task 4)
- Produces (計画 3 のレビュー画面が呼ぶ):
  - `pub enum Proposal { Link { from: String, to: String, similarity: f32 }, Hub { hub: String, note: String } }`、`pub fn Proposal::key(&self) -> String` (`link:<from>:<to>` / `hub:<hub>:<note>`)
  - `pub fn list_proposals(og: &Omnigraph) -> Result<Vec<Proposal>, Error>` (review にあって main にない Links と Includes。Links を先に、近い順)
  - `pub enum Decision { Adopt { reason: Option<String> }, Reject }`
  - `pub struct ReviewState { pub decisions: BTreeMap<String, Decision>, pub reinsert: Vec<Proposal> }` (serde)。`pub fn ReviewState::load(path: &Path) -> Result<ReviewState, Error>` (なければ空)、`pub fn save(&self, path: &Path) -> Result<(), Error>`
  - `pub struct Summary { pub links: usize, pub hubs: usize }`、`pub fn Summary::message(&self) -> String` (`merge: links 3, hub 1`。0 件の種類は書かない。両方 0 なら `merge`)
  - `pub fn apply(og: &Omnigraph, state_path: &Path, queue: &Queue) -> Result<Summary, Error>`

`apply` の決まり (途中で落ちても、もう一度呼べば続きから終わるようにする):

1. `ensure_review`
2. `list_proposals` のうち判断のないものを `reinsert` に入れて `save` する (消す前に書く。Review Focus 5)
3. 却下したものと `reinsert` のものを review から消す (Links は `delete Links where @src = $a and @dst = $b`、Includes も同じ形)
4. 理由つきで採用したリンクは、`reason` を付けて review に書き直す (`@key` なので 1 本のまま)
5. `branch merge review into main`
6. `reinsert` のものを review に書き戻す (`ensure_review` のあとで)。`decisions` と `reinsert` を空にして `save`
7. `Job::Backup { message: summary.message() }` を積む

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test] fn lists_only_proposals_not_in_main() {}
#[test] fn apply_merges_adopted_and_drops_rejected() { /* 提案 2 本、1 本採用 1 本却下 → main に 1 本、list_proposals は空 */ }
#[test] fn adopt_with_reason_sets_reason_in_main() { /* main の Links の reason が "粒度の話" */ }
#[test] fn undecided_stays_in_review() { /* 3 本のうち 1 本だけ採用 → main に 1 本、list_proposals に残り 2 本 */ }
#[test] fn apply_twice_in_a_row_works() { /* 2 回目の往復: apply → 新しい提案 → apply が成功する */ }
#[test] fn rerun_after_crash_keeps_undecided() { /* ReviewState に reinsert を書いた状態 (手順 3 の後で止まった状態) を作って apply → 未判断の提案が review に戻る */ }
#[test] fn summary_message_skips_zero() {
    assert_eq!(Summary { links: 3, hubs: 0 }.message(), "merge: links 3");
    assert_eq!(Summary { links: 3, hubs: 1 }.message(), "merge: links 3, hub 1");
    assert_eq!(Summary { links: 0, hubs: 0 }.message(), "merge");
}
#[test] fn apply_queues_backup() { /* pending に Backup { message: "merge: links 1" } */ }
```

- [ ] **Step 2: テストが失敗することを確かめる**

Run: `nix develop -c cargo test --test review`
Expected: FAIL

- [ ] **Step 3: `src/review.rs` を書く**

- [ ] **Step 4: テストが通ることを確かめる**

Run: `nix develop -c cargo test --test review`
Expected: PASS (8 件)

- [ ] **Step 5: Commit**

```bash
git add src/review.rs src/lib.rs tests/review.rs
git commit -m "feat: Apply review decisions to main"
```

### Task 11: バックアップ

**Files:**
- Create: `src/backup.rs`
- Modify: `src/worker.rs` (`Job::Backup` で `backup::run` を呼ぶ)、`src/main.rs` (`zl --backup`)

**Interfaces:**
- Consumes: `Omnigraph::export` (Task 2)、`BackupConfig` (計画 1)
- Produces:
  - `pub enum Outcome { Committed, NoChanges, NotConfigured }`
  - `pub fn commit_export(repo: &Path, file_name: &str, contents: &str, message: &str, push: bool) -> Result<Outcome, Error>` (ファイルを書き、`git add`、`git commit -m <message>`。変更がなければ `NoChanges`。`push` なら `git push`。push の失敗は Transient、ほかの git の失敗は Permanent)
  - `pub fn run(ctx: &Ctx, message: &str) -> Result<Outcome, Error>` (`backup.repo` が None なら `NotConfigured`。main を export して `<graph>.jsonl` に書く)
  - CLI: `zl --backup` (メッセージは `backup`)。`NotConfigured` なら `backup.repo が設定されていません` を stderr に出して 1 で終わる。`NoChanges` なら `変更はありません`

- [ ] **Step 1: 失敗するテストを書く**

`src/backup.rs` の単体テスト (一時ディレクトリで `git init` したリポジトリを使う。`user.name` と `user.email` はそのリポジトリの設定に書く):

```rust
#[test] fn first_export_is_committed() { /* Committed、git log -1 --format=%s が "merge: links 3" */ }
#[test] fn same_contents_gives_no_changes() { /* 2 回目は NoChanges、commit は 1 件のまま */ }
#[test] fn push_failure_is_transient() { /* remote のない repo で push = true → Transient。commit は残る */ }
#[test] fn commit_message_has_no_attribution() { /* git log -1 --format=%B に "Co-Authored-By" がない */ }
```

- [ ] **Step 2: テストが失敗することを確かめる**

Run: `nix develop -c cargo test --lib backup`
Expected: FAIL

- [ ] **Step 3: `commit_export`、`run`、worker と CLI のつなぎを書く**

`Job::Backup` で `NotConfigured` は `Ok(())` として扱う (設定していない人には何も起きない)。

- [ ] **Step 4: テストが通ることを確かめる**

Run: `nix develop -c cargo test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src
git commit -m "feat: Back up main to the data repository"
```

### Task 12: 手元のサーバーで通して確かめる

自動テストの代わりに、利用者の環境の omnigraph-server と Ollama で、書いてから提案が review に入るまでを 1 度通す。

**Files:**
- Create: `docs/superpowers/notes/2026-11-20-core-smoke.md`

- [ ] **Step 1: グラフを作り、サーバーを起動する**

データの置き場所は `~/.local/share/zettelist/graph/` とする (git に入れない)。`schema/` を cluster 設定として、確認の記録の手順 1 から 4 で起動する (`--bind 127.0.0.1:8080`)。launchd での常駐は dotfiles の作業なので、ここでは手で起動する。

- [ ] **Step 2: 書いて、worker の結果を見る**

Run: `zl "小さいノートはつなぎやすい"` と `zl -s 978-4-480-06899-8 -p p.42 "箇条書きは考えを止める"`。数秒後に `ls ~/.local/share/zettelist/queue/pending` と `tail ~/.local/share/zettelist/worker.log`
Expected: 2 件とも `保存しました <slug>`。pending は空。Fleeting と Literature に埋め込みが入っている (`omnigraph query` で `embedding` を出して確かめる)

- [ ] **Step 3: 昇格と提案を通す**

TUI がまだないので、昇格は手で代わりをする。

1. `omnigraph load` で、話題の近い Permanent を 3 件 (埋め込みなし) 入れる
2. `zl --reembed` で埋め込みを作る
3. 4 件目の Permanent を `omnigraph load` で入れ、`queue/pending/` に `Job::Promote { slug: <4 件目> }` のファイルを Task 4 の形式で手で置き、`zl --worker` を実行する

Expected: review に 4 件目からの Links が入る (`omnigraph branch diff review --into main --json`)

- [ ] **Step 4: 結果を記録する**

`docs/superpowers/notes/2026-11-20-core-smoke.md` に、手順、かかった時間 (書き込みから埋め込みが入るまで)、気づいた問題を書く。問題があれば、直すタスクを足すか利用者に聞く。

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/notes/2026-11-20-core-smoke.md
git commit -m "docs: Record core smoke test"
```

## このあと

- 計画 3 (TUI): inbox、昇格の題の入力欄と `$EDITOR`、レビュー (`a` `A` `d` `c` `n` `u` `m`)、失敗の一覧、似ているノートの表示、`[theme]` と `[keys]`。この計画の `inbox`、`promote`、`delete_notes`、`nearest`、`list_proposals`、`ReviewState`、`apply`、`Queue::failed` などを呼ぶ
- 計画 4 (出典の登録): `Job::RegisterSource`、Crossref / openBD / Google Books / arXiv / ページのメタデータ、slug の規則と辞書 (lindera、`wana_kana`)、レビューでの出典の採用と「直す」(新しい ID の Source への Cites の付け替えは、Task 1 と同じ形の確認から始める)
