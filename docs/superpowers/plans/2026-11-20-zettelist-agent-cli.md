# zettelist エージェント向けコマンドライン Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** TUI で行う操作 (inbox、全文、似ているノート、昇格、削除、レビューの採否とマージ、失敗のやり直し) を `zl --<操作>` で行えるようにし、エージェント向けの skill を zl から書き出せるようにする。

**Architecture:** 引数の解釈と振り分けを `src/cli.rs` に移し、各操作は計画 2 のライブラリ関数を呼ぶだけにする。出力は `--json` で 1 つの JSON 値、エラーは標準エラーに JSON。skill は `skill/SKILL.md` を `include_str!` で埋め込む。

**Tech Stack:** Rust 1.99、clap (derive)、serde_json、既存の `zettelist` ライブラリ

**Spec:** `docs/superpowers/specs/2026-11-20-zettelist-agent-cli-design.md` (本体の設計: `docs/superpowers/specs/2026-11-19-zettelist-design.md`)

## Global Constraints

- サブコマンドは持たない。`zl "<本文>"` は必ずメモ。操作はすべて `--` で始まるオプション
- 操作のオプションは同時に 1 つだけ。本文と操作のオプションは同時に使えない (clap の `ArgGroup` で排他)
- 削除 (`--delete`)、捨てる (`--discard`)、マージ (`--merge`) は `--yes` 必須。ないときは何もせず終了コード 2 で理由を出す
- `--json`: 成功時は標準出力に 1 つの JSON 値。失敗時は標準エラーに `{"error": "<文言>", "kind": "invalid|permanent|transient"}`
- 終了コード: 0 成功、1 処理の失敗 (Permanent / Transient)、2 使い方の誤りと入力の誤り (`Error::Invalid`、clap の誤り、`--yes` なし)
- 判断 (`review.json`) の書き込みは `review::lock_review(paths.review_lock())` を取ってから行う
- 昇格とマージと失敗のやり直しのあとは `worker::spawn_worker` を呼ぶ
- 既存の `zl "<本文>"`、`--worker`、`--reembed`、`--backup` の挙動と出力は変えない (既存の tests/cli.rs がそのまま通る)
- テストは `nix develop -c cargo test`。spawn する zl には `ZETTELIST_NO_SPAWN=1`、`ZETTELIST_STATE_DIR`、`ZETTELIST_CONFIG`、`OMNIGRAPH_HOME` (一時ディレクトリ) を渡す
- 文書とメッセージは日本語、絵文字なし。commit に Claude の関与を書かない。`sed -i` と Python は使わない。リポジトリ全体の `cargo fmt` はしない

## JSON の形 (skill にもこの形を書く)

```
inbox:    [{"kind":"fleeting|literature","slug":"...","body":"...","created":"<RFC3339 UTC>","old":bool,"source_id":str|null,"source_slug":str|null,"locator":str|null}]
show:     {"kind":"fleeting|literature|permanent","slug":"...","title":str|null,"body":"...","created":"..."}
similar:  {"inbox":[{"kind","slug","body","similarity"}],"permanent":[{"slug","title","body","similarity"}]}
review:   [{"key":"link:a:b","type":"link","from":"a","to":"b","similarity":0.82,"decision":"adopt|reject|null","reason":str|null}
           |{"key":"hub:h:n","type":"hub","hub":"h","note":"n","decision":...}]
promote:  {"slug":"<Permanent の slug>"}
merge:    {"links":3,"hubs":1}
failed:   [{"id":"...","job":"write_note|embed|promote|backup","attempts":4,"last_error":str|null}]
その他の成功 (delete、adopt、reject、undecide、retry、discard、skill-export): {"ok":true}
```

## Review Focus

1. 本文が `--inbox` などのオプション名に見える文字列 → `zl -- "--inbox"` はメモとして書かれる (既存の `--` の扱いを壊さない)
2. 存在しない slug や key を渡す (`--show nosuch`、`--adopt link:x:y`) → 終了コード 2、`kind: "invalid"`、何も書き換えない
3. `--promote` に昇格済みや Permanent の slug が混ざる → 終了コード 2、何も書かない (`promote` の `Invalid` をそのまま返す)
4. `--merge` と worker の提案書き込みが同時に走る → `review.lock` で順番になり、未判断の提案が main に入らない (計画 2 の `apply` が保証。CLI は余計なことをしない)
5. `--json` を付けたのにエラーが人向けの文言だけで出る → 必ず JSON で出る (clap の使い方の誤りも含む)

