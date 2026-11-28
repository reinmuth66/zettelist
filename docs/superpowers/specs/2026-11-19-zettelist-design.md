# zettelist 設計

状態: 承認済み (2026-11-19)。`2026-11-19-zettelkasten-tui-design.md` (引き継ぎ) の未確認事項を決め、前提の一部を変えたもの。引き継ぎと食い違うところは、この文書が優先する。

## 最初に読むもの

- 運用の決まりとスキーマの正本: `~/ghq/github.com/reinmuth66/zettelkasten/docs/superpowers/specs/2026-11-19-zettelkasten-design.md`。ノートの種類、slug、出典、検索、保存はそこに従う。ただし「この文書で変えること」に挙げた点は、この文書が優先する (運用文書の側は 2026-11-20 に直した)
- 引き継ぎ: `docs/superpowers/specs/2026-11-19-zettelkasten-tui-design.md`。Omnigraph について分かっていること、環境、利用者についてはそこを参照する
- Omnigraph の使い方: `omnigraph skill export ~/.claude/skills/omnigraph --force` で、導入した CLI に合う agent skill を入れてから作業する

## 目的

Zettelkasten on Omnigraph で、書いたものを片付けてグラフに入れるまでを 1 つの道具で受け持つ。利用者は書くことと、提案を確かめることだけをすればよい。

- 利用者の価値: リンク付けと出典の登録の手作業を減らす
- 制約: 追加の費用をかけない。LLM のトークンを増やさない。操作は少なく、最初の操作が明らかで小さいこと

## 名前

- 道具の名前は **zettelist**、コマンドは **`zl`**
- 2026-11-19 に確認: crates.io、npm、PyPI、nixpkgs、GitHub のリポジトリ名に同名なし。GitHub のコード中の 157 件はすべて `GazetteList` の部分一致で無関係。`zl` というコマンドも手元と nixpkgs になし
- 公開も視野に入れる。公開の前に商標 (USPTO、EUIPO) を確かめる (未確認)
- リポジトリ名は `zettelist` (2026-11-20 に `zettelkasten-tui` から変更)

## 道具の分担

| 道具 | 役割 | 使う場面 |
|---|---|---|
| `zl "<本文>"` (zsh) | 書く | 思いついたとき、読みながら |
| `zl` (TUI) | 書く、整理する | 1 日 1 回ほど。inbox を片付け、提案を確かめる |
| Omnigraph Studio | 眺める、細かく直す | Permanent の閲覧、リンクをたどる、全体の図、表での直接編集、過去の版、クエリ |

3 つとも、常駐する `omnigraph-server` の同じグラフを見る。

TUI に持たせないもの: Permanent の閲覧とリンクのたどり歩き、表での一括編集、クエリ、過去の版。これらは Studio が受け持つ (Studio の機能は 2026-11-19 に README で確認)。

## この文書で変えること (引き継ぎと運用文書から)

| 項目 | 前 | 後 |
|---|---|---|
| グラフへの接続 | `--target ./zettel.omni` でファイルを直接扱う | 常駐の `omnigraph-server` に HTTP でつなぐ。Studio は HTTP のサーバーにしかつなげないため。常駐は dotfiles の launchd で行う |
| リンクの種類 | `supports / contradicts / extends / example_of` | 種類なし |
| リンクの理由 | LLM が 30 字以内で必ず書く | 利用者がレビューで、書きたいときだけ書く |
| 判定役 | Haiku (`claude -p`) | なし。ベクトルの近さだけで候補を出し、採否は利用者がレビューで決める。判定器は後から差し込める作りにする |
| 出典の補完 | 欠けた項目を Haiku が埋める | スクリプトと辞書で埋める。LLM は使わない |

スキーマの変更 (運用文書のスキーマに対して):

```
edge Links: Permanent -> Permanent {
  @key(@src, @dst)       // 同じ組は 1 本にする (確認済み、「計画 2 の確認の結果」)
  reason: String?        // 利用者が書いたときだけ
  similarity: F32        // 提案したときのベクトルの近さ (型は Omnigraph で使えるものを builder が選ぶ)
}

node Fleeting   { ... embedding: Vector(1024)? }  // 追加。inbox で似ているノートを示すため
node Literature { ... embedding: Vector(1024)? }  // 追加
node Permanent  { ... embedding: Vector(1024)? }  // null を許すように変える (worker が後から書き足すため)
```

`kind` はなくす。`provenance` の取得元の値から `LLM` がなくなり、`dict` (辞書で読みを取った) が加わる。

Source の ID を、利用者が渡した元の値から作る。slug は ID ではなく、書き換えられる項目にする。

