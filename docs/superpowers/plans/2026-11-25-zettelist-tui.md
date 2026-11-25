# zettelist TUI (計画 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `zl` だけで起動したときに、inbox、書く欄、昇格、リンクと Hub のレビュー、失敗の一覧を持つ TUI を開く。

**Architecture:** 画面の状態とキーの処理 (`tui::app::App`) を、描画 (`tui::view`) と端末 (`tui::run`) から分ける。データの出し入れは `tui::store::Store` trait 越しにし、本物は `Ctx` で、テストは偽物で動かす。CLI と TUI が同じ処理を呼ぶよう、`src/cli.rs` の書き込み、昇格、マージ、やり直し、似ているノートを `src/ops.rs` に移す。

**Tech Stack:** Rust 1.99、ratatui 0.30 (crossterm は `ratatui::crossterm` の再輸出を使う。別に依存を足さない)、unicode-width 0.2、既存の `zettelist` ライブラリ

**Spec:** `docs/superpowers/specs/2026-11-25-zettelist-tui-design.md` (範囲と作り方)。画面、キー、見た目の方針は `docs/superpowers/specs/2026-11-19-zettelist-design.md` の「機能」「キー操作」「見た目」「入力の方法」

## Global Constraints

- 色は端末の 16 色 (ANSI の色) だけ。色だけに頼らず、記号 (`●`、`30+`、`[x]`、`>`) も必ず添える
- 端末の幅が 80 桁未満なら、右の欄を隠して一覧だけにする
- 長い題は表示幅で切り、`…` を付ける。全角文字を半分で切らない。幅は `unicode-width` で数える
- 削除、捨てる、マージだけ確認を 1 回はさむ (`y` で実行、それ以外で中止)
- その画面や提案で使えないキーは、押しても何も起きない (メッセージも出さない)。画面の下のキーの案内は、使えるキーだけを出す
- 入力欄の中では文字の入力を優先し、操作は `Enter`、`Esc`、`Tab`、`Shift-Tab`、`←` `→` `Ctrl-a` `Ctrl-e` `Ctrl-w` `Ctrl-u`、`Backspace` だけ
- 複数行になるもの (昇格の Permanent の本文) だけ `$EDITOR` を使う。`$EDITOR` がなければ `vi`
- 既存の CLI の標準出力、標準エラー、終了コードは変えない。例外は `zl` を引数なしで起動したときだけ (Task 4)
- `review.json` の書き込みは `review::decide` などの、自分で `review.lock` を取る関数を通す。昇格、マージ、やり直しのあとは worker を起動する (`ops` が受け持つ)
- TUI の中では標準出力にも標準エラーにも書かない (画面が壊れる)。警告とエラーは画面の下の 1 行に出す
- テストは `nix develop -c cargo test`。サーバーの要るテストは `tests/common` の `TestServer`。TUI の状態と描画のテストは `src/tui/` の中の `#[cfg(test)]` で、偽の `Store` と `ratatui::backend::TestBackend` を使い、サーバーも端末も使わない
- 文書とメッセージは日本語、絵文字なし。commit に Claude の関与を書かない。`sed -i` と Python は使わない。リポジトリ全体の `cargo fmt` はしない

## Review Focus

1. omnigraph-server が止まっている (起動時、使っている途中) → TUI は落ちずに開き、画面の下に原因を出す。キーは効き続け、`q` で端末を元に戻して終わる (Task 4 の `server_down_shows_message_and_keeps_running`)
2. 全角文字の本文と入力 → 一覧の行は表示幅で切れ、入力欄のカーソルは全角 1 文字で 2 桁進む。横に流れても全角文字を半分で切らない (Task 3 のテスト)
3. エディタで何も書かずに閉じる、`$EDITOR` が起動できない、0 以外で終わる → 昇格は取り消され、何も書かれず、画面の下に理由が出る (Task 6 の `editor_failure_cancels_promotion`、`placeholder_only_cancels`)
4. 一覧を出したあとで提案が消えた (worker の提案の書き直しや、別の `zl --merge`) → 判断は `Invalid` で返る。TUI は理由を出して提案を読み直し、落ちない (Task 7 の `vanished_proposal_reloads`)
5. 端末を極端に小さくする (幅 10、高さ 3 など) → 描画で panic しない (Task 4 の `tiny_terminal_does_not_panic`)

---

## File Structure

| ファイル | 役割 |
|---|---|
| `src/ops.rs` (新規) | CLI と TUI が呼ぶ操作。警告は表示せずに返す |
| `src/cli.rs` | `ops` を呼ぶように直す。`Action::Tui` で `tui::run` を呼ぶ |
| `src/notes.rs` | `inbox_embeddings`、`sources` を足す。`NoteKind` に `Eq` と `Hash` |
| `src/review.rs` | `hubs`、`rehub`、`unrehub` を足す |
| `src/source_ref.rs` | `source_url`、`kind_hint` を足す |
| `src/queue.rs` | `Job::name` を足す (CLI の `job_name` を移す) |
| `src/config.rs` | `theme` と `keys` を生の表で持つ |
| `src/tui/mod.rs` | `run`: 端末の用意と後始末、イベントの繰り返し、エディタとブラウザの起動 |
| `src/tui/store.rs` | `Store` trait、`CtxStore`、テスト用の `FakeStore` |
| `src/tui/app.rs` | `App`: 画面の状態とキーの処理 |
| `src/tui/view.rs` | `draw`: `App` を描く |
| `src/tui/input.rs` | 1 行の入力欄 `LineInput` |
| `src/tui/text.rs` | 表示幅で切る、日付の短い形 |
| `src/tui/keys.rs` | `Action`、`Keymap` (既定値と `[keys]` の読み込み) |
| `src/tui/theme.rs` | `Theme` (既定値と `[theme]` の読み込み) |
| `src/tui/draft.rs` | 昇格の下書きを作る、読む |

---

### Task 1: 操作を `src/ops.rs` に移す

**Files:**
- Create: `src/ops.rs`
- Modify: `src/cli.rs`, `src/lib.rs`, `src/queue.rs`, `src/notes.rs:5` (derive)
- Test: 既存の `tests/cli.rs`、`tests/cli_agent.rs` がそのまま通ること

