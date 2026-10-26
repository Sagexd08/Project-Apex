#!/usr/bin/env bash
# Prints "files=N digest=MD5" (md5 of the sorted per-file md5 list). Optional: --manifest FILE
set -euo pipefail
cd "$(dirname "$0")/.."
SRC=$(find crates ci .github .gitattributes Cargo.toml Cargo.lock deny.toml -type f -not -path '*/target/*' | sort | xargs md5sum | sed 's/ \*/  /')
D=$(echo "$SRC" | md5sum | cut -c1-32)
echo "files=$(echo "$SRC" | wc -l) digest=$D"
if [ "${1:-}" = "--manifest" ]; then
  { echo "# source manifest"; echo "# digest = md5 of the sorted per-file md5 list (lines below): $D"; echo "# recompute: bash scripts/digest.sh"; echo "$SRC"; } > "$2"
fi