```
node Source {
  id: String @key        // isbn:9784xxxxxxxxx / doi:10.1145/... / arxiv:2401.01234 / url:<URL>
  slug: String           // ahrens2017 など。zl が重なりを確かめる
  title: String
  ...                    // 残りは運用文書のとおり
}
```

- ID の作り方: ISBN はハイフンを除き 13 桁にそろえる。DOI は小文字にそろえる。arXiv ID は版 (`v2` など) を除く。URL は運用文書のとおり doi.org / arxiv.org なら DOI / arXiv ID の ID にする。それ以外の URL の正規化は builder が決める
- ID は書いた瞬間に決まり、変わらない。Cites は最初から最後まで同じ Source を指す
- slug は Omnigraph の `@key` ではないので、重ならないことは zl が確かめる (同じ slug があれば運用文書の規則で `b`、`c` を付ける)
- `-s <slug>` で指したときは、slug の項目で Source を探す

## 機能

### 書く

- zsh: `zl "<本文>"` で Fleeting を追加。`-s <URL / DOI / ISBN / arXiv ID / slug>` で出典、`-p <locator>` で位置を付けると Literature になる。本文は引用符で囲んだ 1 つの引数とする
- TUI: inbox で `i` を押すと、画面の下に入力欄が出る。`Enter` で保存
  ```
  新規  箇条書きは考えを止める_
  出典  978-4-xxxx-xxxx-x        位置  p.42
  ```
  - `Tab` で本文、出典、位置の欄を移る。出典が空なら Fleeting、入っていれば Literature
  - 出典の欄では登録済みの slug を補完する (`ahr` で `ahrens2017` が候補に出る)
  - 位置の欄は設定 `ui.locator` で出すか決める。既定値はオフ (利用者は位置をほとんど書かないため)。オフのときは本文と出典の欄だけ
  - 位置の欄には、出典の種類に合わせた書き方の例を薄く出す (book `p.42` / `ch.3`、paper `sec.3.2` / `fig.4`、article は見出し、video `12:30`、talk `slide 14`)。`locator` はただの文字列で、形式は決めない
- どちらも、出典が未登録なら、元の値から ID を作り、仮の Source を Literature と Cites と同じ 1 回の書き込みで main に入れる (Cites は `@card(1..1)` のため)。仮の Source は `title` に元の値、`kind` に `other` を入れ、`slug` は空にする。そのあと待ち行列に出典の登録を積んで、すぐに戻る
- 同じ元の値の Source がすでにあれば (仮のものも含む)、それを指す
- 登録済みの出典 (slug か、登録済みの元の値) を渡したときは、既存の Source を指す
- メモは、裏の処理の成否に関係なく必ず保存する

### コマンドの形

- サブコマンドは持たない。`zl` だけなら TUI、`zl "<本文>"` は必ずメモとして書く
- ほかの操作は `--` で始まるオプション (例: `zl --backup`、`zl --worker`)
- 本文が `-` で始まるときは `zl -- "<本文>"`
- 操作のオプションは同時に 1 つだけ。本文や `-s` `-p` とも組み合わせられない
- 読む: `--inbox`、`--show <slug>`、`--similar <slug>`
- 昇格と削除: `--promote <slug,...> --title <題> (--body <本文> | --body-file <パス>)`、`--delete <slug,...> --yes`
- レビュー: `--review`、`--adopt <キー> [--reason <理由>]`、`--reject <キー>`、`--undecide <キー>`、`--merge --yes`
- 失敗したジョブ: `--failed`、`--retry <id>`、`--retry-all`、`--discard <id> --yes`
- 保守: `--worker`、`--reembed`、`--backup`
- skill の書き出し: `--skill-export <dir> [--force]`
- `--json` を付けると、結果は標準出力の JSON、失敗は標準エラーの JSON になる (エージェント向け)
- 取り消せない操作 (`--delete`、`--merge`、`--discard`) は `--yes` がないと何もせずエラー (終了コード 2) で止まる
- エージェント向けの設計は `docs/superpowers/specs/2026-11-20-zettelist-agent-cli-design.md`、skill の本文は `skill/SKILL.md`

### inbox (最初の画面)

未処理の Fleeting / Literature (`DerivedFrom` / `DerivedFromLit` がまだないもの) を古い順に並べる。

```
inbox 12 件 (30日超 3)     review 4 件     失敗 0 件
┌──────────────────────────┬──────────────────────────┐
│[x] 1118 小さいノートは…  F │ 小さいノートはつなぎやす │
│> [ ] 1117 Ahrens p.42…  L │ い。理由は…              │
│  [ ] 1020 長いメモは…  F 30+│                          │
│                          │ 出典 ahrens2017  p.42    │
└──────────────────────────┴──────────────────────────┘
[i]書く [space]選ぶ [d]削除 [r]レビュー [q]終了
```

