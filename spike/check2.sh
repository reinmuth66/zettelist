#!/bin/sh
# 計画 2 の確認 (設計書 2026-11-19-zettelist-design.md の「まだ確かめていないこと」)。使い捨て。
# 前提: spike/ で omnigraph-server が 127.0.0.1:8080 で動いている (spike/.server.pid に PID)。
#       spike/data と spike/.home を作り直した直後の、空のグラフであること。
# nix develop の中で実行する。最後 (項目 5) でサーバーを止める。
set -u
cd "$(dirname "$0")"
export OMNIGRAPH_HOME="$PWD/.home"
OG="omnigraph --operator-config $PWD/operator.yaml --target local"
TMP="$PWD/.tmp"; mkdir -p "$TMP"

say() { printf '\n==== %s\n' "$*"; }
# コマンドの標準出力、標準エラー、終了コードを分けて見せる
run() { # $@: command
  "$@" > "$TMP/o" 2> "$TMP/e"; rc=$?
  printf -- '--- cmd: %s\n--- exit: %s\n--- stdout:\n' "$*" "$rc"; cat "$TMP/o"
  printf -- '--- stderr:\n'; cat "$TMP/e"
}
perm() { # $1 slug -> JSONL 1 行 (埋め込みなし)
  printf '{"type":"Permanent","data":{"slug":"%s","title":"%s","body":"body %s","created":"2026-11-20T10:00:00.000Z"}}\n' "$1" "$1" "$1"
}
link() { # $1 src $2 dst [$3 reason]
  if [ $# -ge 3 ]; then
    printf '{"edge":"Links","from":"%s","to":"%s","data":{"similarity":0.8,"reason":"%s"}}\n' "$1" "$2" "$3"
  else
    printf '{"edge":"Links","from":"%s","to":"%s","data":{"similarity":0.8}}\n' "$1" "$2"
  fi
}
load() { # $1 file [$2 branch]
  if [ $# -ge 2 ]; then $OG load --data "$1" --branch "$2" --mode merge --json 2>&1 | grep -v '^target' | tr -d '\n ' | head -c 400; echo
  else $OG load --data "$1" --mode merge --json 2>&1 | grep -v '^target' | tr -d '\n ' | head -c 400; echo
  fi
}
links_on() { # $1 branch -> 全 Links を slug の組で
  $OG query --branch "$1" -e 'query q() { match { $a: Permanent $a $w:links $b } return { $a.slug as src, $b.slug as dst, $w.reason as reason, $w.similarity as sim } order { $a.slug, $b.slug } }' --json 2>/dev/null | jq -c '.rows'
}
perms_on() {
  $OG query --branch "$1" -e 'query q() { match { $p: Permanent } return { $p.slug } order { $p.slug } }' --json 2>/dev/null | jq -c '[.rows[].slug]'
}
DEL_LINK='query del_link($a: String, $b: String) { delete Links where @src = $a and @dst = $b }'

say "0. 準備: main に p1 p2 を入れ、review を作る"
{ perm p1; perm p2; } > "$TMP/base.jsonl"
load "$TMP/base.jsonl"
$OG mutate -e 'branch create review from main' --json 2>&1 | grep -v '^target' | jq -c '.outcome'
perms_on main

say "1. 2 回目の往復"
echo "-- 1 回目の往復: review に p1->p2 を書き、review を main にマージ"
link p1 p2 > "$TMP/l.jsonl"; load "$TMP/l.jsonl" review
$OG mutate -e 'branch merge review into main' --json 2>&1 | grep -v '^target' | jq -c '.outcome'
echo "main の Links:"; links_on main
echo "-- main に p3 を足す"
perm p3 > "$TMP/p3.jsonl"; load "$TMP/p3.jsonl"
echo "-- p3 を取り込む前に review に p3->p1 を書く (拒否されるはず)"
link p3 p1 > "$TMP/l.jsonl"; load "$TMP/l.jsonl" review
echo "-- branch merge main into review"
$OG mutate -e 'branch merge main into review' --json 2>&1 | grep -v '^target' | jq -c '.outcome'
echo "-- 取り込んだあとで review に p3->p1 を書く"
load "$TMP/l.jsonl" review
echo "review の Links:"; links_on review
echo "review の Permanent:"; perms_on review

say "2. 却下"
echo "-- main に p4 p5 を足し、review に取り込み、review に p3->p4 と p3->p5 を書く"
{ perm p4; perm p5; } > "$TMP/p45.jsonl"; load "$TMP/p45.jsonl"
$OG mutate -e 'branch merge main into review' --json 2>&1 | grep -v '^target' | jq -c '.outcome'
{ link p3 p4; link p3 p5; } > "$TMP/l.jsonl"; load "$TMP/l.jsonl" review
echo "review の Links (3 本のはず: p1->p2 は main から、p3->p1、p3->p4、p3->p5 で 4 本):"; links_on review
echo "-- p3->p5 を review から消す (where の @src と @dst を and でつなぐ)"
$OG mutate --branch review -e "$DEL_LINK" --params '{"a":"p3","b":"p5"}' --json 2>&1 | grep -v '^target' | head -c 600; echo
echo "review の Links:"; links_on review
echo "-- review を main にマージ"
$OG mutate -e 'branch merge review into main' --json 2>&1 | grep -v '^target' | head -c 600; echo
echo "main の Links:"; links_on main
echo "-- 参考: 存在しない組を消すとどうなるか"
$OG mutate --branch review -e "$DEL_LINK" --params '{"a":"p3","b":"nothing"}' --json 2>&1 | grep -v '^target' | head -c 600; echo
echo "-- 参考: @dst だけで消せるか (and なしの where)"
$OG mutate --branch review -e 'query d($b: String) { delete Links where @dst = $b }' --params '{"b":"p4"}' --json 2>&1 | grep -v '^target' | head -c 600; echo
echo "review の Links:"; links_on review

say "3. 重複"
{ perm p6; perm p7; } > "$TMP/p67.jsonl"; load "$TMP/p67.jsonl"
link p6 p7 > "$TMP/l.jsonl"
echo "-- 1 回目 (reason なし)"; load "$TMP/l.jsonl"
echo "-- 2 回目 (同じもの)"; load "$TMP/l.jsonl"
echo "main の Links (p6 の分):"; links_on main | jq -c '[.[] | select(.src=="p6")]'
link p6 p7 "同じ話題" > "$TMP/l.jsonl"
echo "-- 3 回目 (reason を付ける)"; load "$TMP/l.jsonl"
echo "main の Links (p6 の分):"; links_on main | jq -c '[.[] | select(.src=="p6")]'
echo "-- mutate の insert でも同じか"
$OG mutate -e 'query ins($a: String, $b: String) { insert Links { from: $a, to: $b, similarity: 0.5, reason: "mutate" } }' --params '{"a":"p6","b":"p7"}' --json 2>&1 | grep -v '^target' | head -c 400; echo
echo "main の Links (p6 の分):"; links_on main | jq -c '[.[] | select(.src=="p6")]'

say "4. 削除"
cat > "$TMP/lit.jsonl" <<'EOF'
{"type":"Source","data":{"id":"isbn:9784000000001","slug":"","title":"S","kind":"other","input":"x","provenance":"{}"}}
{"type":"Literature","data":{"slug":"lit1","body":"文献メモ","created":"2026-11-20T10:00:00.000Z"}}
{"edge":"Cites","from":"lit1","to":"isbn:9784000000001","data":{"locator":"p.1"}}
{"type":"Fleeting","data":{"slug":"fl-plain","body":"素の走り書き","created":"2026-11-20T10:00:00.000Z"}}
{"type":"Fleeting","data":{"slug":"fl-derived","body":"派生された走り書き","created":"2026-11-20T10:00:00.000Z"}}
{"edge":"DerivedFrom","from":"p6","to":"fl-derived","data":{}}
EOF
load "$TMP/lit.jsonl"
echo "-- 削除前の export (Cites / DerivedFrom / Links の行)"
$OG export 2>/dev/null | grep -E '"(Cites|DerivedFrom)"' | head -5
echo "-- delete Literature (Cites つき)"
$OG mutate -e 'query dl($s: String) { delete Literature where slug = $s }' --params '{"s":"lit1"}' --json 2>&1 | grep -v '^target' | head -c 800; echo
echo "Literature:"; $OG query -e 'query q() { match { $l: Literature } return { $l.slug } }' --json 2>/dev/null | jq -c '.rows'
echo "-- 削除後の export (Cites の行が残っているか)"
$OG export 2>/dev/null | grep -E '"(Cites)"' | head -5; echo "(Cites 行数: $($OG export 2>/dev/null | grep -c '"Cites"'))"
echo "-- Source 側から Cites をたどる"
$OG query -e 'query q() { match { $l: Literature $l cites $s } return { $l.slug, $s.id } }' --json 2>/dev/null | jq -c '.rows'
echo "-- 参考: Cites を先に消してから Literature を消す手順 (lit2 で)"
cat > "$TMP/lit2.jsonl" <<'EOF'
{"type":"Literature","data":{"slug":"lit2","body":"文献メモ 2","created":"2026-11-20T10:00:00.000Z"}}
{"edge":"Cites","from":"lit2","to":"isbn:9784000000001","data":{"locator":"p.2"}}
EOF
load "$TMP/lit2.jsonl"
$OG mutate -e 'query dc($s: String) { delete Cites where @src = $s }' --params '{"s":"lit2"}' --json 2>&1 | grep -v '^target' | head -c 600; echo
$OG mutate -e 'query dl($s: String) { delete Literature where slug = $s }' --params '{"s":"lit2"}' --json 2>&1 | grep -v '^target' | head -c 600; echo
echo "(Cites 行数: $($OG export 2>/dev/null | grep -c '"Cites"'))"
echo "-- delete Fleeting (DerivedFrom なし)"
$OG mutate -e 'query df($s: String) { delete Fleeting where slug = $s }' --params '{"s":"fl-plain"}' --json 2>&1 | grep -v '^target' | head -c 600; echo
echo "-- 参考: delete Fleeting (DerivedFrom あり)"
$OG mutate -e 'query df($s: String) { delete Fleeting where slug = $s }' --params '{"s":"fl-derived"}' --json 2>&1 | grep -v '^target' | head -c 600; echo
echo "Fleeting:"; $OG query -e 'query q() { match { $f: Fleeting } return { $f.slug } }' --json 2>/dev/null | jq -c '.rows'
echo "(DerivedFrom 行数: $($OG export 2>/dev/null | grep -c '"DerivedFrom"'))"
echo "-- 参考: delete Permanent (Links を持つ p7)"
$OG mutate -e 'query dp($s: String) { delete Permanent where slug = $s }' --params '{"s":"p7"}' --json 2>&1 | grep -v '^target' | head -c 600; echo
echo "Permanent:"; perms_on main
echo "(Links の行: $($OG export 2>/dev/null | grep '"Links"' | grep -c '"p7"') 本が p7 を含む)"
echo "main の Links (p6 の分):"; links_on main | jq -c '[.[] | select(.src=="p6")]'

say "6. 出力の形 (サーバーが動いている間に)"
echo "-- query --json (行あり)。標準出力と標準エラーを分ける"
run $OG query -e 'query q() { match { $p: Permanent { slug: "p1" } } return { $p.slug, $p.title } }' --json
echo "-- query --json (行なし)"
run $OG query -e 'query q() { match { $p: Permanent { slug: "none" } } return { $p.slug } }' --json
echo "-- query -e 'branch list' --json"
run $OG query -e 'branch list' --json
echo "-- load --json (成功)"
echo '{"type":"Permanent","data":{"slug":"p8","title":"p8","body":"b","created":"2026-11-20T10:00:00.000Z"}}' > "$TMP/p8.jsonl"
run $OG load --data "$TMP/p8.jsonl" --mode merge --json
echo "-- mutate --json (成功)"
run $OG mutate -e 'query dp($s: String) { delete Permanent where slug = $s }' --params '{"s":"p8"}' --json

say "5a. サーバーが動いているときの @card 違反と、そのほかの 4xx の出力 (比較用)"
echo '{"type":"Literature","data":{"slug":"lit3","body":"出典なし","created":"2026-11-20T10:00:00.000Z"}}' > "$TMP/card.jsonl"
run $OG load --data "$TMP/card.jsonl" --mode merge --json
echo "-- 同じものを --json なしで"
run $OG load --data "$TMP/card.jsonl" --mode merge
echo "-- 存在しないノードへのリンク (400 のはず)"
link p1 nonexist > "$TMP/l.jsonl"
run $OG load --data "$TMP/l.jsonl" --mode merge --json
echo "-- 存在しないブランチ"
run $OG query --branch nobranch -e 'query q() { match { $p: Permanent } return { $p.slug } }' --json
echo "-- 構文エラーのクエリ"
run $OG query -e 'query q( { broken' --json

say "5b. サーバーを止める"
kill "$(cat .server.pid)" 2>/dev/null
i=0; while kill -0 "$(cat .server.pid)" 2>/dev/null && [ $i -lt 20 ]; do sleep 0.5; i=$((i+1)); done
kill -0 "$(cat .server.pid)" 2>/dev/null && echo "サーバーがまだ生きている" || echo "サーバー停止"

say "5c. 停止中の load / query"
echo "-- query --json"
run $OG query -e 'query q() { match { $p: Permanent } return { $p.slug } }' --json
echo "-- query (--json なし)"
run $OG query -e 'query q() { match { $p: Permanent } return { $p.slug } }'
echo "-- load --json (正常なデータ)"
run $OG load --data "$TMP/p8.jsonl" --mode merge --json
echo "-- load (--json なし)"
run $OG load --data "$TMP/p8.jsonl" --mode merge
echo "-- load --json (@card 違反のデータ)"
run $OG load --data "$TMP/card.jsonl" --mode merge --json
echo "-- mutate --json"
run $OG mutate -e 'branch list' --json
echo "-- branch list (query -e)"
run $OG query -e 'branch list' --json
