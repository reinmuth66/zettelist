# omnigraph-server の確認 (2026-11-20)

設計書 `docs/superpowers/specs/2026-11-19-zettelist-design.md` の「最初の確認」の結果。手順は `spike/check.sh`、設定は `spike/cluster/`、出力の例は本文に抜き出した。

## 環境

- omnigraph 0.15.0 (flake input `github:ModernRelay/omnigraph/v0.15.0`、ソースからビルド)。HTTP contract 0.14
- Ollama 0.40.16、`qwen3-embedding:0.6b`
- 確認は `OMNIGRAPH_HOME=spike/.home` で行い、利用者の `~/.omnigraph` には触れていない

## サーバーの立て方 (確認 4)

手順 1 から 5 は実際に行った。「cluster 設定からしか起動できない」「スキーマの変更は main だけ」は skill の記述による (試していない)。

サーバーは cluster 設定からしか起動できない。単独のグラフを公開するモードはない。

```
spike/cluster/cluster.yaml   # グラフ zettelkasten、スキーマ、埋め込みプロバイダ
spike/cluster/schema.pg
```

1. `omnigraph cluster validate --config ./cluster`
2. `omnigraph cluster apply --target ./data --config ./cluster --as <actor> --json` でグラフを作る。`./data` は先に作っておく (ないと `storage_root_invalid`)
3. apply が持ったままのロックを外す: `omnigraph cluster status --target ./data --json` の `lock_id` を `omnigraph cluster force-unlock <lock_id> --target ./data`。status の JSON は複数行に整形されて出るので、1 行を前提に切り出さない (外し忘れると、サーバーは `state_lock_held` で起動しない)
4. `omnigraph-server --cluster file:///<絶対パス>/data --bind 127.0.0.1:8080 --unauthenticated`
5. CLI からは operator 設定の server target (`kind: server`、`endpoint: http://127.0.0.1:8080`、`auth: {provider: anonymous}`、`default_graph: zettelkasten`) で使う。HTTP の平文は loopback の anonymous のときだけ許される

- 待ち受けは `--bind` で決める。既定値は `127.0.0.1:8080` (2026-11-20 に `--bind` なしで起動して確かめた。環境変数 `OMNIGRAPH_LISTEN_ADDR` でも変えられる)。既定値に頼らず、launchd では `--bind 127.0.0.1:8080` を明示する
- 埋め込みプロバイダは cluster.yaml の `providers.embedding` に書き、グラフの `embedding_provider` で結ぶ。`api_key` は `${環境変数}` で必須なので、Ollama でも `OLLAMA_API_KEY=ollama` などのダミーを渡す
- スキーマの変更は main だけで行い、生きているブランチが 1 本のときにしかできない (skill の記述)。スキーマを変えるときは review ブランチをいったん消す

## 確認 1: できる操作

CLI を server target に向けると、次はすべて動いた。HTTP を直接呼ぶ形は試していない (CLI が HTTP を使うことは skill の記述による)。

| 操作 | コマンド | 結果 |
|---|---|---|
| ノードとエッジの書き込み | `omnigraph load --data f.jsonl --mode merge --json` | Literature、仮の Source、Cites を 1 回で入れられた |
| `@card(1..1)` | 同上 | Cites のない Literature は `@card violation on edge Cites: source '...' has 0 edges (min 1)` で拒否 (400) |
| ブランチの作成 | `omnigraph mutate -e 'branch create review from main'` | `outcome.kind: created` |
| ブランチへの書き込み | `load ... --branch review` | 動いた (確認 2 の条件つき) |
| 差分 | `omnigraph branch diff review --into main --json` | review で足したエッジが `changes[]` に `op: insert` で出る |
| マージ | `omnigraph mutate -e 'branch merge review into main'` | main が先に進んでいても `merge: merged` |
| 書き出し | `omnigraph export` (`--json` はない。JSONL を標準出力に出す) | ノードとエッジが 1 行ずつ |
| ベクトル検索 | `query` の `order { nearest($p.embedding, $q) } limit N` | 確認 3 |

## 確認 2: review ブランチ

- review を main から作ったあとで main に足した Permanent へのリンクを、review に書くと拒否される: `__src 'p5' not found in Permanent` (400)。ブランチは作った時点の main しか見ない
- `branch merge main into review` で main を review に取り込むと (`fast_forward`)、書けるようになる
- review の内容を main にマージするとき、main がその間に進んでいても `merged` になる

結論: review は常にある 1 本のブランチでよさそうだが、この時点で試したのは 1 往復だけだった。2 回目以降の往復、却下したエッジを消してからのマージ、同じリンクの重複は、下の「計画 2 の確認」で確かめた (3 点ともできた)。提案を書く前に、毎回 main を review に取り込む。