- 左に一覧、右に選んでいるノートの全文 (折り返して表示)
- 似ているノート: カーソルのあるノートに似ている inbox のノートを、一覧の中で色と印 (`●`) で示す。カーソルを動かすたびに変わる。右の欄の下に、似ている Permanent を上位 3 件まで出す (すでに書いた考えに気づくため。選べない)。どちらもベクトル検索だけで、近さが `inbox_similarity` 以上のもの
- 似ているものをまとめて選ぶキーは設けない。何をまとめて昇格するかは、利用者が中身を読んで選ぶ
- 似ているもの同士のグループを常に色分けする表示は、使ってみて欲しくなったら足す
- 30 日を超えたものに印を付ける。選んで削除できる (確認を 1 回はさむ)
- 上部に review の件数と、裏の処理の失敗の件数を出す

### 昇格

1. inbox で 1 つ以上選び、`i` を押す
2. 画面の下に 1 行の入力欄が出て、Permanent の題 (`title`) を書く。選んだノートは上に見えたまま。空のまま `Enter` か `Esc` なら昇格を取り消す
3. `$EDITOR` が開き、本文 (`body`) を書く。下書きは次の形:
   ```
   (ここに Permanent を書く)

   # ---- この行から下は保存時に消える ----
   # [1118 F] 小さいノートはつなぎやすい。理由は…
   # [1117 L ahrens2017 p.42] 箇条書きは…
   ```
4. 保存して閉じると Permanent を作り、選んだ元のノートに `DerivedFrom` / `DerivedFromLit` を張る。元のノートは inbox から消える
5. 待ち行列に、リンク判定と Hub の振り分けを積む
6. 本文を何も書かずに閉じたら、昇格を取り消す

### レビュー

review ブランチの提案 (リンク、Hub への追加、出典) を 1 件ずつ出す。

```
review 4 件  (links 3 / hub 0 / source 1)          (近さ 0.82)
┌ 新しいノート ──────────────┬ 候補 ─────────────────────┐
│ 小さいノートはつなぎやすい │ 長いメモは読み返さない    │
│ 1 枚に 1 つの考えだけを…   │ 長く書いたメモは後で…     │
└────────────────────────────┴───────────────────────────┘
理由: _                                   (A のときだけ出る)
[a]採用 [A]理由を書いて採用 [d]却下 [n]次へ [u]取消 [m]マージ [Esc]inbox
```

- リンク: 両端の本文を左右に並べ、ベクトルの近さを出す。`A` を押すと画面の下に 1 行の入力欄が出て、両方の本文を見たまま理由を書ける。空のまま `Enter` なら理由なしで採用
- Hub: 新しいノートと、提案された Hub の題名を出す
- 出典: 元の値 (`input`) と、項目ごとの値と取得元を並べる。辞書や代わりの値で埋めた項目に `*` を付ける。`o` で元のページをブラウザで開く (URL はそのまま、DOI は `https://doi.org/<DOI>`、arXiv ID は `https://arxiv.org/abs/<ID>`、ISBN は書誌検索のページ)。開いたページで確かめられるのは題、著者、年。本や有料の論文の本文までは読めない
- 採用したものは review に残し、`m` を押した時点でまとめて main にマージする (確認を 1 回はさむ)。途中でやめても採用の結果は残る
- 却下したものは review から消す
- 出典の提案は「採用」か「直す」だけで、却下はない。仮の Source を消すと、それを指す Literature が `@card(1..1)` に反するため。元の値を打ち間違えたときは、「直す」で元の値を書き換えると、新しい ID の Source を作って Cites を付け替え、書誌を取り直す (この場合だけ付け替えが起きる)
- 仮の Source のままでも、その Literature は昇格などを普通にできる

```
Source  isbn:9784xxxxxxxxx    [o] 元のページを開く
  input   978-4-xxxx-xxxx-x
  title   ノートの技法          (openBD)
  author  鈴木一郎              (Google Books)
  slug    suzuki2019          (dict *)
```

出典の修正は、TUI の中で項目を選び 1 行の入力欄で直す (例: 姓のローマ字を `suzuki` と打つと slug が `suzuki2019` に決まる)。

### キー操作

考え方:

- vim に近い。矢印キーも使える
- よく使う操作は小文字 1 文字。大文字は同じ操作の強い版
- 削除、捨てる、マージだけ確認を 1 回はさむ (`y` で実行、それ以外で中止)
- 入力欄の中では文字の入力を優先し、操作は `Enter`、`Esc`、`Tab` と zsh 風の編集キーだけ
- その画面や提案で使えないキーは、押しても何も起きない (メッセージも出さない)
- 画面にキーの案内は出さない。キーは `?` の一覧で確かめる
- キー割り当ては設定ファイル (`[keys]`) で変えられる。形式は builder が決める

