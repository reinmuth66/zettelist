# zettelist 土台と最初の確認 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** nix flake の開発環境、`zl` バイナリの骨組みと設定の読み込みを作り、設計書の「最初の確認」4 項目を実際の omnigraph-server と Ollama で確かめて、結果を設計書に書き戻す。

**Architecture:** この計画は本体を作らない。本体 (書き込み、inbox、昇格、レビュー、worker) は、確認の結果で Omnigraph への接続の作りが変わるため、計画 2 として確認のあとに書く。確認は使い捨ての shell スクリプトで行い、結果は文書に残す。

**Tech Stack:** Rust (stable), nix flake, serde + toml, clap, omnigraph / omnigraph-server, Ollama (`qwen3-embedding:0.6b`)

**Spec:** `docs/superpowers/specs/2026-11-19-zettelist-design.md`

## Global Constraints

- コマンド名は `zl`、パッケージ名は `zettelist`
- 設定ファイルは `~/.config/zettelist/config.toml`。ファイルがなくても既定値で動く
- 既定値: `omnigraph.server = "http://127.0.0.1:8080"` (確認 4 で直す)、`omnigraph.graph = "zettelkasten"`、`backup.push = false`、`link.candidates = 5`、`link.min_similarity = 0.0`、`link.inbox_similarity = 0.0`、`ui.locator = false`
- Python と Python ライブラリは使わない。ファイル編集に `sed -i` を使わない
- commit メッセージに Claude の関与 (Co-Authored-By など) を書かない
- 文書とメッセージは日本語。絵文字を使わない
- 埋め込みは omnigraph-server 側で作る。環境変数 `OMNIGRAPH_EMBED_PROVIDER=openai-compatible`、`OMNIGRAPH_EMBED_BASE_URL=http://localhost:11434/v1`、`OMNIGRAPH_EMBED_MODEL=qwen3-embedding:0.6b`
- 作業の前に `omnigraph skill export ~/.claude/skills/omnigraph --force` で、入れた CLI に合う agent skill を入れる

## Review Focus

1. 設定ファイルに一部の項目しかない (例: `[link]` だけ) → 書かれていない項目は既定値になる
2. 設定ファイルの `~` で始まるパス (`backup.repo`) → ホームディレクトリに展開される
3. 設定ファイルの書式が壊れている → どの行が悪いかを示して終了し、黙って既定値で動かない
4. 設定ファイルに知らない項目がある (`[theme]`、`[keys]` は計画 2 で中身を決める) → エラーにせず読み飛ばす
5. omnigraph が nixpkgs にない → flake で配布物から入れ、devShell で `omnigraph --version` と `omnigraph-server --version` が通る

---

### Task 1: nix flake と `zl` の骨組み

**Files:**
- Create: `flake.nix`, `Cargo.toml`, `src/main.rs`, `.gitignore`
- Test: `tests/cli.rs`

**Interfaces:**
- Produces: バイナリ `zl`。`zl --version` が `zettelist <version>` を出す。`flake.nix` の `devShells.default` と `packages.default`

- [ ] **Step 1: `flake.nix` を書く**

`devShells.default` に Rust のツールチェーン (cargo, rustc, clippy, rustfmt, rust-analyzer)、`ollama`、`omnigraph` と `omnigraph-server` を入れる。omnigraph は nixpkgs になければ、GitHub の配布物 (ModernRelay/omnigraph のリリースの aarch64-darwin 版) を `fetchurl` して入れる derivation を flake の中に書く。版は書いた時点の最新に固定する。`packages.default` は `rustPlatform.buildRustPackage` で `zl` を出す。

- [ ] **Step 2: devShell が通ることを確かめる**

Run: `nix develop -c sh -c 'cargo --version && ollama --version && omnigraph --version && omnigraph-server --version'`
Expected: 4 つとも版が表示される

- [ ] **Step 3: 失敗するテストを書く**