---

## File Structure

| ファイル | 役割 |
|---|---|
| `src/cli.rs` | clap の定義、操作の振り分け、出力 (人向け / JSON)、終了コード |
| `src/main.rs` | `zettelist::cli::main()` を呼ぶだけ |
| `src/notes.rs` | `show_note` を足す |
| `src/review.rs` | `decide` を足す (判断の書き込み) |
| `skill/SKILL.md` | エージェント向けの skill |
| `src/skill.rs` | 埋め込みと書き出し |
| `tests/cli_agent.rs` | 新しい操作の結合テスト |

---

### Task 1: 振り分けと出力の土台

**Files:**
- Create: `src/cli.rs`
- Modify: `src/main.rs`, `src/lib.rs`, `src/error.rs` (`kind()` を足す)
- Test: `tests/cli.rs` (既存がそのまま通ること)、`tests/cli_agent.rs` (新規)

**Interfaces:**
- Produces:
  - `pub fn cli::main() -> std::process::ExitCode`
  - `pub fn Error::kind(&self) -> &'static str` (`"invalid" | "permanent" | "transient"`)
  - `src/cli.rs` 内: `enum Action { Write(NewNote), Worker{once}, Reembed, Backup, Tui, ... }` と `fn output(json: bool, value: serde_json::Value, human: &str)`、`fn fail(json: bool, e: &Error) -> ExitCode` (Invalid は 2、ほかは 1)
  - `--json` フラグ。操作オプションの `ArgGroup("action")` と本文の排他

- [ ] **Step 1: 失敗するテストを書く** (`tests/cli_agent.rs`、サーバーなし)

```rust
#[test] fn json_usage_error_is_json_on_stderr() {
    // zl --json --inbox --review → 終了コード 2、stderr を serde_json で読むと kind == "invalid"
}
#[test] fn body_and_action_conflict() { /* zl --inbox "メモ" → 終了コード 2 */ }
#[test] fn dash_dash_inbox_is_a_note() { /* zl -- "--inbox" (server 停止) → 0、WriteNote の本文は "--inbox" */ }
```

- [ ] **Step 2: 失敗を確かめる** — Run: `nix develop -c cargo test --test cli_agent` / Expected: FAIL
- [ ] **Step 3: `src/cli.rs` に移し、`--json` と排他と終了コードを実装する**。clap の誤りは `Cli::try_parse` で受け、`--json` が引数にあれば JSON で、なければ clap の表示のまま出して 2 で終わる
- [ ] **Step 4: 通ることを確かめる** — Run: `nix develop -c cargo test` / Expected: 既存も含めて PASS
- [ ] **Step 5: Commit** — `git commit -m "refactor: Move CLI dispatch into cli module"`

### Task 2: 読む操作 (`--inbox`、`--show`、`--similar`)

**Files:** Modify `src/cli.rs`, `src/notes.rs`; Test `tests/cli_agent.rs`

**Interfaces:**
- Consumes: `notes::inbox`, `notes::nearest`, `notes::get_embedding`
- Produces: `pub struct Note { pub kind: NoteKind, pub slug: String, pub title: Option<String>, pub body: String, pub created: DateTime<Utc> }`、`pub fn show_note(og: &Omnigraph, slug: &str) -> Result<Note, Error>` (Fleeting、Literature、Permanent の順に探す。なければ `Error::Invalid("ノート '<slug>' はありません")`)

`--similar <slug>`: そのノートの埋め込みで、inbox (Fleeting と Literature、自分を除く、`inbox_similarity` 以上、上位 10) と Permanent (`inbox_similarity` 以上、上位 3) を引く。埋め込みがまだなら両方空で返す。

- [ ] **Step 1: 失敗するテストを書く** (TestServer、`FixedEmbedder` は使わず `set_embedding` で直接入れる)

```rust
#[test] fn inbox_json_lists_notes() { /* 2 件書く → --inbox --json の配列長 2、[0].slug が古いほう、old == false */ }
#[test] fn show_returns_body() { /* --show <slug> --json の body が一致 */ }
#[test] fn show_unknown_is_invalid() { /* 終了コード 2、kind == "invalid" */ }
#[test] fn similar_lists_near_inbox_and_permanent() { /* 埋め込みを入れた 3 件 → inbox に近いほうだけ (threshold 0.5)、自分は出ない */ }
#[test] fn similar_without_embedding_is_empty() {}
```