## 確認 3: 埋め込みと近さ

- **書き込み時に埋め込みは自動で入らない。** load の結果に `"embedding_generation": "unsupported"` とあり、埋め込みを付けずに書いた Permanent の `embedding` は空 (null) のまま。skill にも「writes never embed automatically」とある
- 書き込む側 (zl) が Ollama の `POST /v1/embeddings` で 1024 次元のベクトルを作り、書き込みの `embedding` に入れれば入る
- `nearest` にテキストを渡すと、サーバーの埋め込みプロバイダ (cluster.yaml) が埋め込みにして検索する。ベクトルを `$v: Vector(1024)` で渡すこともできる
- `nearest(...) as distance` で距離が付く。距離は 2 乗ユークリッド距離で、ベクトルは正規化されているので 0 から 4。コサイン類似度は `1 - distance / 2`

例 (問い「小さいノートはつなぎやすい。1 枚に 1 つの考えだけを書く」):

| ノート | distance | 類似度 |
|---|---|---|
| 同じ文 | 0.000 | 1.00 |
| 短いカードは他のカードと組み合わせやすい | 0.899 | 0.55 |
| メールは短く書くと読まれやすい | 0.963 | 0.52 |
| コーヒー豆は焙煎してから二週間以内に飲む | 1.454 | 0.27 |

「メール」の文も「カード」の文とほぼ同じ近さに出た。話題が近いだけの候補がレビューに来ることはある (設計の判断どおり、人がレビューで落とす)。

## 計画 2 の確認 (2026-11-20)

手順は `spike/check2.sh`、出力の全文は `spike/check2.out` (git には入れない)。スキーマの `Links` と `Includes` に `@key(@src, @dst)` を足し (本体の先頭)、`spike/data` と `spike/.home` を消して、上の手順 1 から 5 でグラフを作り直したあとで行った。埋め込みは使っていない (Permanent の `embedding` は null のまま)。サーバーは確認のあとで止めた。

### 項目 1: 2 回目の往復 (できた)

以下の `{"kind":"merged",...}` は、`mutate ... --json` の出力の `outcome` キーだけを抜き出したもの (全体の形は項目 6)。

1. review に p1 -> p2 を書き、review を main にマージ: `{"kind":"merged","source":"review","target":"main","merge":"fast_forward"}`。main に p1 -> p2 が入った
2. main に p3 を足し、取り込む前に review へ p3 -> p1 を書くと拒否される: `{"error":"__src 'p3' not found in Permanent","code":"bad_request",...,"http_status":400}`
3. `branch merge main into review` は `{"kind":"merged","source":"main","target":"review","merge":"fast_forward"}`。そのあと review に p3 -> p1 を書けた
4. 続けて main に p4 p5 を足して取り込むと `"merge":"merged"` (fast_forward ではなく、review にも書き込みがあるため通常のマージ)。そのあとも書けた

2 回目以降の往復も問題ない。提案を書く前に毎回 `branch merge main into review` をすればよい。

### 項目 2: 却下 (できた)

review に p3 -> p4 と p3 -> p5 を書き、1 本を消した。

```
omnigraph mutate --branch review -e 'query del_link($a: String, $b: String) { delete Links where @src = $a and @dst = $b }' --params '{"a":"p3","b":"p5"}' --json
  -> "affected_nodes": 0, "affected_edges": 1, "commit": {... "published": true ...}
```

- review から p3 -> p5 が消え、`branch merge review into main` のあと main には p3 -> p4 だけが入った (p3 -> p5 は入らない)
- `where` で `@src` と `@dst` を `and` でつなげる。`@dst` だけの `where` でも消せる (`delete Links where @dst = $b` が `affected_edges: 1`)
- 存在しない組を消しても失敗せず、`"affected_edges": 0, "commit": null` (コミットも作られない)
- 消すのは `mutate --branch review` で、delete だけのクエリにする (insert とは混ぜられない)

### 項目 3: 重複 (`@key(@src, @dst)` で 1 本になる)

- 同じ組のリンクを `load --mode merge` で 2 回書いても、`Links` の p6 -> p7 は 1 本
- 3 回目に `reason` を付けて書くと、1 本のまま `reason` が入った: `[{"src":"p6","dst":"p7","reason":"同じ話題","sim":0.8}]`
- `mutate` の `insert Links { from: $a, to: $b, ... }` も同じ組を upsert する (`reason` と `similarity` が新しい値に置き換わる。1 本のまま)
- `reason` が null の行では、`query --json` の行にそのキーが出ない (項目 6)