どの画面でも共通:

| キー | 動作 |
|---|---|
| `j` / `k` (`↓` / `↑`) | 下 / 上へ移動 |
| `gg` / `G` | 先頭 / 末尾へ |
| `Ctrl-d` / `Ctrl-u` | 右の欄 (本文) を下 / 上に送る |
| `?` | キーの一覧 |
| `q` | 終了 (inbox 以外では inbox に戻る) |

inbox:

| キー | 動作 |
|---|---|
| `i` | 何も選んでいなければ新しいメモを書く。1 つ以上選んでいれば、選んだものから Permanent を書く (題の入力欄、次にエディタ) |
| `space` | 選ぶ / 外す |
| `Esc` | 選択を全部外す |
| `d` | 削除 (選んだもの、なければカーソルの 1 件。確認あり) |
| `r` | レビューへ |
| `f` | 失敗の一覧へ |
| `o` | カーソルのノートの出典の元のページを開く (Literature のとき) |

入力欄 (書く、題、理由、出典の修正):

| キー | 動作 |
|---|---|
| `Enter` / `Esc` | 保存 / やめる |
| `Tab` / `Shift-Tab` | 次 / 前の欄へ |
| `←` `→` `Ctrl-a` `Ctrl-e` | カーソル移動、行頭、行末 |
| `Ctrl-w` / `Ctrl-u` | 1 語消す / 行頭まで消す |

レビュー (並び順は 出典、リンク、Hub。出典を先に確定させると、リンクの画面で出典の名前が正しく出るため):

| キー | リンク | Hub | 出典 |
|---|---|---|---|
| `a` 採用して次へ | 張る | この Hub に入れる | この書誌で確定 |
| `A` 理由を書いて採用 | 入力欄で理由を書いてから張る | なし | なし |
| `d` 却下して次へ | 張らない | 入れない | なし (出典は消せない) |
| `c` 直す | なし | 別の Hub を一覧から選び直す | 項目 (題、著者、年、slug、元の値) を選び、入力欄で直す |
| `o` | なし | なし | 元のページをブラウザで開く |
| `n` / `N` | 判断せずに次 / 前へ | 同じ | 同じ |
| `u` / `Ctrl-r` | 直前の判断を取り消す / やり直す | 同じ | 同じ |
| `m` | 採用したものをまとめて main に入れる (確認あり) | 同じ | 同じ |
| `Esc` | inbox へ | 同じ | 同じ |

`c` は TUI の中で直す操作。エディタを開く操作 (`i` からの昇格) と区別するため `e` は使わない。

失敗の一覧:

| キー | 動作 |
|---|---|
| `Enter` | やり直す |
| `x` | 捨てる (確認あり) |
| `R` | 全部やり直す |
| `Esc` | inbox へ |

### 見た目

方針だけ決め、細部 (色の割り当て、枠の線、左右の欄の比率、上下の帯の見た目、確認の出し方) は、動く画面ができた段階でスクリーンショットを見ながら利用者と決める。

- TUI と CLI の文言 (エラー、`--help`、出力) は英語にする (ノートの本文は除く)
- 色は端末の 16 色 (ANSI の色) だけを使う。利用者の端末のテーマにそのまま合わせるため
- 色は設定 (`[theme]`) で変えられる
- 色だけに頼らず、記号 (`●`、`*`、`30+`) も必ず添える
- 端末の幅が 80 桁未満なら、右の欄を隠して一覧だけにする
- 長い題は表示幅で切り、`…` を付ける。全角文字を半分で切らない

### 入力の方法

- 1 行で済むもの (TUI で新しく書く、Permanent の題、リンクの理由、出典の修正) は TUI の中の 1 行の入力欄。長くなったら横に流す。全角文字の幅は `unicode-width` で数える
- 複数行になるもの (昇格の Permanent) だけ `$EDITOR` を使う

## 裏の処理

### 実行方法

- 処理を待ち行列 (`~/.local/share/zettelist/queue/` のファイル) に積み、`zl --worker` を裏で起動して、すぐに戻る
- worker は待ち行列が空になったら終わる。すでに動いていれば新しく起動しない (ロックファイルなどで排他する。方法は builder が選ぶ)
- 失敗は種類で分ける
  - 一時的なもの (オフライン、タイムアウト、omnigraph-server や、埋め込みに使う Ollama が起動していない): 10 秒、1 分、5 分の間を空けて 3 回やり直す。短い通信の途切れを救える程度でよい (回数に根拠はない)
  - 何度やっても同じもの (ISBN が見つからない、ページにメタデータがない): やり直さず、すぐ「失敗」に回す
