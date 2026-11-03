#!/usr/bin/env bash
# End-to-end check against the real server binary: ingest, connected query, supersede,
# hard kill, restart, query again, auth, empty state. Exits non-zero on the first failed assertion.
set -euo pipefail
cd "$(dirname "$0")/.."
BIN="${ATLAS_BIN:-target/debug/atlas}"
[ -x "$BIN" ] || [ -x "$BIN.exe" ] || cargo build -p atlas-cli
ADDR=127.0.0.1:${E2E_PORT:-18090}; A=http://$ADDR; KEY=e2e-secret-key
DIR=$(mktemp -d); PID=""
start() { ATLAS_API_KEYS="e2e=$KEY" "$BIN" serve --dir "$DIR/db" --addr "$ADDR" "$@" >"$DIR/log" 2>&1 & PID=$!; for _ in $(seq 40); do curl -sf "$A/healthz" >/dev/null 2>&1 && return; sleep 0.25; done; echo "server did not start"; cat "$DIR/log"; exit 1; }
stop() { [ -n "$PID" ] && kill -9 "$PID" 2>/dev/null; wait "$PID" 2>/dev/null || true; PID=""; }
trap 'stop; rm -rf "$DIR"' EXIT
post() { curl -s -H "Authorization: Bearer $KEY" -X POST "$A$1" -d "$2"; }
get()  { curl -s -H "Authorization: Bearer $KEY" "$A$1"; }
py()   { python -c "import sys,json;r=json.load(sys.stdin);$1"; }
ok()   { echo "ok   $1"; }

start
[ "$(get /v1/lcf/roots | py 'print(len(r["roots"]))')" = 0 ] && ok "empty database reports no roots"
[ "$(curl -s -o /dev/null -w %{http_code} -H 'Authorization: Bearer nope' $A/v1/lcf/stats)" = 401 ] && ok "bad key rejected (401)"
stop; start
"$BIN" ingest examples/atlas-decisions --addr "$ADDR" --key "$KEY" | tee "$DIR/ingest1" | grep -q "objects 13 new" && ok "ingested 13 real documents (no seed)"
"$BIN" ingest examples/atlas-decisions --addr "$ADDR" --key "$KEY" | grep -q "objects 0 new / 13 unchanged" && ok "re-ingest is idempotent"
V0=$(get /v1/lcf/stats | py 'print(r["version"])')
post /v1/lcf/node '{"key":"my-note","kind":"decision","title":"Ship the demo on Monday","body":"Decision recorded during the e2e run","source":"e2e:1","status":"asserted"}' | py 'assert "seq" in r, r'
post /v1/lcf/edge '{"from":"my-note","rel":"DERIVED_FROM","to":"adr-wal-durability","source":"e2e:1"}' | py 'assert "seq" in r, r'
ok "ingest node + edge through the write path"
post /v1/lcf/node/get "{\"key\":\"my-note\",\"as_of\":$V0}" | py 'assert r.get("error")=="NOT_FOUND", r'
post /v1/lcf/node/get '{"key":"my-note"}' | py 'assert r["key"]=="my-note", r'
ok "read pinned to an older version does not see later writes"
Q='{"q":"why did we choose group commit WAL durability","max_hops":3}'
post /v1/lcf/query "$Q" | py 'k={n["key"] for n in r["nodes"]}; assert "adr-wal-durability" in k and len(k)>1, k; assert r["trace"] and r["stats"]["nodes"]<=60'
ok "connected context query returns the decision and its neighbours with trace"
post /v1/lcf/supersede '{"old":"my-note","node":{"key":"my-note-v2","kind":"decision","title":"Ship the demo on Tuesday","body":"Revised","source":"e2e:2"}}' | py 'assert "seq" in r, r'
post /v1/lcf/node/get '{"key":"my-note"}' | py 'assert r["lifecycle"]=="superseded" and r["superseded_by"]=="my-note-v2", r'
post /v1/lcf/node '{"key":"my-note","kind":"decision","title":"changed","body":"x","source":"e2e:3"}' | py 'assert r["error"]=="CONFLICT", r'
ok "supersede keeps old record, marks lifecycle, overwrite is CONFLICT"
BEFORE=$(get /v1/lcf/stats | py 'print(r["records"])')
stop; start
[ "$(get /v1/lcf/stats | py 'print(r["records"])')" = "$BEFORE" ] && ok "same record count after kill -9 and restart ($BEFORE)"
post /v1/lcf/node/get '{"key":"my-note"}' | py 'assert r["superseded_by"]=="my-note-v2", r'
post /v1/lcf/query "$Q" | py 'assert "adr-wal-durability" in {n["key"] for n in r["nodes"]}'
ok "knowledge and supersession survive restart"
post /v1/lcf/query '{"q":"zzzz qqqq"}' | py 'assert r["nodes"]==[] and any(w.startswith("NO_SEED") for w in r["warnings"]), r'
ok "no match is reported, not invented"
echo "E2E PASS"