**Interfaces:**
- Produces:
  - `pub struct Done<T> { pub value: T, pub queue_warning: Option<String>, pub worker_warning: Option<String> }`
  - `pub enum WriteOutcome { Saved(String), Queued(Error) }` (`Saved` は slug。`Queued` は書けなかった理由)
  - `pub fn start_worker(ctx: &Ctx) -> Option<String>` (失敗したら `"worker を起動できません: {e}"`)
  - `pub fn write(ctx: &Ctx, note: NewNote) -> Result<Done<WriteOutcome>, Error>`
  - `pub fn promote(ctx: &Ctx, from: &[(NoteKind, String)], title: &str, body: &str) -> Result<Done<String>, Error>`
  - `pub fn merge(ctx: &Ctx) -> Result<Done<Summary>, Error>`
  - `pub fn retry(ctx: &Ctx, id: &str) -> Result<Done<()>, Error>`、`pub fn retry_all(ctx: &Ctx) -> Result<Done<usize>, Error>`
  - `pub struct Similar { pub inbox: Vec<Near>, pub permanent: Vec<Near> }`、`pub fn similar(ctx: &Ctx, slug: &str) -> Result<Similar, Error>`
  - `pub fn similar_permanent(ctx: &Ctx, kind: NoteKind, slug: &str) -> Result<Vec<Near>, Error>` (埋め込みがまだなら空。`inbox_similarity` 以上、上位 3、自分を除く)
  - `pub fn Job::name(&self) -> &'static str` (`"write_note" | "embed" | "promote" | "backup"`)
  - `NoteKind` に `Eq, Hash`

移すだけで、振る舞いは変えない。`write` の `queue_warning` は `"埋め込みの登録に失敗しました: {e}"`、`promote` の `queue_warning` は `"昇格しましたが、リンク提案を待ち行列に積めませんでした: {e}"`。CLI は今と同じ形で出す: `write` の警告は人向けの文言のまま、`promote` の警告は `--json` なら `{"warning": ..., "slug": ...}`、`worker_warning` はどちらも人向けの文言のまま (skill/SKILL.md の 75 行目と 135 行目の約束)。

- [ ] **Step 1: 既存のテストが通ることを確かめる** — Run: `nix develop -c cargo test` / Expected: PASS (移す前の基準)
- [ ] **Step 2: `src/ops.rs` を作り、`src/cli.rs` の `write`、`promote_notes`、`merge`、`retry`、`retry_all`、`similar`、`start_worker`、`job_name`、`count_notes`、`kind_type` の中身を移す**。CLI の関数は `ops` を呼んで出力するだけにする
- [ ] **Step 3: 通ることを確かめる** — Run: `nix develop -c cargo test` / Expected: PASS、テストの変更なし
- [ ] **Step 4: Commit** — `git commit -m "refactor: Move shared operations into ops module"`

### Task 2: TUI が読む関数と、Hub の選び直し

**Files:**
- Modify: `src/notes.rs`, `src/review.rs`, `src/source_ref.rs`
- Test: `tests/notes.rs`, `tests/review.rs`, `src/source_ref.rs` の `#[cfg(test)]`

**Interfaces:**
- Consumes: `review::list_proposals`、`review::lock_review`、`propose::ensure_review`
- Produces:
  - `pub fn notes::inbox_embeddings(og: &Omnigraph) -> Result<HashMap<(NoteKind, String), Vec<f32>>, Error>` (inbox の Fleeting と Literature のうち、埋め込みのあるもの)
  - `pub struct SourceSummary { pub slug: String, pub kind: String }`、`pub fn notes::sources(og: &Omnigraph) -> Result<Vec<SourceSummary>, Error>` (slug が空でない Source を slug の順に)
  - `pub struct HubSummary { pub slug: String, pub title: String }`、`pub fn review::hubs(og: &Omnigraph) -> Result<Vec<HubSummary>, Error>` (main の Hub を題名の順に)
  - `pub struct Rehubbed { pub old_key: String, pub new_key: String, pub created: bool, pub previous: Option<Decision> }`
  - `pub fn review::rehub(og: &Omnigraph, state_path: &Path, lock_path: &Path, key: &str, hub: &str) -> Result<Rehubbed, Error>`
  - `pub fn review::unrehub(og: &Omnigraph, state_path: &Path, lock_path: &Path, r: &Rehubbed) -> Result<(), Error>`
  - `pub fn source_ref::source_url(id: &str) -> Option<String>`
  - `pub fn source_ref::kind_hint(input: &str) -> Option<&'static str>` (`"book" | "paper" | "article" | "video" | "talk"`)

`rehub` は `review.lock` を取ってから: `key` が `list_proposals` の Hub の提案でなければ `Invalid`。`hub` が main になければ `Invalid`、元と同じ Hub なら `Invalid`、`Includes(hub, note)` が main にあれば `Invalid("すでにその Hub に入っています")`。提案にまだなければ `ensure_review` のあと review に書き (`created = true`)、元の鍵を `Reject`、新しい鍵を `Adopt { reason: None }` にして `review.json` に保存する。`previous` は元の鍵の判断。`unrehub` は `created` なら新しい提案を review から消し、新しい鍵の判断を外し、元の鍵を `previous` に戻す。

`source_url`: `url:<URL>` は URL、`doi:<DOI>` は `https://doi.org/<DOI>`、`arxiv:<ID>` は `https://arxiv.org/abs/<ID>`、`isbn:<ISBN>` は `https://books.google.com/books?vid=ISBN<ISBN>` (書誌検索ページは builder が決める項目。Google Books の ISBN 指定にする)、ほかは `None`。

`kind_hint`: 位置の欄の書き方の例を選ぶための推測。`SourceRef::parse` で ISBN は `book`、DOI と arXiv は `paper`、URL は youtube.com / youtu.be なら `video`、speakerdeck.com なら `talk`、それ以外は `article`。スラッグは `None` (呼ぶ側が `sources` の `kind` を使う)。

- [ ] **Step 1: 失敗するテストを書く**