- 「失敗」に回ったものは待ち行列の「失敗」に残す。TUI の上部に件数を出し、一覧で原因を見て、キー 1 つでやり直すか、捨てられる
- 出典の登録が失敗しても、仮の Source のまま Literature は使える。レビューの「直す」で書誌を手で埋めてもよい

処理の種類:

| 処理 | きっかけ |
|---|---|
| 出典の登録 | 未登録の出典つきでメモを書いたとき |
| 埋め込み | メモを書いたとき、昇格したとき |
| リンクの候補探し、Hub の振り分け | 昇格したとき |
| バックアップ | `m` でマージしたとき |

### リンクの候補探し

1. 新しい Permanent に似た既存の Permanent を、ベクトル検索で出す
2. 近さが `min_similarity` 以上のものから、上位 `candidates` 件 (既定値 5) を review ブランチに入れる
3. 近さが足りるものが 1 件もなければ、提案はしない (新しい考えとして扱う)

- 判定器 (LLM) は使わない。採否は利用者がレビューで決める
- 1 回の昇格で提案するのは最大 `candidates` 件。古いノートは、あとから昇格したノートの候補に選ばれることでリンクが増えていく
- 繋げすぎを避けるため、既定値は 5 とする。多いと感じたら設定で減らす
- `min_similarity` の既定値は、Permanent がたまってから決める (運用文書の埋め込みの確認と同じ時期)。それまでは仮の値を置く

**判定器を足す場合 (後で、必要になったら)**: レビューで却下が多すぎると感じたら、候補ごとに「リンクを張る価値があるか」を yes / no の確率で答える判定器を差し込む。そのため、候補を出す処理と review に入れる処理のあいだに、差し替えられる絞り込みの段を 1 つ設けておく (最初は何もしない)。調べた候補は次のとおり (2026-11-19)。足すときに 30 組ほどを正解付けし、つながらない組も半分入れて比べる。

| 候補 | 動かし方 |
|---|---|
| Qwen3 1.7B / 4B | Ollama の `/api/generate` で 1 トークンだけ生成し、logprobs から `P(yes) / (P(yes) + P(no))` を出す。Ollama 0.12.11 以降 |
| Qwen3-Reranker 0.6B | 同上 (Ollama の rerank の機能は不安定) |
| Jeff (GLiFormer 400M) | Ollama にない。別の実行方法を調べる |
| Groq の無料 API | logprobs に対応していないので確率が取れない。ノートが外に出る |

### Hub の振り分け

新しい Permanent について、既存の Hub のどれに入れるか (どれにも入れないか) を提案し、review ブランチに入れる。Hub の題名の埋め込みと、ノートの埋め込みの近さで決める (LLM なし)。Hub に埋め込みを持たせるか、その都度作るかは builder が選ぶ。

### 出典の登録

運用文書の「自動登録の分担」の段 3 (Haiku) をなくし、スクリプトと辞書で埋める。

| 入力 | 取得元 | 欠けやすい項目の埋め方 |
|---|---|---|
| 既存の slug | なし | 登録しない |
| DOI | Crossref | 姓は `family`。`type` を `kind` に対応表で変換する |
| arXiv ID | arXiv API | 姓は著者名の最後の語。`kind` は常に `paper` |
| ISBN (国内書) | openBD | 著者の読み (「スズキ, イチロウ」の形) の読点より前を姓とし、カナをローマ字にする。`kind` は常に `book` |
| ISBN (洋書) | Google Books | 姓は著者名の最後の語。`kind` は常に `book` |
| URL | ページのメタデータ | `citation_*` のメタタグ、JSON-LD (`author`、`datePublished`)、`og:` / `article:published_time`、`<time>` の順に試す。`kind` はドメインの表で決める (YouTube は `video`、Speaker Deck は `talk`、それ以外は `article`)。閲覧日 `accessed` を付ける |

スクリプトで埋まらないとき:

- 著者のない Web 記事: 運用文書の規則どおり slug は `<サイト名><年>`
- 日付のない Web 記事: 閲覧日の年で埋め、`*` を付ける
- 読みのない日本人の名前 (Google Books にしか載っていない本など): lindera (IPADIC か UniDic) で漢字の名前を姓と名に分けて読みを取り、`wana_kana` でローマ字にする。読みが複数ある姓は外れることがあるので `*` を付ける。分けられなければ slug を空のまま review に入れ、利用者が直す

### バックアップ

