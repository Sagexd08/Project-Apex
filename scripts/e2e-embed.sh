#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
BIN="${ATLAS_BIN:-target/debug/atlas}"
[ -x "$BIN" ] || [ -x "$BIN.exe" ] || cargo build -p atlas-cli
EMBED_URL="${ATLAS_EMBED_URL:-http://127.0.0.1:11500/v1/embeddings}"
MODEL="${ATLAS_EMBED_MODEL:-all-MiniLM-L6-v2}"
DIM="${ATLAS_EMBED_DIM:-384}"
ADDR=127.0.0.1:${E2E_PORT:-18091}; A=http://$ADDR; KEY=e2e-secret-key
Q='{"q":"Can I trust that a confirmed save will not vanish when the machine is switched off abruptly?","max_hops":1}'
curl -sf -m 30 -X POST "$EMBED_URL" -d '{"input":"ping"}' | python -c "import sys,json;assert len(json.load(sys.stdin)['data'][0]['embedding'])==$DIM" \
  || { echo "embedder at $EMBED_URL is not serving $DIM-dim vectors"; exit 1; }
DIR=$(mktemp -d); PID=""
stop() { [ -n "$PID" ] && kill -9 "$PID" 2>/dev/null; wait "$PID" 2>/dev/null || true; PID=""; }
trap 'stop; rm -rf "$DIR"' EXIT
run() {
  name=$1; shift
  env -u ATLAS_EMBED_URL -u ATLAS_EMBED_MODEL "$@" ATLAS_API_KEYS="e2e=$KEY" "$BIN" serve --dir "$DIR/$name" --addr "$ADDR" $DIMFLAG >"$DIR/$name.log" 2>&1 & PID=$!
  for _ in $(seq 60); do curl -sf "$A/healthz" >/dev/null 2>&1 && break; sleep 0.25; done
  "$BIN" ingest examples/atlas-decisions --addr "$ADDR" --key "$KEY" >/dev/null
  curl -s -H "Authorization: Bearer $KEY" -X POST "$A/v1/lcf/query" -d "$Q" \
    | python -c "import sys,json;r=json.load(sys.stdin);k=[n['key'] for n in r['nodes']];print('$name:',k[:6]);sys.exit(0 if 'adr-wal-durability' in k else 3)"
  rc=$?; stop; return $rc
}
set +e
DIMFLAG="--dim $DIM" run semantic ATLAS_EMBED_URL="$EMBED_URL" ATLAS_EMBED_MODEL="$MODEL"; SEM=$?
DIMFLAG="" run control; CTL=$?
set -e
[ "$SEM" = 0 ] || { echo "E2E-EMBED FAIL: real embedder did not surface adr-wal-durability"; exit 1; }
echo "control (hash embedder) found it: $([ "$CTL" = 0 ] && echo yes || echo no)"
echo "E2E-EMBED PASS"