```rust
// tests/notes.rs
#[test] fn inbox_embeddings_skip_promoted_and_unembedded() {
    // F1 (埋め込みあり)、F2 (なし)、F3 (埋め込みあり、昇格済み) → キーは {(Fleeting, F1)} だけ
}
#[test] fn sources_list_registered_slugs_only() {
    // slug "ahrens2017" (book) と仮の Source (slug 空) → [SourceSummary { slug: "ahrens2017", kind: "book" }]
}
// tests/review.rs
#[test] fn rehub_adopts_new_hub_and_rejects_old() {
    // Hub h1, h2、提案 hub:h1:p → rehub(.., "hub:h1:p", "h2") → created == true、
    // list_proposals に hub:h2:p がある、decisions["hub:h1:p"] == Reject、decisions["hub:h2:p"] == Adopt { reason: None }
    // apply のあと main の Includes は (h2, p) だけ
}
#[test] fn unrehub_restores_previous_state() {
    // rehub のあと unrehub → list_proposals に hub:h2:p がない、decisions は rehub の前と同じ
}
#[test] fn rehub_rejects_unknown_hub_and_same_hub() { /* "nosuch" と "h1" はどちらも Err(Invalid)、review.json は変わらない */ }
#[test] fn hubs_are_sorted_by_title() {}
// src/source_ref.rs
#[test] fn source_url_for_each_id_kind() {
    // "doi:10.1145/x" → "https://doi.org/10.1145/x"、"arxiv:2401.01234" → "https://arxiv.org/abs/2401.01234"、
    // "isbn:9784000000000" → "https://books.google.com/books?vid=ISBN9784000000000"、"url:https://a.example/b" → "https://a.example/b"、"x" → None
}
#[test] fn kind_hint_from_input() {
    // "978-4-00-000000-0" → Some("book")、"10.1145/x" → Some("paper")、"https://youtu.be/abc" → Some("video")、
    // "https://speakerdeck.com/a/b" → Some("talk")、"https://a.example/" → Some("article")、"ahrens2017" → None
}
```

- [ ] **Step 2: 失敗を確かめる** — Run: `nix develop -c cargo test --test notes --test review && nix develop -c cargo test --lib source_ref` / Expected: FAIL (関数がない)
- [ ] **Step 3: 上の関数を実装する**。`inbox_embeddings` は `inbox` と同じ「昇格していない」条件のクエリに `$n.embedding` を足し、null の行を飛ばす。`rehub` と `unrehub` の review からの削除は既存の `delete_from_review` を使う
- [ ] **Step 4: 通ることを確かめる** — Run: `nix develop -c cargo test` / Expected: PASS
- [ ] **Step 5: Commit** — `git commit -m "feat: Add reads and hub reassignment for the TUI"`

### Task 3: 1 行の入力欄と表示幅

**Files:**
- Create: `src/tui/mod.rs` (この段階では `pub mod input; pub mod text;` だけ), `src/tui/input.rs`, `src/tui/text.rs`
- Modify: `src/lib.rs` (`pub mod tui;`), `Cargo.toml` (`ratatui = "0.30"`, `unicode-width = "0.2"`)
- Test: 各ファイルの `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `pub struct LineInput` (`text: String` と、文字単位のカーソル位置)
  - `LineInput::new(text: &str) -> LineInput` (カーソルは末尾)、`fn text(&self) -> &str`、`fn handle(&mut self, key: KeyEvent) -> bool` (文字と編集キーを処理したら true。`Enter`、`Esc`、`Tab`、`BackTab` は false を返して何もしない)
  - `fn view(&self, width: u16) -> (String, u16)`: 幅 `width` に収まる見える部分と、その中のカーソルの桁。カーソルが常に見えるよう横に流す
  - `pub fn text::truncate(s: &str, width: usize) -> String`: 表示幅が `width` を超えたら、`…` を含めて `width` 以内に切る。全角文字を半分で切らない
  - `pub fn text::short_date(slug: &str) -> &str`: ノートの slug (`YYYYMMDDHHMMSS`) から `MMDD`。14 桁でなければそのまま

編集キー: `←` `→`、`Ctrl-a` (行頭)、`Ctrl-e` (行末)、`Ctrl-w` (カーソルの前の 1 語を消す。語は空白で区切る)、`Ctrl-u` (行頭まで消す)、`Backspace`。

- [ ] **Step 1: 失敗するテストを書く**

```rust
// input.rs
#[test] fn typing_inserts_at_cursor() { /* "ab" で ← のあと "x" → "axb" */ }
#[test] fn ctrl_w_deletes_previous_word() { /* "foo bar" → "foo " */ }
#[test] fn ctrl_u_deletes_to_line_start() { /* "foo bar" で ← 3 回のあと Ctrl-u → "bar" */ }
#[test] fn enter_esc_tab_are_not_consumed() { /* handle が false、text は変わらない */ }
#[test] fn wide_chars_move_cursor_two_columns() { /* "あい" の view(20) → ("あい", 4) */ }
#[test] fn view_scrolls_to_keep_cursor_visible() {
    // "あいうえおかきくけこ" (幅 20) の view(9) → 見える部分の幅 <= 9、末尾が "こ"、カーソルの桁 <= 8
}
// text.rs
#[test] fn truncate_ascii() { /* truncate("abcdef", 4) == "abc…" */ }
#[test] fn truncate_does_not_split_wide_char() { /* truncate("あいう", 5) == "あい…" (幅 5)、truncate("あいう", 4) == "あ…" */ }
#[test] fn truncate_keeps_short_text() { /* truncate("あい", 4) == "あい" */ }
#[test] fn short_date_from_slug() { /* short_date("20261118093000") == "1118"、short_date("x") == "x" */ }
```

- [ ] **Step 2: 失敗を確かめる** — Run: `nix develop -c cargo test --lib tui::` / Expected: FAIL
- [ ] **Step 3: `LineInput`、`truncate`、`short_date` を実装する** (幅は `unicode_width::UnicodeWidthChar`)
- [ ] **Step 4: 通ることを確かめる** — Run: `nix develop -c cargo test --lib tui::` / Expected: PASS
- [ ] **Step 5: Commit** — `git commit -m "feat: Add line input and width helpers for the TUI"`

### Task 4: TUI の土台と inbox の一覧

**Files:**
- Create: `src/tui/store.rs`, `src/tui/app.rs`, `src/tui/view.rs`, `src/tui/keys.rs`, `src/tui/theme.rs`
- Modify: `src/tui/mod.rs`, `src/cli.rs` (`Action::Tui`), `tests/cli.rs:66-72`, `tests/cli_agent.rs:1086-1094`
- Test: `src/tui/app.rs` と `src/tui/view.rs` の `#[cfg(test)]`