- `m` でマージしたら、待ち行列にバックアップを積む。worker が main を `omnigraph export` で JSONL にし、データ用リポジトリに commit する
- commit のメッセージに件数を入れる (例: `merge: links 3, source 1`)。commit に Claude の関与を書かない
- push は設定 `backup.push` で選ぶ。`true` ならマージのたびに commit のあと push する。既定値は `false` (公開したとき、知らないうちにノートが外に送られないため)。利用者の環境では `true` にする。push の失敗は一時的な失敗として待ち行列でやり直す
- `zl --backup` で手動でも実行できる
- zsh から書いた Fleeting は main に直接入るので、次のマージのときにまとめて commit される

## 設定

置き場所: `~/.config/zettelist/config.toml`

```toml
[omnigraph]
server = "http://127.0.0.1:8080"   # 常駐の omnigraph-server
graph  = "zettelkasten"            # サーバー上のグラフ名。各自が決める。既定値は zettelkasten

[backup]
repo = "~/ghq/github.com/reinmuth66/zettelkasten"   # データ用リポジトリ
push = false                                        # true でマージのたびに push

[embed]
ollama = "http://localhost:11434"   # 書き込みのときに zl が埋め込みを作る
model  = "qwen3-embedding:0.6b"     # cluster.yaml の providers.embedding の model と同じにする

[link]
candidates       = 5      # 1 回の昇格で提案するリンクの最大数
min_similarity   = 0.0    # これ未満の近さの候補は出さない。仮の値。Permanent がたまってから決める
inbox_similarity = 0.0    # inbox で「似ている」と示す近さ。仮の値

[ui]
locator = false           # true で TUI の入力欄に位置の欄を出す

[theme]                    # 色の割り当て。ANSI の 16 色 (black..white、bright_*、default) の名前で書く
similar = "cyan"           # cursor, selected, similar, old, dim, header, error, adopted, rejected
old     = "bright_black"

[keys.common]              # 節は common, inbox, review, failed。動作は Action の snake_case
down = ["j", "down"]       # 既定のキーを置き換える。1 文字、名前 (space 等)、ctrl-<文字>
top  = ["g g"]             # 2 つのキーの並びは空白で区切る

[keys.review]
adopt_with_reason = ["A"]
```

- 埋め込み (確認で分かったこと。`docs/superpowers/notes/2026-11-19-omnigraph-check.md`):
  - 書き込み時に Omnigraph は埋め込みを作らない。zl の側でベクトルを用意する。方法は 2 つあり、builder が選ぶ: (a) zl が Ollama の `POST /v1/embeddings` を直接呼ぶ (設定の `[embed]` で URL とモデルを決める)。(b) zl が `omnigraph embed --input <jsonl> --output <jsonl> --spec <json>` をサブプロセスで呼ぶ (環境変数 `OMNIGRAPH_EMBED_PROVIDER=openai-compatible`、`OMNIGRAPH_EMBED_BASE_URL=http://localhost:11434/v1`、`OMNIGRAPH_EMBED_MODEL=qwen3-embedding:0.6b` で設定)。(b) は確認していない
  - 書き込みは Ollama を待たない。`embedding` は null を許す (`Vector(1024)?`)。メモと Permanent はベクトルなしですぐに保存し、埋め込みは待ち行列に積んで worker が後から書き足す。`nearest` は null のベクトルを飛ばすので、埋め込みがまだのノートは似ているノートの表示とリンクの候補に出ないだけで済む (確認 3a で null のまま書けることを確かめた)。昇格のあとのリンクの候補探しは、その Permanent の埋め込みを作ってから行う
  - 検索で `nearest` にテキストを渡すと、サーバーが cluster.yaml の `providers.embedding` で埋め込みにする。zl の `[embed].model` と cluster.yaml の `model` は同じものにする (違うと検索が狂う)。zl は検索でもベクトルを自分で作って渡してよい (`$v: Vector(1024)`)。どちらにするかは builder が決める
  - 近さ: `nearest(...) as distance` は 2 乗ユークリッド距離 (0 から 4)。zl は `similarity = 1 - distance / 2` (コサイン類似度) に直して扱う。`min_similarity`、`inbox_similarity`、Links の `similarity` はこの値
  - モデルを替えたら全ノートの埋め込みを作り直す (`zl --reembed`)
- 公開を考え、各項目に既定値を持たせる。設定ファイルがなくても既定値で動く
- エディタは環境変数 `$EDITOR` に従う

## 作り方