- [ ] **Step 2: 失敗を確かめる** — `nix develop -c cargo test --test cli_agent`
- [ ] **Step 3: `show_note` と 3 つの操作を実装する** (JSON の形は上の節のとおり。`created` は RFC3339 の UTC)
- [ ] **Step 4: 通ることを確かめる** — `nix develop -c cargo test`
- [ ] **Step 5: Commit** — `git commit -m "feat: Read inbox and notes from the command line"`

### Task 3: 昇格と削除 (`--promote`、`--delete`)

**Files:** Modify `src/cli.rs`; Test `tests/cli_agent.rs`

**Interfaces:**
- Consumes: `notes::promote`, `notes::delete_notes`, `show_note` (種類を知るため), `Queue::push(Job::Promote)`, `spawn_worker`
- CLI: `--promote <slug,...> --title <題> (--body <本文> | --body-file <path>)`、`--delete <slug,...> --yes`。`--title`、`--body`、`--body-file` は `--promote` のときだけ使える (`requires`)

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test] fn promote_creates_permanent_and_queues_job() { /* Fleeting 2 件 → --promote a,b --title T --body B --json → slug が返る、--inbox は空、pending に Promote */ }
#[test] fn promote_reads_body_file() {}
#[test] fn promote_permanent_slug_is_invalid() { /* 終了コード 2、何も書かれない */ }
#[test] fn delete_requires_yes() { /* --delete a → 終了コード 2、ノートは残る */ }
#[test] fn delete_with_yes_removes() {}
#[test] fn delete_unknown_is_invalid() {}
```

- [ ] **Step 2〜4**: 失敗を確かめる → 実装 → `nix develop -c cargo test`
- [ ] **Step 5: Commit** — `git commit -m "feat: Promote and delete notes from the command line"`

### Task 4: レビュー (`--review`、`--adopt`、`--reject`、`--undecide`、`--merge`)

**Files:** Modify `src/review.rs`, `src/cli.rs`; Test `tests/cli_agent.rs`

**Interfaces:**
- Consumes: `review::{list_proposals, ReviewState, Decision, apply, lock_review}`, `spawn_worker`
- Produces: `pub fn decide(og: &Omnigraph, state_path: &Path, lock_path: &Path, key: &str, decision: Option<Decision>) -> Result<(), Error>` (`review.lock` を取り、`key` が `list_proposals` にあることを確かめ、なければ `Error::Invalid("提案 '<key>' はありません")`。`None` は判断を外す)
- CLI: `--review`、`--adopt <key> [--reason <理由>]`、`--reject <key>`、`--undecide <key>`、`--merge --yes`。`--reason` は `--adopt` のときだけ

- [ ] **Step 1: 失敗するテストを書く** (propose のテストと同じ要領で review に Links を 2 本作る)

```rust
#[test] fn review_json_lists_with_decisions() { /* adopt 1 本 → [..].decision が "adopt" と null */ }
#[test] fn adopt_with_reason_then_merge() { /* --adopt k --reason R、--merge --yes --json → {"links":1,"hubs":0}、main の reason が R、pending に Backup */ }
#[test] fn reject_then_merge_drops() {}
#[test] fn undecide_clears() {}
#[test] fn unknown_key_is_invalid() { /* review.json は変わらない */ }
#[test] fn merge_requires_yes() {}
```

- [ ] **Step 2〜4**: 失敗を確かめる → 実装 → `nix develop -c cargo test`
- [ ] **Step 5: Commit** — `git commit -m "feat: Review proposals from the command line"`

### Task 5: 失敗の一覧とやり直し (`--failed`、`--retry`、`--retry-all`、`--discard`)

**Files:** Modify `src/cli.rs`; Test `tests/cli_agent.rs` (サーバーなし。failed/ に Entry を置いて試す)

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test] fn failed_json_lists_entries() { /* job == "embed", attempts, last_error */ }
#[test] fn retry_moves_to_pending() {}
#[test] fn retry_all_moves_all() {}
#[test] fn discard_requires_yes() {}
#[test] fn retry_unknown_is_invalid() { /* Queue の「ありません」を Invalid として返す */ }
```