**Interfaces:**
- Consumes: Task 1 の `ops`、Task 2 の関数、Task 3 の `LineInput`、`truncate`、`short_date`
- Produces:
  - `pub trait Store` (メソッドは下の一覧)。`pub struct CtxStore(pub Ctx)` が `ops` と既存の関数で実装する。`#[cfg(test)] pub struct FakeStore` は、メモリ上の inbox、提案、判断、失敗の一覧を持ち、呼ばれた操作を `pub calls: Vec<String>` に記録する。`pub fail_with: Option<Error>` が入っていると、どのメソッドもそのエラーを返す
  - `pub struct Settings { pub inbox_similarity: f32, pub locator: bool, pub theme: Theme, pub keys: Keymap }`
  - `pub struct App` と `App::new(store: Box<dyn Store>, settings: Settings) -> App` (作るときに inbox を読む。失敗は `message` に入れる)
  - `pub fn App::on_key(&mut self, key: KeyEvent) -> Option<Effect>`
  - `pub enum Effect { Quit, Edit { draft: String }, Open(String) }`
  - `pub fn App::message(&self) -> Option<&str>` (画面の下の 1 行。次のキーで消える)
  - `pub enum Screen { Inbox, Review, Failed }`
  - `pub enum Action { Down, Up, Top, Bottom, ScrollDown, ScrollUp, Help, Quit, Write, Toggle, ClearSelection, Delete, ToReview, ToFailed, OpenSource, Adopt, AdoptWithReason, Reject, Change, Next, Prev, Undo, Redo, Merge, Back, Retry, Discard, RetryAll }`
  - `pub struct Keymap`、`Keymap::default()`、`fn resolve(&self, screen: Screen, pending: Option<KeyEvent>, key: KeyEvent) -> Resolved` (`enum Resolved { Action(Action), Pending, Nothing }`)、`fn label(&self, screen: Screen, action: Action) -> Option<String>` (案内に出す最初のキー。`"j"`、`"space"`、`"Ctrl-d"`、`"gg"` の形)
  - `pub struct Theme` (項目は `cursor`、`selected`、`similar`、`old`、`dim`、`header`、`error`、`adopted`、`rejected`。値は `ratatui::style::Color` の 16 色)、`Theme::default()`
  - `pub fn view::draw(frame: &mut Frame, app: &App)`
  - `pub fn tui::run(ctx: Ctx) -> Result<(), Error>`

`Store` のメソッド (すべて `&mut self`、`Result<_, Error>` を返す):

```
inbox() -> Vec<InboxItem>
inbox_embeddings() -> HashMap<(NoteKind, String), Vec<f32>>
similar_permanent(kind: NoteKind, slug: &str) -> Vec<Near>
sources() -> Vec<SourceSummary>
write(note: NewNote) -> Done<WriteOutcome>
promote(from: &[(NoteKind, String)], title: &str, body: &str) -> Done<String>
delete(notes: &[(NoteKind, String)]) -> ()
proposals() -> (Vec<Proposal>, ReviewState)
note(slug: &str) -> Note
hubs() -> Vec<HubSummary>
decide(key: &str, decision: Option<Decision>) -> ()
rehub(key: &str, hub: &str) -> Rehubbed
unrehub(r: &Rehubbed) -> ()
merge() -> Done<Summary>
failed() -> Vec<Entry>
retry(id: &str) -> Done<()>
retry_all() -> Done<usize>
discard(id: &str) -> ()
```

既定のキー (どの画面でも共通: `j` `↓` Down、`k` `↑` Up、`gg` Top、`G` Bottom、`Ctrl-d` ScrollDown、`Ctrl-u` ScrollUp、`?` Help、`q` Quit):

| 画面 | キー |
|---|---|
| inbox | `i` Write、`space` Toggle、`Esc` ClearSelection、`d` Delete、`r` ToReview、`f` ToFailed、`o` OpenSource |
| review | `a` Adopt、`A` AdoptWithReason、`d` Reject、`c` Change、`o` OpenSource、`n` Next、`N` Prev、`u` Undo、`Ctrl-r` Redo、`m` Merge、`Esc` Back |
| failed | `Enter` Retry、`x` Discard、`R` RetryAll、`Esc` Back |

この Task で動かすのは、共通のキーと inbox の `space` `Esc` `r` `f` (画面を切り替えるだけ。中身は Task 7 と 8)。ほかのキーは各 Task で足す。それまでは押しても何も起きず、案内にも出さない。

inbox の画面 (本体の設計の「inbox (最初の画面)」の図のとおり):
- 上の帯: `inbox N 件 (30日超 M)     review K 件     失敗 F 件`。K は `proposals()` の件数、F は `failed()` の件数
- 左の一覧: 1 行に `>` (カーソル)、`[x]` / `[ ]` (選択)、`short_date(slug)`、本文の 1 行目 (`truncate`)、種類 `F` / `L`、`30+` (30 日超)、`●` (似ている)。古い順
- 似ている印: カーソルのノートの埋め込みと、ほかの inbox のノートの埋め込みのコサイン類似度が `inbox_similarity` 以上なら `●` と `theme.similar` の色。どちらかに埋め込みがなければ印なし。埋め込みは inbox を読むときに `inbox_embeddings()` でまとめて読む
- 右の欄: カーソルのノートの全文 (折り返す)。Literature なら `出典 <source_slug か source_id>  <locator>`。その下に `似ている Permanent` と `similar_permanent()` の上位 3 件 (題名と近さ)。結果は slug ごとに覚え、inbox を読み直したら捨てる。`Ctrl-d` / `Ctrl-u` で右の欄を送る
- 幅 80 未満: 右の欄を出さない
- inbox が空: 一覧に `整理されていないメモはありません`
- 下の帯: 使えるキーの案内 (`Keymap::label` から作る)。`message` があればその上の 1 行に出す
- inbox を読み直すのは、起動時、ほかの画面から戻ったとき、書き込み、昇格、削除、マージのあと

`tui::run`: `CtxStore` と `Settings` (Task 9 までは `Theme::default()` と `Keymap::default()`) で `App` を作り、`ratatui::init()` で端末を用意し、描画とキーの読み込みを繰り返す。`Effect::Quit` で抜け、どの道でも `ratatui::restore()` を呼ぶ。`Edit` と `Open` は Task 6 で実装する (それまでは来ない)。

`cli.rs` の `Action::Tui`: `--json` なら `Error::Invalid("--json では TUI を起動できません。操作のオプションか本文を渡します")`。標準入力か標準出力が端末でなければ (`std::io::IsTerminal`) `Error::Invalid("TUI は端末で起動します")`。どちらでもなければ `Ctx::from_env()` のあと `tui::run`。

