---
name: zettelist
description: zl (zettelist) コマンドで Zettelkasten を操作する。メモを残す、inbox (未整理のメモ) を見る、メモを読む、似ているノートを探す、Permanent ノートへ昇格する、ノートを消す、リンクやハブの提案をレビューして採用・却下・マージする、失敗したジョブをやり直す、といった話が出たら、利用者が zl や zettelist と言わなくても、Zettelkasten、メモの整理、inbox、昇格、提案のレビューが話題の時点で必ずこの skill を使うこと。
---

# zettelist (zl)

Omnigraph 上の Zettelkasten をコマンドラインから操作する。サブコマンドはない。`zl "<本文>"` は必ずメモで、操作はすべて `--` で始まるオプション。操作のオプションは同時に 1 つだけで、本文とも組み合わせられない。

エージェントは毎回 `--json` を付ける。成功は標準出力に JSON 1 つ、失敗は標準エラーに JSON が出るので、出力を機械的に読める。

## 方針

判断を誰が下すかは利用者の運用で決まる。エージェントに任せると言われていなければ、次の操作の前に利用者に確かめる。昇格、提案の採用・却下、マージ、削除。理由は、どれも利用者の知識の整理そのものを変える操作で、`--delete` `--merge` `--discard` は取り消せないから。`--yes` は、利用者が了承してから付ける。

Permanent の題と本文は、昇格元の内容を自分の言葉でまとめたものにする。元のメモの写しでは、整理した意味がなくなる。

## メモを書く

```
zl --json "本文"
zl --json -s <出典> [-p <位置>] "本文"
zl --json -- "--で始まる本文"
```

- `-s` は出典で、ISBN、DOI、arXiv ID、URL、登録済みのスラッグのどれか。`-p` は出典の中の位置 (ページなど) で、`-s` と一緒に使う。
- `-s` `-p` は操作のオプションと組み合わせられない。本文が `-` で始まるときは `--` のあとに置く。
- 成功すると `{"slug": "..."}`。
- サーバーに届かない、またはサーバーが拒否したときは、メモを待ち行列に保存して `{"queued": true}` を返す (終了コード 0)。メモは失われず、worker があとで書くので、書き直さない。
- 本文が空、出典が未登録などの入力の誤りは、保存せず終了コード 2。

## 読む

| コマンド | 出力 (`--json`) |
|---|---|
| `zl --json --inbox` | `[{"kind":"fleeting\|literature","slug","body","created":"<RFC3339 UTC>","old":bool,"source_id":str\|null,"source_slug":str\|null,"locator":str\|null}]` |
| `zl --json --show <slug>` | `{"kind":"fleeting\|literature\|permanent","slug","title":str\|null,"body","created"}` |
| `zl --json --similar <slug>` | `{"inbox":[{"kind","slug","body","similarity"}],"permanent":[{"slug","title","body","similarity"}]}` |

- `--inbox` は、まだ昇格していないメモの一覧。`old` が true のものは、長く手つかずのまま残っている。
- slug は種類をまたいで重なることがまれにある。そのとき `--show` は Fleeting、Literature、Permanent の順で最初の 1 件を返す。
- `--similar` は、埋め込みがまだ作られていないノートでは両方とも空になる。少し待ってからやり直す。

## 昇格と削除

```
zl --json --promote <slug,...> --title <題> (--body <本文> | --body-file <パス>)
zl --json --delete <slug,...> --yes
```

- `--promote` は、inbox のノート (Fleeting、Literature) を 1 件以上、カンマで区切って渡し、Permanent を 1 つ作る。`--title` と、`--body` か `--body-file` が要る。成功すると `{"slug": "<Permanent の slug>"}`。
- Permanent の slug や昇格済みのノートが混ざっていると、何も書かずに終了コード 2。
- 昇格のあと、リンク提案のジョブを待ち行列に積めなかったときは、標準エラーに `{"warning": "...", "slug": "..."}` が出る。昇格は済んでいて、標準出力は `{"slug": ...}`、終了コードは 0。
- `--delete` は `--yes` がないと何もせず終了コード 2。知らない slug が 1 件でもあれば、何も消さない。成功すると `{"ok": true}`。

## レビュー

```
zl --json --review
zl --json --adopt <キー> [--reason <理由>]
zl --json --reject <キー>
zl --json --undecide <キー>
zl --json --merge --yes
```

`--review` の出力:

```
[{"key":"link:a:b","type":"link","from":"a","to":"b","similarity":0.82,"decision":"adopt|reject|null","reason":str|null},
 {"key":"hub:h:n","type":"hub","hub":"h","note":"n","decision":"adopt|reject|null","reason":str|null}]
```

- `key` を `--adopt` `--reject` `--undecide` に渡す。`decision` が `null` のものは未判断。`--reason` は `--adopt` にだけ付けられる。
- 提案は、昇格のあとに worker が裏で作る。昇格の直後は `--review` が空のことがあるので、少し待ってからもう一度見る。
- `--merge --yes` は、採用したものを main に取り込み、却下したものを捨てる。未判断の提案は取り込まれない。成功すると `{"links": 3, "hubs": 1}`。
- 存在しない key は終了コード 2 で、何も書き換えない。

## 失敗したジョブ

```
zl --json --failed
zl --json --retry <id>
zl --json --retry-all
zl --json --discard <id> --yes
```

- `--failed` の出力は `[{"id","job":"write_note|embed|promote|backup","attempts":4,"last_error":str|null}]`。
- `--retry` `--retry-all` `--discard` は成功すると `{"ok": true}`。`--discard` は `--yes` が要る。知らない id は終了コード 2。

## 典型の流れ

1. `--inbox` で未整理のメモを見る。
2. 気になるものを `--show <slug>` で読み、`--similar <slug>` で近いノートを調べる。
3. 利用者と相談して、`--promote` で Permanent にする。
4. worker が提案を作るのを待つ。
5. `--review` で提案を見て、利用者の判断で `--adopt` か `--reject` を付ける。やり直したいものは `--undecide`。
6. 判断が済んだら `--merge --yes` で main に取り込む。
7. 失敗したジョブがあれば、`--failed` で見て、`--retry` か `--discard --yes`。

## 終了コードとエラー

- 0: 成功 (待ち行列に保存した場合も含む)
- 1: 処理の失敗
- 2: 使い方の誤り、入力の誤り、`--yes` がない

失敗すると、標準エラーに `{"error": "<文言>", "kind": "invalid|permanent|transient"}` が出る。clap の使い方の誤りも同じ形。

- `invalid` (終了コード 2): 入力を直す。そのままやり直しても通らない。
- `permanent` (終了コード 1): 設定やデータの誤り。そのままやり直しても通らないので、利用者に伝える。
- `transient` (終了コード 1): サーバーに届かないなど。少し待ってやり直してよい。

結果は標準出力の JSON から読む。標準エラーの JSON を読むのは、終了コードが 0 でないときだけ。`--json` を付けても、標準エラーに人向けの文言の警告 (worker を起動できない、など) が出ることがあり、終了コード 0 の警告は結果に影響しない。

## skill の書き出し

```
zl --skill-export <dir> [--force]
```

`<dir>/SKILL.md` を書く。`<dir>` がなければ作る。すでに `SKILL.md` があるときは、`--force` がないと何も書かず終了コード 2。成功すると `{"ok": true}`。