- [ ] **Step 2〜4**: 失敗を確かめる → 実装 (やり直しのあと `spawn_worker`) → `nix develop -c cargo test`
- [ ] **Step 5: Commit** — `git commit -m "feat: Inspect and retry failed jobs from the command line"`

### Task 6: skill と書き出し

**Files:** Create `skill/SKILL.md`, `src/skill.rs`; Modify `src/cli.rs`, `src/lib.rs`, `flake.nix` (src に `skill/` が入ることを確かめる); Test `tests/cli_agent.rs`

**Interfaces:**
- Produces: `pub fn skill::export(dir: &Path, force: bool) -> Result<(), Error>` (`SKILL.md` を書く。dir に `SKILL.md` があり `force` がなければ `Error::Invalid("<dir> にはすでに skill があります。上書きするなら --force")`)
- CLI: `--skill-export <dir> [--force]`

`skill/SKILL.md` の中身:
- frontmatter: `name: zettelist`、`description:` (zl で Zettelkasten のメモ、inbox、昇格、レビューを扱うときに使う)
- 各操作のコマンドと JSON の形 (上の節と同じ)
- 典型の流れ: `--inbox --json` → `--show` / `--similar` → `--promote` → (worker の提案を待つ) → `--review --json` → `--adopt` / `--reject` → `--merge --yes`
- `--yes` が要る操作、終了コードと `kind` の意味、`transient` は少し待ってやり直してよい
- 方針: 判断や Permanent の本文をエージェントが決めるかは利用者の運用に従う。指示がなければ、昇格、採否、マージ、削除の前に利用者に確かめる

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test] fn skill_export_writes_file() { /* 一時ディレクトリ → SKILL.md の先頭に "name: zettelist" */ }
#[test] fn skill_export_refuses_overwrite() { /* 2 回目は終了コード 2、--force なら 0 */ }
#[test] fn skill_mentions_every_action() { /* include された文字列に --inbox --show --similar --promote --delete --review --adopt --reject --undecide --merge --failed --retry --discard がすべて含まれる */ }
```

- [ ] **Step 2〜4**: 失敗を確かめる → 実装 → `nix develop -c cargo test` と `nix build` (nix のビルドに `skill/` が含まれ、埋め込みが通ること)
- [ ] **Step 5: Commit** — `git commit -m "feat: Embed and export the agent skill"`

### Task 7: 運用文書と設計書の更新

**Files:**
- Modify: `~/ghq/github.com/reinmuth66/zettelkasten/docs/superpowers/specs/2026-11-19-zettelkasten-design.md` (「他のデータとの関係」の「エージェントには Fleeting までしか書かせない」を「道具は制限しない。エージェントに何をさせるかは運用で決める。この利用者は Fleeting まで」に)
- Modify: `docs/superpowers/specs/2026-11-19-zettelist-design.md` (「コマンドの形」に操作のオプションの一覧と、設計書 `2026-11-20-zettelist-agent-cli-design.md` への参照を足す)

- [ ] **Step 1: 両方を直す**
- [ ] **Step 2: Commit (2 つのリポジトリで別々に)** — 運用側 `docs: Leave agent permissions to operation`、zettelist 側 `docs: Link agent CLI design`

### Task 8: 手元のサーバーで通して確かめる (計画 2 の Task 12 を置き換える)

利用者の承認を得てから行う。`~/.local/share/zettelist/graph/` にグラフを作り、`--bind 127.0.0.1:8080` でサーバーを起動し、次を通す。

1. `zl "小さいノートはつなぎやすい"` と `zl -s 978-4-480-06899-8 -p p.42 "箇条書きは考えを止める"`
2. `zl --inbox --json`、`zl --similar <slug> --json`
3. `zl --promote <slug> --title "..." --body "..."` を 2 回 (話題の近いもの)
4. `zl --review --json` に提案が出る → `zl --adopt <key>` → `zl --merge --yes`
5. `zl --backup` (backup.repo を設定している場合)

結果、かかった時間、気づいた問題を `docs/superpowers/notes/2026-11-20-core-smoke.md` に書いて commit する (`docs: Record core smoke test`)。

## このあと

- 計画 4 (TUI): この計画の操作と同じ関数を画面から呼ぶ
- 計画 5 (出典の登録)