- [ ] **Step 1: 古い文言を見ている 2 つのテストを直す**。`tests/cli.rs` の `no_body_shows_tui_notice` を `no_body_without_terminal_is_an_error` にし、終了コード 2 と、標準エラーに `TUI は端末で起動します` を確かめる。`tests/cli_agent.rs` の `json_without_action_is_an_error_not_success` は `TUI を起動できません` を含むことを確かめる
- [ ] **Step 2: 失敗するテストを書く** (`FakeStore` に inbox を 3 件入れて `App::new`)

```rust
// app.rs
#[test] fn j_k_move_cursor_and_stop_at_ends() { /* k を押しても 0 のまま、j 3 回で 2 */ }
#[test] fn gg_and_g_jump() { /* G で 2、g g で 0。g のあと j なら Top にならず Down */ }
#[test] fn space_toggles_and_esc_clears_selection() {}
#[test] fn q_on_inbox_quits() { /* on_key('q') == Some(Effect::Quit) */ }
#[test] fn q_on_review_returns_to_inbox() { /* r のあと q → screen == Inbox、Effect なし */ }
#[test] fn similar_mark_follows_cursor() {
    // 埋め込み A=(0:1)、B=(0:1)、C=(1:1)、inbox_similarity 0.5 → カーソルが A のとき B だけ似ている。C に動くと A も B も似ていない
}
#[test] fn server_down_shows_message_and_keeps_running() {
    // fail_with = Transient("届きません") → App::new は落ちず、message が "届きません" を含む。j と q が効く
}
#[test] fn unbound_key_does_nothing() { /* 'z' → Effect なし、状態も message も変わらない */ }
// view.rs (TestBackend に draw して、バッファの文字列で確かめる)
#[test] fn inbox_shows_counts_and_marks() {
    // 上の帯に "inbox 3 件" と "review 0 件" と "失敗 0 件"。30 日超のノートの行に "30+"、選んだ行に "[x]"、カーソルの行に ">"
}
#[test] fn narrow_terminal_hides_right_pane() { /* 幅 79 では本文の全文の欄の文字が出ない。幅 80 では出る */ }
#[test] fn footer_lists_only_usable_keys() { /* inbox の案内に "space" と "r" があり、"m" (マージ) と "a" はない */ }
#[test] fn empty_inbox_says_so() {}
#[test] fn tiny_terminal_does_not_panic() { /* 10x3、1x1 で draw しても panic しない */ }
```

- [ ] **Step 3: 失敗を確かめる** — Run: `nix develop -c cargo test --lib tui::` / Expected: FAIL
- [ ] **Step 4: `Store`、`CtxStore`、`FakeStore`、`Keymap::default`、`Theme::default`、`App` の inbox の部分、`draw`、`run`、`Action::Tui` を実装する**。左右の欄の比率と色の割り当ては仮に決めてよい (Task 9 のあとで利用者と決める)
- [ ] **Step 5: 通ることを確かめる** — Run: `nix develop -c cargo test` / Expected: PASS
- [ ] **Step 6: 手で起動して確かめる** — Run: `nix develop -c cargo run` (omnigraph-server が動いている端末で) / Expected: inbox が出て、`j` `k` で動き、`q` で端末が元に戻る。サーバーを止めて起動すると、画面の下に原因が出る
- [ ] **Step 7: Commit** — `git commit -m "feat: Open the inbox in the TUI"`

### Task 5: 書く欄

**Files:**
- Modify: `src/tui/app.rs`, `src/tui/view.rs`
- Test: `src/tui/app.rs`, `src/tui/view.rs` の `#[cfg(test)]`

**Interfaces:**
- Consumes: `Store::write`、`Store::sources`、`LineInput`、`source_ref::kind_hint`
- Produces: `App` の書く欄の状態 (本文、出典、`locator` のときだけ位置)

本体の設計の「書く」の TUI の項のとおり:
- inbox で何も選んでいないときの `i` で、画面の下に欄が出る。`Tab` / `Shift-Tab` で欄を移る (端で回る)
- `Enter` で保存。本文が空 (空白だけ) なら何もしない。出典が空なら `source: None` (Fleeting)、入っていれば `Some` (Literature)。位置は出典があり、空でないときだけ。`created` は `Local::now()`
- 保存の結果: `Saved(slug)` は `保存しました <slug>`、`Queued(e)` は `Transient` なら `omnigraph-server に届かないので待ち行列に保存しました`、ほかは `書けなかったので待ち行列に保存しました: <e>` を `message` に出し、inbox を読み直す。`Err(e)` (入力の誤りなど) は欄を閉じずに `message` に出す。警告 (`queue_warning`、`worker_warning`) があれば `message` に足す
- `Esc` でやめる (何も書かない)
- 出典の欄の補完: 入力が空でなく、`sources()` の slug に前方一致するものがあれば、最初の 1 つの残りを `theme.dim` で入力の後ろに出す。カーソルが末尾にあるとき `→` か `Ctrl-e` で確定する (zsh-autosuggestions と同じ。確定のキーは builder が決める項目)。`sources()` は欄を開いたときに 1 回読む
- 位置の欄の薄い例: 出典が登録済みの slug ならその `kind`、そうでなければ `kind_hint(出典)` で選ぶ。book `p.42 / ch.3`、paper `sec.3.2 / fig.4`、article `見出し`、video `12:30`、talk `slide 14`。分からなければ出さない。位置の欄に文字があれば出さない

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test] fn write_saves_fleeting() {
    // i、"箇条書きは考えを止める" を打つ、Enter → FakeStore.calls に write、note.source == None、message に "保存しました"
}
#[test] fn write_with_source_saves_literature() { /* i、本文、Tab、"ahrens2017"、Enter → source == Some("ahrens2017")、locator == None */ }
#[test] fn write_empty_body_does_nothing() { /* i、Enter → write は呼ばれず、欄は開いたまま */ }
#[test] fn write_esc_cancels() {}
#[test] fn source_completion_accepts_with_right_arrow() { /* sources = [ahrens2017]、出典の欄で "ahr"、→ → 出典の欄は "ahrens2017" */ }
#[test] fn locator_field_only_when_enabled() { /* locator=false では Tab が本文と出典の 2 つを回る。true では 3 つ */ }
#[test] fn queued_write_reports_queue() { /* write が Queued(Transient) → message に "待ち行列に保存しました" */ }
#[test] fn i_with_selection_does_not_open_write_form() { /* space のあと i → 書く欄ではない (Task 6 の題の欄) */ }
// view.rs
#[test] fn write_form_shows_fields_and_hint() { /* locator=true、出典 "978-4-00-000000-0" → 位置の欄に "p.42" が薄く出る */ }
```

- [ ] **Step 2: 失敗を確かめる** — Run: `nix develop -c cargo test --lib tui::` / Expected: FAIL
- [ ] **Step 3: 書く欄を実装する**
- [ ] **Step 4: 通ることを確かめる** — Run: `nix develop -c cargo test --lib tui::` / Expected: PASS
- [ ] **Step 5: Commit** — `git commit -m "feat: Write notes from the TUI"`

### Task 6: 昇格、削除、出典のページを開く

**Files:**
- Create: `src/tui/draft.rs`
- Modify: `src/tui/app.rs`, `src/tui/view.rs`, `src/tui/mod.rs`
- Test: `src/tui/draft.rs`, `src/tui/app.rs` の `#[cfg(test)]`

