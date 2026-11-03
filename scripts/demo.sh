#!/usr/bin/env bash
# Start Atlas and ingest the real example documents (examples/atlas-decisions) through the product path.
# Data persists in ATLAS_DEMO_DIR (default ./.atlas-demo); ingestion is idempotent, so re-running is safe.
# Point it at your own notes: ATLAS_DOCS=path/to/markdown-dir scripts/demo.sh
set -euo pipefail
cd "$(dirname "$0")/.."
DIR="${ATLAS_DEMO_DIR:-.atlas-demo}"
ADDR="${ATLAS_ADDR:-127.0.0.1:8080}"
KEY="${ATLAS_DEMO_KEY:-demo-secret-key}"
DOCS="${ATLAS_DOCS:-examples/atlas-decisions}"
cargo build -p atlas-cli --release
ATLAS_API_KEYS="demo=$KEY" target/release/atlas serve --dir "$DIR/db" --addr "$ADDR" &
PID=$!
trap 'kill $PID 2>/dev/null' EXIT
for _ in $(seq 40); do curl -sf "http://$ADDR/healthz" >/dev/null 2>&1 && break; sleep 0.25; done
target/release/atlas ingest "$DOCS" --addr "$ADDR" --key "$KEY"
echo "Open: http://$ADDR/explorer#key=$KEY   (Ctrl+C to stop)"
wait $PID
