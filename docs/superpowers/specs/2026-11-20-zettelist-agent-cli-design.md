# zettelist: TUI の操作をコマンドラインでも行う (エージェント向け)

状態: 承認待ち (2026-11-20)

## 目的

TUI で行う操作 (inbox の確認、昇格、削除、レビューの採否とマージ、失敗のやり直し) を、すべてコマンドラインからも行えるようにする。主な使い手は AI エージェント。エージェントに判断までさせるかは使う人が決める。道具では制限しない。

あわせて、エージェントがこのコマンドを正しく使うための skill を作り、zl から書き出せるようにする。

## 目標と、目標にしないこと

- 目標: TUI でできる操作は、どれもコマンド 1 回で行える。出力は `--json` で機械が読める
- 目標: skill の中身が、入っている zl の版と一致する
- 目標にしないこと: 対話的な操作 (エディタを開くなど) をコマンドラインで再現すること。本文は引数かファイルで渡す
- 失敗とみなすこと: `zl "<本文>"` がメモ以外の意味になること (設計書の「サブコマンドは持たない」に反する)

## 選んだ方法

| 案 | 内容 | 結論 |
|---|---|---|
| A | `--` で始まるオプションを足す | 採用。設計書の決定と今の形 (`--backup`、`--worker`) を保てる |
| B | サブコマンド (`zl inbox`) | 不採用。`zl inbox` というメモが書けなくなる |
| C | 別のコマンド (`zlctl`) | 不採用。バイナリが 2 つになり、設定と待ち行列の共有が要る |

## コマンド

すべて計画 2 の関数を呼ぶだけにする。TUI (計画 3) も同じ関数を使う。

| 操作 | コマンド | 呼ぶ関数 |
|---|---|---|
| inbox の一覧 | `zl --inbox [--json]` | `notes::inbox` |
| ノートの全文 | `zl --show <slug> [--json]` | (新規。種類を問わず 1 件) |
| 似ているノート | `zl --similar <slug> [--json]` | `notes::nearest` (`inbox_similarity` 以上) |
| 昇格 | `zl --promote <slug>[,<slug>...] --title "<題>" (--body "<本文>" \| --body-file <path>)` | `notes::promote`、`Job::Promote`、`spawn_worker` |
| 削除 | `zl --delete <slug>[,<slug>...] --yes` | `notes::delete_notes` |
| 提案の一覧 | `zl --review [--json]` | `review::list_proposals` と手元の判断 |
| 採用 | `zl --adopt <key> [--reason "<理由>"]` | `ReviewState` に書く |
| 却下 | `zl --reject <key>` | 同上 |
| 判断を外す | `zl --undecide <key>` | 同上 (TUI の `u` に当たる) |
| マージ | `zl --merge --yes` | `review::apply`、`spawn_worker` |
| 失敗の一覧 | `zl --failed [--json]` | `Queue::failed` |
| やり直す / 捨てる | `zl --retry <id>`、`zl --retry-all`、`zl --discard <id> --yes` | `Queue` の各関数、`spawn_worker` |
| skill の書き出し | `zl --skill-export <dir> [--force]` | (新規) |

- `<key>` は `Proposal::key()` の形 (`link:<from>:<to>`、`hub:<hub>:<note>`)
- 削除、捨てる、マージは、TUI の確認の代わりに `--yes` を必須にする。ない場合は何もせず、終了コード 2 で理由を出す
- `--json` の出力は 1 つの JSON 値。エラーのときは標準エラーに `{"error": "...", "kind": "invalid|permanent|transient"}` を出す
- 終了コード: 0 成功、1 処理の失敗、2 使い方の誤りや入力の誤り (`Error::Invalid` を含む)
- 判断 (`review.json`) の書き込みは `review.lock` で `apply` と排他する

## skill

- リポジトリの `skill/SKILL.md` (必要なら `skill/references/`) に書く。ビルド時に `include_str!` で埋め込む
- `zl --skill-export <dir>` で書き出す。すでにあれば `--force` がないと拒否する
- 中身: 各コマンドの形と出力、典型の流れ (inbox を見る → 似ているノートを見る → 昇格する → 提案を見て採否 → マージ)、`--yes` が要る操作、失敗の扱い
- 判断をエージェントに任せるかは利用者が決める、と skill に書く。Permanent の本文を誰が書くかも、利用者の運用に従うと書く

## 運用文書の修正

運用文書の「エージェントには Fleeting までしか書かせない」を、「道具は制限しない。エージェントに何をさせるかは運用で決める (この利用者は Fleeting まで)」に書き直す。

## builder に任せること

- 人向けの表示 (`--json` なし) の細部
- `--json` の各フィールド名 (skill に書いた形と一致させる)
- clap での排他の指定 (同時に使えないオプションの組)