**Interfaces:**
- Consumes: `Store::promote`、`Store::delete`、`source_ref::source_url`
- Produces:
  - `pub fn draft::build(items: &[InboxItem]) -> String`
  - `pub fn draft::parse(text: &str) -> Option<String>` (本文。何も書いていなければ `None`)
  - `pub fn App::finish_promote(&mut self, body: Result<String, String>)` (`Ok` はエディタで保存した下書きの全文、`Err` はエディタが起動できなかった、0 以外で終わった理由)
  - `tui::run` の `Effect::Edit` と `Effect::Open` の処理

昇格 (本体の設計の「昇格」のとおり):
1. 1 つ以上選んで `i` → 画面の下に題の 1 行の入力欄。選んだノートは一覧に見えたまま。空のまま `Enter`、または `Esc` で取り消す
2. 題を `Enter` → `Effect::Edit { draft: draft::build(選んだもの) }`
3. `run` は `ratatui::restore()`、下書きを一時ファイル (`tempfile`、拡張子 `.md`) に書き、`$EDITOR` (なければ `vi`。空白で区切って先頭をコマンド、残りを引数にする) をそのファイルで起動して待つ。終わったら `ratatui::init()` で戻し、`finish_promote` に結果を渡す
4. `finish_promote`: `Err(理由)` か `parse` が `None` なら、何もせず `昇格を取り消しました` (理由があれば付ける) を `message` に出す。本文があれば `Store::promote(選んだもの, 題, 本文)`。成功なら `昇格しました <slug>` と警告を出し、選択を外して inbox を読み直す。失敗ならその理由を出す (選択は残す)

下書きの形:

```
(ここに Permanent を書く)

# ---- この行から下は保存時に消える ----
# [1118 F] 小さいノートはつなぎやすい。理由は…
# [1117 L ahrens2017 p.42] 箇条書きは…
```

- 1 行目は目印の文。`[MMDD 種類 出典 位置]` の出典は `source_slug`、なければ `source_id`。位置がなければ省く。本文は改行を空白に置き換えた全文 (切らない。読み返しながら書くため)
- `parse`: 目印の行 (`# ---- この行から下は保存時に消える ----`) から下を捨て、目印の文 `(ここに Permanent を書く)` だけの行を除き、前後の空白を除く。空なら `None`。目印の行がなければ全文を本文として扱う

削除: `d` → 選んだもの、なければカーソルの 1 件について `<N> 件消します。よろしいですか [y/N]` を出す。`y` で `Store::delete`、ほかのキーで中止。消したら選択を外して inbox を読み直す。inbox が空なら `d` は何もしない。

出典を開く: `o` はカーソルのノートが Literature で、`source_url(source_id)` があるときだけ `Effect::Open(url)`。`run` は macOS では `open`、ほかでは `xdg-open` を、標準出力と標準エラーを捨てて起動し、待たない。起動できなければ `message` に出す。

- [ ] **Step 1: 失敗するテストを書く**

```rust
// draft.rs
#[test] fn build_lists_selected_notes_as_comments() {
    // F (slug 20261118..., 本文 "a\nb") と L (source_slug ahrens2017、locator p.42) →
    // "# [1118 F] a b" と "# [1117 L ahrens2017 p.42] ..." の行がある。1 行目は "(ここに Permanent を書く)"
}
#[test] fn parse_drops_comment_section() { /* "本文\n\n# ---- この行から下は保存時に消える ----\n# x" → Some("本文") */ }
#[test] fn placeholder_only_cancels() { /* build の結果をそのまま parse → None */ }
#[test] fn parse_without_marker_keeps_all() {}
// app.rs
#[test] fn promote_flow_calls_store() {
    // 2 件選ぶ、i、"小さいノート"、Enter → Effect::Edit。finish_promote(Ok("本文\n# ---- この行から下は保存時に消える ----")) →
    // promote(from = 選んだ 2 件, title = "小さいノート", body = "本文")、選択は空、message に "昇格しました"
}
#[test] fn empty_title_cancels_promotion() { /* 選んで i、Enter → Effect なし、promote は呼ばれない */ }
#[test] fn editor_failure_cancels_promotion() { /* finish_promote(Err("vi が見つかりません")) → promote は呼ばれず、message に理由 */ }
#[test] fn delete_asks_and_y_deletes() { /* d → 確認、y → delete(カーソルの 1 件) */ }
#[test] fn delete_other_key_cancels() { /* d のあと n → delete は呼ばれない */ }
#[test] fn open_only_for_literature_with_url() { /* Fleeting で o → Effect なし。source_id "doi:10.1/x" の Literature で o → Open("https://doi.org/10.1/x") */ }
```

- [ ] **Step 2: 失敗を確かめる** — Run: `nix develop -c cargo test --lib tui::` / Expected: FAIL
- [ ] **Step 3: `draft`、昇格、削除、`o`、`run` の `Edit` と `Open` を実装する**
- [ ] **Step 4: 通ることを確かめる** — Run: `nix develop -c cargo test --lib tui::` / Expected: PASS
- [ ] **Step 5: 手で確かめる** — Run: `nix develop -c cargo run` で 1 件選んで昇格する / Expected: エディタが開き、保存して閉じると TUI に戻り、そのノートが inbox から消える。何も書かずに閉じると何も変わらない
- [ ] **Step 6: Commit** — `git commit -m "feat: Promote and delete notes from the TUI"`

