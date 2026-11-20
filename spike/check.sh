#!/bin/sh
# 最初の確認 (設計書 2026-11-19-zettelist-design.md の「最初の確認」)。使い捨て。
# 前提: spike/ で omnigraph-server が 127.0.0.1:8080 で動いている。nix develop の中で実行する。
set -u
cd "$(dirname "$0")"
export OMNIGRAPH_HOME="$PWD/.home"
OG="omnigraph --operator-config $PWD/operator.yaml --target local"
TMP="$PWD/.tmp"; mkdir -p "$TMP"

say() { printf '\n==== %s\n' "$*"; }

embed() { # $1: text -> JSON array of 1024 floats (Ollama, OpenAI 互換)
  printf '{"model":"qwen3-embedding:0.6b","input":%s}' "$(printf '%s' "$1" | jq -Rs .)" > "$TMP/req.json"
  ax http://localhost:11434/v1/embeddings --all -X POST -H 'content-type: application/json' -d @"$TMP/req.json" | jq -c '.body | (if type == "string" then fromjson else . end) | .data[0].embedding'
}

say "1a. Literature + 仮の Source + Cites を 1 回の load で main に入れる"
cat > "$TMP/lit.jsonl" <<'EOF'
{"type":"Source","data":{"id":"isbn:9784000000000","slug":"","title":"978-4-0000-0000-0","kind":"other","input":"978-4-0000-0000-0","provenance":"{}"}}
{"type":"Literature","data":{"slug":"20261119100000","body":"箇条書きは考えを止める","created":"2026-11-19T10:00:00.000Z"}}
{"edge":"Cites","from":"20261119100000","to":"isbn:9784000000000","data":{"locator":"p.42"}}
EOF
$OG load --data "$TMP/lit.jsonl" --mode merge --json 2>&1 | tail -15

say "1b. Literature だけ (Cites なし) は拒否されるか"
cat > "$TMP/lit2.jsonl" <<'EOF'
{"type":"Literature","data":{"slug":"20261119100001","body":"出典のない文献メモ","created":"2026-11-19T10:00:01.000Z"}}
EOF
$OG load --data "$TMP/lit2.jsonl" --mode merge --json 2>/dev/null | jq -c '{error, code}'

say "3a. 埋め込みを付けずに書いた Permanent に埋め込みが入るか"
cat > "$TMP/p0.jsonl" <<'EOF'
{"type":"Permanent","data":{"slug":"p-noembed","title":"埋め込みなし","body":"埋め込みを付けずに書いたノート","created":"2026-11-19T10:00:02.000Z"}}
EOF
$OG load --data "$TMP/p0.jsonl" --mode merge --json 2>&1 | grep -E '"commit"|error' | head -3
$OG query -e 'query q() { match { $p: Permanent { slug: "p-noembed" } } return { $p.slug, $p.embedding } }' --json 2>&1 | grep -v '^target' | head -c 800; echo

say "3b. zl が Ollama で埋め込みを作り、Permanent を 4 件入れる"
: > "$TMP/p.jsonl"
i=0
for t in "小さいノートはつなぎやすい。1 枚に 1 つの考えだけを書く" \
         "短いカードは他のカードと組み合わせやすい" \
         "メールは短く書くと読まれやすい" \
         "コーヒー豆は焙煎してから二週間以内に飲む"; do
  i=$((i+1))
  v=$(embed "$t")
  jq -cn --arg s "p$i" --arg b "$t" --argjson v "$v" \
    '{type:"Permanent",data:{slug:$s,title:$s,body:$b,created:"2026-11-19T11:00:00.000Z",embedding:$v}}' >> "$TMP/p.jsonl"
done
$OG load --data "$TMP/p.jsonl" --mode merge --json 2>&1 | grep -E '"commit"|error' | head -3

say "3c. nearest にテキストを渡す (サーバーの埋め込みプロバイダ) と距離が付くか"
$OG query -e 'query near($q: String) { match { $p: Permanent } return { $p.slug, $p.body, nearest($p.embedding, $q) as distance } order { nearest($p.embedding, $q) } limit 5 }' \
  --params "$(jq -cn --arg q '小さいノートはつなぎやすい。1 枚に 1 つの考えだけを書く' '{q:$q}')" --json 2>&1 | grep -v '^target' | head -c 1500; echo

say "3d. nearest にベクトルを渡せるか"
v=$(embed "短いメモ")
$OG query -e 'query nearv($v: Vector(1024)) { match { $p: Permanent } return { $p.slug, nearest($p.embedding, $v) as distance } order { nearest($p.embedding, $v) } limit 3 }' \
  --params "$(jq -cn --argjson v "$v" '{v:$v}')" --json 2>&1 | grep -v '^target' | head -c 1000; echo

say "2a. review ブランチを main から作る"
$OG mutate -e 'branch create review from main' --json 2>&1 | tail -8

say "2b. review を作ったあとで main に Permanent p5 を足す"
v=$(embed "ノートを短く保つと読み返しやすい")
jq -cn --argjson v "$v" '{type:"Permanent",data:{slug:"p5",title:"p5",body:"ノートを短く保つと読み返しやすい",created:"2026-11-19T12:00:00.000Z",embedding:$v}}' > "$TMP/p5.jsonl"
$OG load --data "$TMP/p5.jsonl" --mode merge --json 2>&1 | grep -E '"commit"|error' | head -3

say "2c. review に、main にしかない p5 へのリンクを書けるか"
echo '{"edge":"Links","from":"p5","to":"p1","data":{"similarity":0.9}}' > "$TMP/link.jsonl"
$OG load --data "$TMP/link.jsonl" --branch review --mode merge --json 2>&1 | tail -8

say "2d. review を main に取り込む (main -> review のマージ) ができるか"
$OG mutate -e 'branch merge main into review' --json 2>&1 | tail -8

say "2e. 取り込んだあとで review にリンクを書けるか"
$OG load --data "$TMP/link.jsonl" --branch review --mode merge --json 2>&1 | grep -E '"commit"|error|code' | head -3

say "2f. review の差分"
$OG branch diff review --into main --json 2>&1 | grep -v '^target' | head -c 1500; echo

say "2g. main がさらに進んだあとで review を main にマージ"
echo '{"type":"Fleeting","data":{"slug":"20261119130000","body":"main で書いたメモ","created":"2026-11-19T13:00:00.000Z"}}' > "$TMP/f.jsonl"
$OG load --data "$TMP/f.jsonl" --mode merge --json 2>&1 | grep -E '"commit"|error' | head -2
$OG mutate -e 'branch merge review into main' --json 2>&1 | tail -10

say "1c. export"
$OG export 2>/dev/null | head -c 600; echo

say "4. 待ち受け"
grep -o 'OMNIGRAPH_LISTEN_ADDR=.*' server.log | head -1