`tests/cli.rs` に `version_prints_package_name`: `assert_cmd` で `zl --version` を実行し、成功して標準出力が `zettelist ` で始まることを確かめる。

- [ ] **Step 4: テストが失敗することを確かめる**

Run: `nix develop -c cargo test --test cli`
Expected: FAIL (バイナリ `zl` がない)

- [ ] **Step 5: `Cargo.toml` (package `zettelist`、`[[bin]] name = "zl"`) と、clap で `--version` だけを持つ `src/main.rs` を書く**

- [ ] **Step 6: テストが通ることを確かめる**

Run: `nix develop -c cargo test --test cli` と `nix build && ./result/bin/zl --version`
Expected: PASS、`zettelist 0.1.0`

- [ ] **Step 7: Commit**

```bash
git add flake.nix flake.lock Cargo.toml Cargo.lock src/main.rs tests/cli.rs .gitignore
git commit -m "build: Add nix flake and zl skeleton"
```

### Task 2: 設定の読み込み

**Files:**
- Create: `src/config.rs`
- Modify: `src/main.rs` (mod 宣言)
- Test: `src/config.rs` の `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: Task 1 のクレート
- Produces:
  - `pub struct Config { pub omnigraph: OmnigraphConfig, pub backup: BackupConfig, pub link: LinkConfig, pub ui: UiConfig }`
  - `pub struct OmnigraphConfig { pub server: String, pub graph: String }`
  - `pub struct BackupConfig { pub repo: Option<PathBuf>, pub push: bool }`
  - `pub struct LinkConfig { pub candidates: usize, pub min_similarity: f32, pub inbox_similarity: f32 }`
  - `pub struct UiConfig { pub locator: bool }`
  - `pub fn Config::from_toml_str(s: &str, home: &Path) -> Result<Config, ConfigError>`
  - `pub fn Config::load() -> Result<Config, ConfigError>` (`~/.config/zettelist/config.toml`。なければ `Config::default()`)

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test] fn empty_file_gives_defaults() {
    let c = Config::from_toml_str("", Path::new("/home/u")).unwrap();
    assert_eq!(c.omnigraph.graph, "zettelkasten");
    assert_eq!(c.omnigraph.server, "http://127.0.0.1:8080");
    assert!(!c.backup.push);
    assert_eq!(c.link.candidates, 5);
    assert!(!c.ui.locator);
}
#[test] fn partial_section_keeps_other_defaults() {
    let c = Config::from_toml_str("[link]\ncandidates = 3\n", Path::new("/home/u")).unwrap();
    assert_eq!(c.link.candidates, 3);
    assert_eq!(c.link.min_similarity, 0.0);
    assert_eq!(c.omnigraph.graph, "zettelkasten");
}
#[test] fn tilde_in_repo_expands_to_home() {
    let c = Config::from_toml_str("[backup]\nrepo = \"~/data\"\n", Path::new("/home/u")).unwrap();
    assert_eq!(c.backup.repo, Some(PathBuf::from("/home/u/data")));
}
#[test] fn unknown_sections_are_ignored() {
    assert!(Config::from_toml_str("[theme]\nx = \"red\"\n[keys]\ny = \"z\"\n", Path::new("/h")).is_ok());
}
#[test] fn broken_toml_reports_line() {
    let e = Config::from_toml_str("[link\n", Path::new("/h")).unwrap_err();
    assert!(e.to_string().contains("1"));
}
```

- [ ] **Step 2: テストが失敗することを確かめる**

Run: `nix develop -c cargo test config`
Expected: FAIL (`Config` がない)

- [ ] **Step 3: `src/config.rs` を書く**

serde の `#[serde(default)]` を構造体ごとに付け、`Default` で既定値を持たせる。`ConfigError` は toml のエラー (行番号を含む) を包む。

- [ ] **Step 4: テストが通ることを確かめる**

