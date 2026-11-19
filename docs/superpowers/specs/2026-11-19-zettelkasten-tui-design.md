# Zettelkasten TUI 設計 (引き継ぎ)

状態: 前提と決定事項の引き継ぎ。画面と操作の設計はこのリポジトリの新しいセッションで行う (superpowers:brainstorming から)。

## 最初に読むもの

- 運用の決まりとスキーマの正本: `~/ghq/github.com/reinmuth66/zettelkasten/docs/superpowers/specs/2026-11-19-zettelkasten-design.md`
  - ノートの種類、slug、出典、リンク付け、レビュー、検索、LLM、保存、スキーマはそこに書いてある。この文書では繰り返さない
- Omnigraph の使い方: `omnigraph skill export ~/.claude/skills/omnigraph --force` で、導入した CLI に合う agent skill を入れてから作業する

## 目的

Zettelkasten on Omnigraph で使う機能 (書き込み、一覧、昇格、レビュー、出典の確認) を 1 つの道具にまとめる。利用者が内容を書くことに集中できるようにする。

## 決まっていること

### 作り方

- Rust + ratatui。1 つのバイナリで TUI と即時の書き込みコマンドの両方を受け持つ
- Omnigraph は CLI (`omnigraph`) をサブプロセスで呼ぶ。Rust の組み込み SDK が外から使える形で公開されているかは未確認。使えるなら後で差し替えられる構成にする
- このリポジトリは非公開。nix flake でビルドし、dotfiles から入れる想定

### 持たせる機能

| 機能 | 内容 |
|---|---|
| 即時の書き込み | 端末から 1 行で Fleeting を追加。出典 (`-s <URL / DOI / ISBN / arXiv ID / slug>`) と位置 (`-p <locator>`) を付けると Literature |
| 出典の自動登録 | 運用文書の「自動登録の分担」のとおり。未登録の出典は review ブランチに入れる |
| inbox | 未処理の Fleeting / Literature を古い順に一覧。30 日以上たったものを出し、選んで削除 |
| 昇格 | inbox から複数を選び、本文を下書きとして `$EDITOR` で開く。保存すると Permanent を作り、`DerivedFrom` / `DerivedFromLit` を張る。埋め込みを作り、リンク判定を裏で始める |
| リンク判定 | ベクトル検索で上位 5 件 → Haiku が判定 → review ブランチに入れる。書き込みと切り離して裏で実行し、利用者を待たせない |
| レビュー | review ブランチの提案 (Links、Hub への追加、Source) を 1 件ずつ採用、修正 (種類と理由)、却下し、main にマージする |
| 出典の確認 | レビュー時に Source の元の値 (`input`) と項目ごとの取得元を表示する。LLM が補った項目に印を付ける。キー 1 つで元のページをブラウザで開く (DOI は `https://doi.org/<DOI>`、arXiv ID は `https://arxiv.org/abs/<ID>`、ISBN は書誌検索のページ) |
| バックアップ | main を `omnigraph export` で JSONL にし、データ用リポジトリに commit する |

レビュー画面の目安 (配置は未決定):

```
review/links  3 件
> 小さいノートはつなぎやすい → 長いメモは読み返さない
    extends  「粒度の話を読み返しの観点に広げる」
  [a]採用  [e]直す  [d]却下  [n]次へ

Source  ahrens2017            [o] 元のページを開く
  input   978-4-xxxx-xxxx-x
  title   How to Take Smart Notes        (API)
  kind    book                           (LLM *)
```

### LLM

- 判定 (リンク、Hub、出典の補完) は Haiku。`claude -p --model haiku` で呼び、JSON 配列だけを返させる。リンク判定の出力の形:
  `{"to": "<slug>", "kind": "supports|contradicts|extends|example_of|none", "reason": "<30字以内>"}`
- 埋め込みはローカルの Ollama (`bge-m3`)。Omnigraph の `openai-compatible` プロバイダで `http://localhost:11434/v1` を指す
- 追加の費用はかけない

## Omnigraph について分かっていること

ドキュメントとソース (2026-11-19 の main) で確認したもの。

- ローカルのグラフは `--target ./zettel.omni` で直接扱える。`omnigraph init --schema <file.pg> <path>` で作る
- データの読み込みは JSONL で、1 行に 1 件。`omnigraph load --data <file> --mode <overwrite|append|merge> --target ...`。`--mode` は必須
  - ノード: `{"type":"Permanent","data":{...}}`
  - エッジ: `{"edge":"Links","from":"<slug>","to":"<slug>","data":{...}}`。`@key` の値がノードの ID になる
- `@card(1..1)` の Cites は、Literature とその Cites を同じ書き込み (1 回の load か 1 つの mutation) で入れないと拒否される
- クエリ (`.gq`) では、エッジ名の先頭を小文字で書く (`Links` は `links`)。`not { ... }` で「エッジがない」を表せる
- ブランチ: `omnigraph branch create <name> --target ...`、`omnigraph branch merge <name> --into main --target ...`。読み込みとクエリは `--branch <name>` で対象を選ぶ。差分は `diff` で出せる
- 書き出し: `omnigraph export` (ブランチを JSONL で出す)
- ベクトル検索は `order { nearest($n.embedding, $q) }`。`$q` にテキストを渡すと、設定したプロバイダが埋め込みにする。Ollama を使う設定は環境変数 `OMNIGRAPH_EMBED_PROVIDER=openai-compatible`、`OMNIGRAPH_EMBED_BASE_URL`、`OMNIGRAPH_EMBED_MODEL`
- 全文検索 (`bm25`、`search`) は日本語に使えない (理由は運用文書)。`contains` の部分一致は使える
- 名前が `_` で始まるプロパティは予約されていて使えない
- Studio (デスクトップアプリ) でも型ごとの表とブランチの差分を見られる。TUI はそれを置き換えるものではない

## 環境

- macOS、Apple M4、メモリ 16GB。パッケージは nix-darwin + home-manager (dotfiles) で管理する
- 入っているもの: `claude`、`jq`、`fzf`、`git`。`python3` は入っていない。`omnigraph` と `ollama` はまだ入っていない (dotfiles から入れる予定)
- 利用者の指示: 応答と文書は日本語。絵文字は使わない。commit メッセージに Claude の関与 (Co-Authored-By など) を書かない。Python ライブラリを無断で入れない

## 利用者について

- 書くことに集中したい。リンクや出典の手作業を減らすことが、この道具の価値
- LLM のトークンと費用を増やしたくない
- 操作は少ないほうがよい。最初の操作が明らかで小さいこと

## 未確認事項 (このリポジトリのセッションで決める)

1. TUI を開いたとき最初に見える画面 (今日のノートか、inbox か、レビュー待ちの件数か)
2. 画面の構成と切り替え方、キー操作
3. 即時の書き込みコマンドの名前と引数の形 (仮に `zk "<本文>"`)
4. 裏で走るリンク判定の実行方法 (TUI の中のタスクか、別プロセスか) と、失敗時の扱い
5. JSONL の書き出しと commit をいつ行うか
6. 設定ファイルの置き場所と項目 (グラフのパス、データ用リポジトリのパス、モデル名)