- Rust + ratatui。1 つのバイナリで TUI、書き込み、worker を受け持つ
- Omnigraph には `omnigraph-server` の HTTP でつなぐ。HTTP の呼び出しは 1 か所にまとめ、後で SDK などに差し替えられるようにする
- LLM は使わない。Ollama は埋め込みのためだけに使い、zl (書き込み) と omnigraph-server (テキストでの検索) から呼ばれる
- Omnigraph には CLI (`omnigraph --target <server target>`) をサブプロセスで呼ぶ。確認で全操作ができたのは CLI だけで、HTTP を直接呼ぶ形は試していない。呼び出しは 1 か所にまとめ、後で HTTP や SDK に差し替えられるようにする。Rust のクレート (`omnigraph-engine`、`omnigraph-api-types` など) は crates.io に出ているが、2026-11-20 の時点で 0.8.0 で、使っている CLI (0.15.0、HTTP contract 0.14) より古い。当面は CLI のままにする
- omnigraph-server は cluster 設定からしか起動できない。cluster.yaml (グラフ、スキーマ、埋め込みプロバイダ) を置き、`cluster apply` でグラフを作り、apply のロックを外してから `omnigraph-server --cluster file://<data> --bind 127.0.0.1:8080 --unauthenticated` で起動する。手順は確認の記録にある。cluster.yaml とスキーマはデータ用リポジトリに置く (builder が場所を決める)
- スキーマの変更は、生きているブランチが main だけのときにしかできない。zl のスキーマを変えるときは review ブランチをいったん消す
- 開発環境とビルドは nix flake。`devShells` に Rust のツールチェーン、`omnigraph` / `omnigraph-server`、`ollama` を入れる。`packages` でバイナリを出し、dotfiles から入れる
- `omnigraph-server` の常駐 (launchd) は dotfiles で行う。この文書の範囲外
- 外部の API (Crossref、openBD、Google Books、arXiv) は無料の公開 API。キーは要らない

## 目的に入れないもの

- Permanent の閲覧画面 (Studio に任せる)
- GUI アプリ (GPUI も検討したが、資料の少なさと版の不安定さから TUI にした)
- LLM が文章を生成すること (理由、要約、補完)
- 自動マージ

## 検討した方法

| 項目 | 選んだもの | 選ばなかったもの |
|---|---|---|
| 形 | TUI (ratatui)。複数行の入力は `$EDITOR` | GPUI のアプリ: 日本語入力は楽だが、資料が少なく版が不安定で、最初に動くまで 1〜2 週間かかる見込み |
| 判定役 | なし (ベクトルの近さで候補を出し、人がレビューで決める) | ローカルの小型モデルを logprobs で判定器として使う: 話題が近いだけの候補を落とせるが、モデルが要り、外れると必要なリンクも落ちる。提案はどのみち人が確かめるので、まず無しで始める。Jev / Laya: 判定専用だが有料で API 専用。Selene Mini: 文章を書く採点用。Granite Guardian: 重く、何回も呼ぶ。mDeBERTa XNLI: 種類が足りず日本語が弱い。Groq の無料 API: logprobs と埋め込みのモデルがない |
| 似ているノートの見せ方 | カーソルのあるノートに似ているものを色で示す | 似ているもの同士のグループを常に色分け: 分け方と色の数が難しい。似ているものをまとめて選ぶキー: 何をまとめるかは利用者が読んで決めるべき |
| リンクの種類 | 種類なし | Discourse Graph の 3 種類 (`supports / opposes / informs`): 裏付けはあるが少数派で、小型モデルでは外れやすい。今の 4 種類: 定義の裏付けがない。Markdown で Zettelkasten をする人の主流は種類なしで、文脈は文章で書く |
| リンクの理由 | 書きたいときだけ、レビューの 1 行の入力欄で | 本文に文章で書く: 手間が大きい。書かない: 何も残らない |
| 接続 | 常駐の `omnigraph-server` | ファイル直接: Studio を使うときだけサーバーを立てることになり、同時の書き込みが安全か分からない |
| 埋め込みを作る側 | zl の側 (書き込み時、worker が後から) とサーバー (テキストでの検索時) | サーバーだけ: 書き込み時に Omnigraph は埋め込みを作らないため、できない (確認で判明。当初はこれを選んでいた)。書き込みのその場で作る: Ollama が止まっていると書き込めず、「すぐに戻る」に反する |
| 裏の処理 | 待ち行列と、必要なときだけ動く worker | launchd での常駐: 常駐が 2 つになる。その場で処理: 待たされる |
| バックアップ | マージのたび | 1 日 1 回: その日のうちに戻せない。手動だけ: 忘れる |
| 未登録の出典 | Source の ID を元の値から作り、slug を項目にする | 仮の slug で作り後で付け替える: ほとんどの出典で付け替えが起きる。書くときにその場で書誌を取る: 1〜3 秒待たされ、オフラインでは結局付け替えが要る。出典が登録されるまで Fleeting にする: その間出典なしに見える。`@card(0..1)` にゆるめる: 出典のない Literature ができる |
| Permanent の題 | エディタの前に TUI の 1 行の入力欄で決める | 下書きの 1 行目を題にする |