結論: `Links` と `Includes` に `@key(@src, @dst)` を付ける。load の JSONL に `id` は書かなくてよい (導出される)。同じ組をもう一度提案すると、既存のエッジの `similarity` と `reason` が上書きされる。上書きしたくない場合 (すでに採用したリンクの `reason` を消さない) は、zl が提案の前に既存のリンクを調べて除く。

### 項目 4: 削除

| 操作 | 結果 |
|---|---|
| `delete Literature where slug = $s` (Cites あり) | 通った。`"affected_nodes": 1, "affected_edges": 1`。Cites は **一緒に消える** (export に Cites の行が残らない)。Source は残る |
| Cites だけを `delete Cites where @src = $s` | **拒否される**: `@card violation on edge Cites: source 'lit2' has 0 edges (min 1)` (400)。Literature を残したまま Cites だけは消せない |
| `delete Fleeting where slug = $s` (DerivedFrom なし) | 通った。`"affected_nodes": 1, "affected_edges": 0` |
| `delete Fleeting where slug = $s` (DerivedFrom あり、参考) | 通った。`"affected_edges": 1`。DerivedFrom は一緒に消える |
| `delete Permanent where slug = $s` (Links あり、参考) | 通った。`"affected_edges": 1`。Links は一緒に消える |

結論: ノードを消すと、そのノードにつながるエッジも一緒に消える。「Cites を消してから Literature を消す」手順は要らず (そもそも Cites だけは消せない)、`delete Literature where slug = $s` の 1 回でよい。Task 6 の `delete_notes` は、ノードの種類ごとに delete だけのクエリを 1 回ずつ流す。DerivedFrom を持つ Fleeting も消せるので、「派生元の Fleeting は消せない」といった判定は zl の側で決める。

### 項目 6: 出力の形

`query --json` の標準出力。行は `rows` に入る。キーは `return` の式の文字列 (`$p.slug` なら `p.slug`)、`as` を付ければその名前。null の列はキーごと出ない。

```
$ omnigraph ... query -e 'query q() { match { $p: Permanent { slug: "p1" } } return { $p.slug, $p.title } }' --json
{
  "query_name": "q",
  "target": {"branch": "main", "snapshot": null},
  "row_count": 1,
  "columns": ["p.slug", "p.title"],
  "rows": [{"p.slug":"p1","p.title":"p1"}],
  "graph_commit_id": "hb1.01M7VAR9K83HNDCHA8385M47KH.17.01M7VARDNTBQMVBZG45GEZFA0S"
}
```

行がないときは `"row_count": 0, "rows": []` (終了コード 0)。標準エラーには `target local (explicit): server http://127.0.0.1:8080/; graph=zettelkasten; config=...` の 1 行が出る (成功時も出る)。

`omnigraph query -e 'branch list' --json`:

```
{
  "query_name": "branch list",
  "target": {"branch": null, "snapshot": null},
  "row_count": 2,
  "columns": ["name"],
  "rows": [{"name":"main"},{"name":"review"}]
}
```

`graph_commit_id` は付かない。ブランチの一覧は `rows[].name`。

書き込みの出力 (すべて標準出力、JSON):

- `load --json`: `{"branch":"main","base_branch":null,"branch_created":false,"mode":"merge","nodes":[{"name":"Permanent","entities_loaded":1}],"edges":[],"total_entities":1,"embedding_generation":"unsupported","actor_id":null,"commit":{"graph_commit_id":"...","graph_branch":null,"graph_manifest_version":2,...,"published":true,"kind":"published"}}`
- `mutate -e 'query ...' --json` (delete / insert): `{"branch":"main","query_name":"dl","affected_nodes":1,"affected_edges":1,"actor_id":null,"commit":{...}}`。何も変わらなかったときは `"commit": null`
- `mutate -e 'branch create review from main' --json`: `{"branch":"review","query_name":"branch create","affected_nodes":0,"affected_edges":0,"actor_id":null,"commit":null,"outcome":{"kind":"created","from":"main","name":"review","from_commit_id":"hb1...."}}` (`branch` は作ったブランチ名)
- `mutate -e 'branch merge main into review' --json`: `{"branch":"review","query_name":"branch merge","affected_nodes":0,"affected_edges":0,"actor_id":null,"commit":{...},"outcome":{"kind":"merged","source":"main","target":"review","merge":"merged"}}`。`outcome.merge` は `fast_forward` か `merged`。`branch` はマージ先。`commit` は新しいコミットを作ったときだけ入る (fast_forward のときは省略されるか null。確かめていない)
- `mutate -e 'branch delete tmpb'` は、server target では `--yes` を付けないと拒否される: `{"error":"refusing destructive `branch delete` against non-local target ... pass --yes to confirm ...","disposition":{"effects":"none","action":"refresh"}}` (`code` なし、終了コード 1)
- `query -e 'branch diff review into main' --json` の行は `{"kind":"edge","type":"Links","id":"[\"p3\",\"p4\"]","op":"delete","before":{"properties":{"reason":null,"similarity":0.8},"endpoints":{"from":"p3","to":"p4"}}}` の形 (`op` は `insert` / `delete` など、`id` は JSON 配列の文字列)。`page` も付く。この例は、main にだけあって review で消えているリンクを `delete` として示す
- 標準出力の JSON は、整形されて複数行に出るものと 1 行のものがある。1 つの JSON として読めばよい