### Task 7: レビューの画面

**Files:**
- Modify: `src/tui/app.rs`, `src/tui/view.rs`
- Test: `src/tui/app.rs`, `src/tui/view.rs` の `#[cfg(test)]`

**Interfaces:**
- Consumes: `Store::proposals`、`note`、`hubs`、`decide`、`rehub`、`unrehub`、`merge`
- Produces: `App` のレビューの状態 (提案、カーソル、判断、取り消しとやり直しの履歴)

本体の設計の「レビュー」とキーの表のとおり (出典の列は計画 4):
- `r` で入ったときと、マージのあとに `proposals()` を読む。並びは `list_proposals` のまま (リンクが近い順、次に Hub)
- 上の帯: `review N 件  (links L / hub H)`、リンクなら右端に `(近さ 0.82)`
- リンク: 左に新しいノート (`from`)、右に候補 (`to`) の題名と本文。`note(slug)` で読み、slug ごとに覚える。幅 80 未満なら左だけ
- Hub: 新しいノートと、提案された Hub の題名 (`hubs()` から)
- 判断済みの提案には `採用` / `却下` (理由があれば理由も) を出し、色は `theme.adopted` / `theme.rejected`
- `a`: `decide(key, Adopt { reason: None })` して次へ。`A` (リンクだけ): 画面の下に `理由: ` の入力欄。両方の本文は見えたまま。`Enter` で `Adopt { reason }` (空白を除いて空なら `None`)、`Esc` でやめる
- `d`: `decide(key, Reject)` して次へ
- `c` (Hub だけ): `hubs()` の一覧 (今の Hub を除く) を出し、`j` `k` で選んで `Enter` で `rehub`、`Esc` でやめる。終わったら `proposals()` を読み直し、カーソルは元の提案の位置に置く
- `n` / `N`: 判断せずに次 / 前へ (端で止まる)
- `u`: 直前の判断を取り消す。判断は `decide(key, 前の判断)`、`c` は `unrehub`。`Ctrl-r`: 取り消したものをやり直す (`decide(key, 後の判断)`、`c` は同じ Hub で `rehub` し直す)。新しい判断をしたら、やり直しの履歴は捨てる。カーソルはその提案に戻る
- `m`: `採用 N 件、却下 M 件を main に入れます。よろしいですか [y/N]`。`y` で `merge()`、`マージしました (リンク L 本、ハブ H 件)` と警告を出し、履歴を捨てて読み直す
- `Esc` と `q`: inbox へ (inbox を読み直す)
- 提案が 0 件: `提案はありません` を出し、使えるキーは `Esc` `q` `?` だけ
- `decide`、`rehub` などが `Error::Invalid` を返したら (提案が消えた)、理由を `message` に出して `proposals()` を読み直す
- 案内: Hub の提案では `A` を出さず、リンクの提案では `c` と `o` を出さない (`o` は出典の提案のためのもので、計画 3 では常に出さない)

- [ ] **Step 1: 失敗するテストを書く** (`FakeStore` にリンクの提案 2 件と Hub の提案 1 件)

```rust
#[test] fn adopt_records_and_moves_next() { /* a → decide("link:a:b", Adopt{None})、カーソル 1 */ }
#[test] fn adopt_with_reason() { /* A、"同じ主張"、Enter → Adopt{Some("同じ主張")} */ }
#[test] fn adopt_with_blank_reason_is_none() { /* A、"  "、Enter → Adopt{None} */ }
#[test] fn capital_a_does_nothing_on_hub() { /* Hub の提案で A → 入力欄は出ず、decide も呼ばれない */ }
#[test] fn reject_records_and_moves_next() {}
#[test] fn undo_and_redo_decision() { /* a、u → decide(key, None)、カーソルは 0。Ctrl-r → decide(key, Adopt{None}) */ }
#[test] fn new_decision_clears_redo() { /* a、u、d → Ctrl-r は何もしない */ }
#[test] fn change_hub_calls_rehub_and_undo_calls_unrehub() {}
#[test] fn merge_asks_then_merges() { /* m → 確認、y → merge が呼ばれ、message に "マージしました" */ }
#[test] fn vanished_proposal_reloads() { /* decide が Invalid("提案 'link:a:b' はありません") → message にその文、proposals が読み直される (calls で確かめる) */ }
#[test] fn esc_returns_to_inbox() {}
// view.rs
#[test] fn link_proposal_shows_both_notes_and_similarity() { /* 両方の題名と "近さ 0.82" が出る */ }
#[test] fn footer_hides_keys_not_for_this_proposal() { /* Hub の提案の案内に "A" がない。リンクの提案の案内に "c" がない */ }
#[test] fn no_proposals_message() {}
```

- [ ] **Step 2: 失敗を確かめる** — Run: `nix develop -c cargo test --lib tui::` / Expected: FAIL
- [ ] **Step 3: レビューの画面を実装する**
- [ ] **Step 4: 通ることを確かめる** — Run: `nix develop -c cargo test --lib tui::` / Expected: PASS
- [ ] **Step 5: Commit** — `git commit -m "feat: Review link and hub proposals in the TUI"`

### Task 8: 失敗の一覧とキーの一覧

**Files:**
- Modify: `src/tui/app.rs`, `src/tui/view.rs`
- Test: `src/tui/app.rs`, `src/tui/view.rs` の `#[cfg(test)]`

**Interfaces:**
- Consumes: `Store::failed`、`retry`、`retry_all`、`discard`、`Job::name`、`Keymap::label`
- Produces: `App` の失敗の一覧の状態と、キーの一覧の表示

失敗の一覧 (inbox の `f`):
- 左に 1 行ずつ `Job::name`、回数、`last_error` の 1 行目 (`truncate`)。右の欄に、カーソルのジョブの `last_error` の全文と、ジョブの中身 (ノートの slug か本文の 1 行目)
- `Enter`: `retry(id)`、`x`: `この失敗を捨てます。よろしいですか [y/N]` のあと `discard(id)`、`R`: `retry_all()`。どれもそのあと `failed()` を読み直し、警告があれば出す
- `Esc` と `q`: inbox へ
- 0 件: `失敗したジョブはありません`