Run: `nix develop -c cargo test config`
Expected: PASS (5 件)

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/main.rs
git commit -m "feat: Load config with defaults"
```

### Task 3: 最初の確認 (omnigraph-server と Ollama)

**Files:**
- Create: `spike/schema.pg`, `spike/check.sh`, `docs/superpowers/notes/2026-11-19-omnigraph-check.md`
- Modify: `docs/superpowers/specs/2026-11-19-zettelist-design.md` (「最初の確認」とそれに関わる節)

**Interfaces:**
- Produces: 確認の記録 (`docs/superpowers/notes/2026-11-19-omnigraph-check.md`)。計画 2 はこれと直した設計書を元に書く

このタスクは使い捨ての確認なので TDD は行わない。`spike/check.sh` は各確認を順に実行し、結果を標準出力に出す。

- [ ] **Step 1: 確認用のスキーマを書く**

`spike/schema.pg` に、設計書の変更後のスキーマ (Source は `id @key` と `slug`、Links は `reason?` と `similarity`、Fleeting / Literature / Permanent に `embedding: Vector(1024)`、Hub、Cites `@card(1..1)`、DerivedFrom、DerivedFromLit、Includes) を書く。残りの項目は運用文書のとおり。

- [ ] **Step 2: Ollama と omnigraph-server を起動する**

Run: `ollama pull qwen3-embedding:0.6b`。omnigraph の skill の手順でグラフ `zettelkasten` を作り、上の環境変数を付けて omnigraph-server を起動する
Expected: サーバーが待ち受けのポートを表示する。そのポートを記録する (確認 4)

- [ ] **Step 3: `spike/check.sh` で 4 項目を確かめる**

HTTP の呼び出しは `ax` で行う (`ax <url> -X POST -d @file`)。

1. HTTP API: ノードとエッジの書き込み (Literature と仮の Source と Cites を 1 回で)、ブランチの作成、ブランチへの書き込み、差分、main へのマージ、export、ベクトル検索。各操作の URL、送ったもの、返ったものを記録する。HTTP にない操作は `omnigraph --target <server>` で試す
2. review ブランチ: review を作ったあとで main に Permanent を足し、review からその Permanent へのリンクを書けるか。書けなければ、取り込みの操作があるか
3. 埋め込み: テキストだけを書き込んだノードに埋め込みが自動で入るか。`nearest` の結果に近さの値が付くか。付くなら、似た 2 文と無関係な 2 文の近さの値を 1 組ずつ記録する
4. 待ち受けの既定のポートと、グラフの作り方の手順

Run: `sh spike/check.sh`
Expected: 4 項目それぞれに「できた / できない」と根拠の出力が出る

- [ ] **Step 4: 結果を記録する**

`docs/superpowers/notes/2026-11-19-omnigraph-check.md` に 4 項目の結果、使った URL と要求の例、分かった制約を書く。

- [ ] **Step 5: 設計書を直す**

設計書の「最初の確認」を結果に置き換える。確認 2 で review ブランチにリンクを載せられなかった場合は、設計書の切り替え案 (提案は zl の手元に持ち、`m` で main に 1 回で書き込む) をレビューの節に書く。確認 3 で近さの値が付かない場合は、`min_similarity` の扱いを書き直す。確認 4 のポートで設定の既定値と `src/config.rs` の既定値を直す (Task 2 のテストも直す)。利用者の判断が要る食い違いが出たら、直す前に止めて利用者に聞く。

- [ ] **Step 6: Commit**

```bash
git add spike docs/superpowers/notes docs/superpowers/specs/2026-11-19-zettelist-design.md src/config.rs
git commit -m "docs: Record omnigraph-server check results"
```

## このあと

計画 2 (本体) を、直した設計書と確認の記録を元に書く。範囲は書き込み (`zl "<本文>"` と TUI の入力欄)、出典の登録、inbox、昇格、リンクの候補探し、Hub の振り分け、レビュー、待ち行列と worker、バックアップ、`[theme]` と `[keys]`。