### 項目 5: サーバーを止めたときの終了コードと出力

Task 2 で Transient (つなぎ直せば通る) と Permanent (直さないと通らない) を分けるための記録。エラーの JSON は標準出力に出る (`--json` あり)。`--json` なしでは標準エラーに文で出る。標準エラーの 1 行目 `target local (explicit): ...` は成功時にも出る。

**サーバー停止中** (`load`、`query`、`mutate`、`export` のどれも同じ):

```
exit: 1
stdout:
{
  "error": "server discovery connection failed (DNS, TCP, proxy, or TLS); could not establish omnigraph-http-api: 0.14; this data request was not sent",
  "code": "api_contract_mismatch",
  "disposition": {"effects": "none", "action": "fix"},
  "request_dispatched": false
}
stderr (--json あり):
target local (explicit): server http://127.0.0.1:8080/; graph=zettelkasten; branch=main; config=...
```

`--json` なしの標準エラー:

```
server discovery connection failed (DNS, TCP, proxy, or TLS); could not establish omnigraph-http-api: 0.14; this data request was not sent
disposition: {"effects":"none","action":"fix"}
```

**サーバーが動いているときの拒否** (比較):

| 場合 | 終了コード | `--json` の標準出力 |
|---|---|---|
| `load` の `@card` 違反 | 2 | `{"error":"@card violation on edge Cites: source 'lit3' has 0 edges (min 1)","code":"bad_request","disposition":{"effects":"unknown","action":"reconcile"},"http_status":400}` |
| `load` で存在しないノードへのリンク | 2 | `{"error":"__dst 'nonexist' not found in Permanent","code":"bad_request","disposition":{"effects":"unknown","action":"reconcile"},"http_status":400}` |
| `query` で存在しないブランチ | 1 | `{"error":"branch 'nobranch' not found","code":"not_found","disposition":{"effects":"none","action":"fix"},"http_status":404}` |
| `query` の構文エラー | 1 | `{"error":"parse error: expected variable","code":"bad_request","disposition":{"effects":"none","action":"fix"},"diagnostic":{"code":"Q001",...},"http_status":400}` |

`--json` なしの `@card` 違反の標準エラー (終了コード 2):

```
@card violation on edge Cites: source 'lit3' has 0 edges (min 1)
HTTP 400
disposition: {"effects":"unknown","action":"reconcile"}
```

見分け方 (`--json` で標準出力の JSON を見る):

- 接続できなかった: `request_dispatched` が `false`、`code` が `api_contract_mismatch`、`http_status` がない。終了コードは `load`、`query` とも 1。サーバーに届いていない (`disposition.effects` が `none`) ので、書き込みは起きていない。Transient として再試行してよい
- サーバーが拒否した (4xx): `http_status` がある。`request_dispatched` はない。`@card` 違反は終了コード 2 (`load` の 400 は 2、`query` の 400 と 404 は 1) なので、**終了コードだけでは見分けられない**。`code` と `http_status` で分ける。400 と 404 は Permanent (データか指示を直さないと通らない)
- `load` の 400 の `disposition.effects` は `unknown`、`action` は `reconcile` となる。これは「書き込みが起きたか分からない」という一般的な注意で、400 の `@card` 違反や存在しないノードへのリンクでは実際には何も書かれていない (拒否されたデータは入っていない)。Task 2 ではこれを理由に再試行しない
- 注意: 接続失敗の `code` が `api_contract_mismatch` という名前なので、`code` の値だけで Permanent と決めつけない。`request_dispatched == false` を先に見る
- 5xx は試していない (起こし方がない)。`http_status` が 500 番台なら Transient に倒すのが安全だが、未確認

終了コードのまとめ: 成功 0、接続失敗 1、`query` の 4xx は 1、`load` の 4xx は 2。