キーの一覧 (`?`): 今の画面で使えるキーを、`キー  動作` の形で全部出す (共通のキーと入力欄のキーも含む)。どのキーでも閉じる。動作の名前は本体の設計のキーの表の言葉を使う (`下へ移動`、`選ぶ / 外す`、`理由を書いて採用` など)。

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test] fn f_opens_failed_list() {}
#[test] fn enter_retries_and_reloads() { /* Enter → retry(id)、そのあと failed が呼ばれる */ }
#[test] fn x_asks_then_discards() {}
#[test] fn capital_r_retries_all() {}
#[test] fn help_lists_keys_and_closes_on_any_key() { /* ? → help が開く。'z' → 閉じ、Effect なし */ }
// view.rs
#[test] fn failed_list_shows_job_and_error() { /* "embed" と "1 回目のエラー" が出る */ }
#[test] fn help_on_review_lists_review_keys() { /* "理由を書いて採用" と "A" が出る */ }
```

- [ ] **Step 2: 失敗を確かめる** — Run: `nix develop -c cargo test --lib tui::` / Expected: FAIL
- [ ] **Step 3: 失敗の一覧とキーの一覧を実装する**
- [ ] **Step 4: 通ることを確かめる** — Run: `nix develop -c cargo test --lib tui::` / Expected: PASS
- [ ] **Step 5: Commit** — `git commit -m "feat: Show failed jobs and key help in the TUI"`

### Task 9: `[theme]` と `[keys]`

**Files:**
- Modify: `src/config.rs`, `src/tui/theme.rs`, `src/tui/keys.rs`, `src/tui/mod.rs`, `docs/superpowers/specs/2026-11-19-zettelist-design.md` (「設定」の `[theme]` と `[keys]` の例、「builder に任せること」に決めた形式と ISBN の書誌検索ページの URL を書き足す)
- Test: `src/config.rs`, `src/tui/theme.rs`, `src/tui/keys.rs` の `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `Config` に `pub theme: BTreeMap<String, String>` と `pub keys: BTreeMap<String, BTreeMap<String, Vec<String>>>` (どちらも `#[serde(default)]`)
  - `pub fn Theme::from_config(map: &BTreeMap<String, String>) -> Result<Theme, Error>`
  - `pub fn Keymap::from_config(map: &BTreeMap<String, BTreeMap<String, Vec<String>>>) -> Result<Keymap, Error>`

書き方 (builder が決める項目。この形にする):

```toml
[theme]
similar = "cyan"          # 項目は Task 4 の Theme の項目名
old     = "bright_black"

[keys.common]             # 節は common、inbox、review、failed
down = ["j", "down"]      # 動作の名前は Action の snake_case
top  = ["g g"]            # 2 つのキーの並びは空白で区切る

[keys.review]
adopt_with_reason = ["A"]
```

- 色の名前: `black` `red` `green` `yellow` `blue` `magenta` `cyan` `white` と、それぞれの `bright_` 付き、`default` (端末の色のまま)。ほかは `Error::Invalid("[theme] の <項目> の色 '<値>' は使えません")`。知らない項目も `Invalid`
- キーの書き方: 1 文字 (`d`、`D`、`?`)、名前 (`space` `enter` `esc` `tab` `backtab` `up` `down` `left` `right` `backspace`)、`ctrl-<1 文字>`。並びは 2 つまで
- 書いた動作は、その動作の既定のキーを置き換える。書かない動作は既定のまま
- 知らない節、知らない動作、読めないキー、同じ画面 (共通を含む) で同じキーが 2 つの動作に付く、ある並びの先頭のキーが別の動作に単独で付く (`g` と `g g`)、は `Error::Invalid` で、どこが悪いかを文言に入れる
- `tui::run` は端末を用意する前に読み、誤りがあれば TUI を開かずにその `Invalid` を返す (CLI が終了コード 2 で出す)
- 入力欄の中のキーと確認の `y` は変えられない

- [ ] **Step 1: 失敗するテストを書く**

```rust
// config.rs
#[test] fn theme_and_keys_are_read() { /* "[theme]\nsimilar = \"red\"\n[keys.inbox]\ndelete = [\"D\"]\n" → theme["similar"] == "red"、keys["inbox"]["delete"] == ["D"] */ }
// 既存の unknown_sections_are_ignored は、[theme] x = "red" と [keys] y = "z" を読んでいる。
// [keys] y = "z" は節の形でないので読めなくなる。テストを "[unknown]\nx = 1\n" に直し、名前を unknown_tables_are_ignored にする
// theme.rs
#[test] fn theme_overrides_one_color() { /* similar = "red" → similar は Color::Red、old は既定のまま */ }
#[test] fn theme_rejects_unknown_color_and_item() { /* "pink" と項目 "nosuch" はどちらも Err、文言に項目名が入る */ }
// keys.rs
#[test] fn keys_override_replaces_default() { /* inbox.delete = ["D"] → D で Delete、d は Nothing */ }
#[test] fn keys_two_key_sequence() { /* common.top = ["t t"] → t で Pending、t で Top */ }
#[test] fn keys_reject_conflict() { /* inbox.delete = ["j"] → Err (common の down と重なる) */ }
#[test] fn keys_reject_prefix_conflict() { /* inbox.delete = ["g"] → Err (g g と重なる) */ }
#[test] fn keys_reject_unknown_action_and_key() { /* inbox.nosuch と inbox.delete = ["ctrl-"] はどちらも Err */ }
#[test] fn ctrl_key_parses() { /* review.redo = ["ctrl-y"] → Ctrl-y で Redo */ }
```

- [ ] **Step 2: 失敗を確かめる** — Run: `nix develop -c cargo test --lib` / Expected: FAIL
- [ ] **Step 3: 読み込みと検査を実装し、`tui::run` で使う**
- [ ] **Step 4: 通ることを確かめる** — Run: `nix develop -c cargo test` / Expected: PASS
- [ ] **Step 5: 本体の設計に、決めた `[theme]` と `[keys]` の形式と ISBN の書誌検索ページの URL を書き足す**
- [ ] **Step 6: Commit** — `git commit -m "feat: Configure TUI colors and keys"`

---

## 実装のあと (指揮役と利用者)

見た目を決める: 実際のデータで `zl` を開き、inbox、書く欄、昇格の題の欄、レビュー (リンクと Hub)、失敗の一覧のスクリーンショットを利用者に見せ、色の割り当て、枠の線、左右の欄の比率、上下の帯、確認の出し方を決める (本体の設計の「見た目」)。決まったものは `Theme::default` と `view.rs` を直す小さな変更にする。