## builder に任せること

- 待ち行列のファイルの形式と、worker の排他の方法
- `similarity` の型
- Hub の埋め込みの持ち方
- キー割り当ての設定ファイルの形式 (計画 3 で決定: `[keys.<節>]` (`common`, `inbox`, `review`, `failed`) の下に `<動作の snake_case> = ["..."]` で書く。キーは 1 文字・名前 (`space` `enter` `esc` `tab` `backtab` `up` `down` `left` `right` `backspace`)・`ctrl-<1 文字>`、2 つまでの並びは空白区切り。書いた動作の既定を置き換える)
- Crossref の `type` と `kind` の対応表、URL のドメインと `kind` の対応表
- ISBN の書誌検索ページの URL (計画 3 で決定: `https://books.google.com/books?vid=ISBN<ISBN>`)

## 最初の確認の結果 (2026-11-20)

詳細は `docs/superpowers/notes/2026-11-19-omnigraph-check.md`。

1. 必要な操作 (書き込み、ブランチの作成とマージ、差分、export、ベクトル検索) は、CLI を server target に向けるとすべてできた。`@card(1..1)` はサーバーでも守られる
2. review ブランチ: review は常にある 1 本のブランチとする。ブランチは作った時点の main しか見ないので、main にだけある Permanent へのリンクは書けない。**提案を review に書く前に、毎回 `branch merge main into review` で main を取り込む**。review を main にマージするとき、main が先に進んでいても問題ない。採用 / 却下の印は zl の手元 (待ち行列と同じ場所) に持ち、`m` のときに採用したものだけを main に入れる (却下したものは review から消してからマージする)
3. 埋め込みは書き込み時に自動では入らない。zl が作る (「設定」の節)。`nearest` の距離は取れるので、`min_similarity` での足切りはそのままできる
4. 待ち受けは `--bind 127.0.0.1:8080` で決める。設定の `server` の初期値は `http://127.0.0.1:8080` のまま

### 計画 2 の確認の結果 (2026-11-20)

「まだ確かめていないこと」の 3 点は、すべてできた。詳細は同じ確認の記録の「計画 2 の確認」の節。

1. 2 回目以降の往復: review を main にマージしたあと、main に Permanent を足し、`branch merge main into review` で取り込めば、その Permanent へのリンクを review に書ける。往復を重ねても問題ない
2. 却下: review に書いたリンクを `mutate --branch review` の `delete Links where @src = $a and @dst = $b` で消してから review を main にマージすると、消したリンクは main に入らず、残したリンクだけが入る。`where` で `@src` と `@dst` を `and` でつなげる。存在しない組を消しても失敗しない (何も変わらない)。delete は insert と同じクエリに混ぜられない
3. 重複: `Links` と `Includes` には、本体の先頭に `@key(@src, @dst)` を付ける。同じ組を `load --mode merge` で何度書いても 1 本で、2 回目に `reason` を付けると 1 本のまま `reason` が入る。同じ組を再び提案すると、既存のエッジの `similarity` と `reason` が上書きされる。すでに採用したリンクの `reason` を消さないよう、zl は提案の前に既存のリンクを調べて除く

```
edge Links: Permanent -> Permanent {
  @key(@src, @dst)
  reason: String?
  similarity: F32
}
edge Includes: Hub -> Permanent {
  @key(@src, @dst)
  position: I32?
}
```

削除については次のとおり。ノードを消すと、そのノードにつながるエッジは一緒に消える (`delete Literature` で Cites、`delete Fleeting` で DerivedFrom、`delete Permanent` で Links)。Cites だけを消すことは `@card(1..1)` で拒否される。したがって「Cites を消してから Literature を消す」手順は要らず、ノードの種類ごとに delete だけのクエリを 1 回流せばよい。

main で Permanent や Hub を消しても (Studio で消したときなど)、review にある提案のエッジは消えない。そのまま `branch merge main into review` を流すと `orphan_edge` で衝突し、apply も提案の書き込みも止まる (2026-11-25 に確認)。そこで zl は、main を取り込む前に、main にないノードを端に持つ提案を review から消す。消えた提案と、それに付けた判断は、知らせずに捨てる。

サーバーが止まっているときの終了コードと出力、`query --json` の形は、確認の記録に書いた。zl はエラーを、終了コードではなく `--json` の標準出力の JSON (`request_dispatched`、`http_status`、`code`) で分類する。

ほかに、`omnigraph embed` を使う方法 (埋め込みの方法 b) を選ぶなら、それは別に確かめる。

## 未確認事項

1. 商標 (USPTO、EUIPO) の確認 (公開の前)
2. ~~運用文書の側の修正~~ (2026-11-20 に反映済み)
