# omnigraph-server の確認 (2026-11-20)

設計書 `docs/superpowers/specs/2026-11-19-zettelist-design.md` の「最初の確認」の結果。手順は `spike/check.sh`、設定は `spike/cluster/`、出力の例は本文に抜き出した。

## 環境

- omnigraph 0.15.0 (flake input `github:ModernRelay/omnigraph/v0.15.0`、ソースからビルド)。HTTP contract 0.14
- Ollama 0.40.16、`qwen3-embedding:0.6b`
- 確認は `OMNIGRAPH_HOME=spike/.home` で行い、利用者の `~/.omnigraph` には触れていない

## サーバーの立て方 (確認 4)

サーバーは cluster 設定からしか起動できない。単独のグラフを公開するモードはない。

```
spike/cluster/cluster.yaml   # グラフ zettelkasten、スキーマ、埋め込みプロバイダ
spike/cluster/schema.pg
```

1. `omnigraph cluster validate --config ./cluster`
2. `omnigraph cluster apply --target ./data --config ./cluster --as <actor> --json` でグラフを作る
3. apply が持ったままのロックを外す: `omnigraph cluster status --target ./data --json` の `lock_id` を `omnigraph cluster force-unlock <lock_id> --target ./data`
4. `omnigraph-server --cluster file:///<絶対パス>/data --bind 127.0.0.1:8080 --unauthenticated`
5. CLI からは operator 設定の server target (`kind: server`、`endpoint: http://127.0.0.1:8080`、`auth: {provider: anonymous}`、`default_graph: zettelkasten`) で使う。HTTP の平文は loopback の anonymous のときだけ許される

- 待ち受けは `--bind` で決める。既定値は確かめていないので、launchd では必ず `--bind 127.0.0.1:8080` を付ける
- 埋め込みプロバイダは cluster.yaml の `providers.embedding` に書き、グラフの `embedding_provider` で結ぶ。`api_key` は `${環境変数}` で必須なので、Ollama でも `OLLAMA_API_KEY=ollama` などのダミーを渡す
- スキーマの変更は main だけで行い、生きているブランチが 1 本のときにしかできない (skill の記述)。スキーマを変えるときは review ブランチをいったん消す

## 確認 1: できる操作

CLI を server target に向けると、次はすべて動いた。CLI はサーバーの HTTP (`POST /query`、`POST /mutate` など) を呼んでいる。

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

結論: review は常にある 1 本のブランチでよい。提案を書く前に、毎回 main を review に取り込む。

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
