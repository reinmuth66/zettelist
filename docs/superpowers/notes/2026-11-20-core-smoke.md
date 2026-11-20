# 手元のサーバーでの通し確認 (2026-11-20)

計画 3 の Task 8 (計画 2 の Task 12 を置き換えたもの) の記録。

## 環境

- 作業用の一時フォルダにグラフ、状態、設定、バックアップ用の git リポジトリを置いた。利用者の `~/.omnigraph`、`~/.local/share/zettelist`、運用のリポジトリには触れていない。終わったあとで全部消した
- omnigraph-server は `--bind 127.0.0.1:8090 --unauthenticated`。グラフは `schema/` の cluster 設定から `cluster apply` で作った (データのフォルダは先に作っておく必要がある。ないと `storage_root_invalid`)
- Ollama は `nix develop -c ollama serve`、モデルは `qwen3-embedding:0.6b`
- 設定: `link.min_similarity = 0.5`、`link.inbox_similarity = 0.5`、`backup.push = false`
- zl は `target/debug/zl` (feat/core の 8fab26a 時点、worker は実際に起動させた)

## 手順と結果

1. メモを 4 件書いた (1 件は `-s 978-4-480-06899-8 -p p.42`)。どれも `保存しました <slug>` で、数秒後には待ち行列も失敗も空。Literature は `source_id: isbn:9784480068998` を持つ
2. `zl --similar <「小さいノート…」> --json`: inbox の「箇条書きは考えを止める」0.59、「短いカード…」0.55。コーヒーの文は 0.5 未満で出なかった
3. 「短いカード…」と「小さいノート…」を順に `--promote`。2 回目の Permanent から 1 回目への Links の提案が review に出た (近さ 0.52)
4. `--adopt <key> --reason "粒度の話として同じ"`、`--merge --yes --json` は `{"hubs":0,"links":1}`。worker がバックアップを `merge: links 1` で commit した (JSONL は 11 行)。review の提案は空、inbox は昇格していない 2 件だけ

書き込みから埋め込みが入るまでと、昇格から提案が出るまでは、どちらも数秒だった。

## 気づいたこと

- Omnigraph Studio から作業用のサーバーを開けなかった。`omnigraph studio --target zl` のリンクは出たが、Studio は自分の設定フォルダの接続先を読むため、作業用の operator 設定は使われない。Studio の画面で接続先を足そうとすると name と bearer token を求められ、足せなかった (認証なしのサーバーでも任意の token で HTTP は通ることは確かめた)。本番で launchd から常駐させるときに、Studio からの接続方法 (token を設定するか、`~/.omnigraph` に接続先を置くか) を決める必要がある
- `cluster apply` の前にデータのフォルダを作る必要がある (確認の記録の手順に足すとよい)
