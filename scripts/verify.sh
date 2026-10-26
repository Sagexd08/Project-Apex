#!/usr/bin/env bash
# One-command Windows verification with a compact summary (replaces scout agents). Full logs: target/verify/*.log
# Usage: bash scripts/verify.sh [--cov]
set -uo pipefail
cd "$(dirname "$0")/.."
mkdir -p target/verify
step() { local n=$1; shift; if "$@" > "target/verify/$n.log" 2>&1; then echo "ok   $n"; else echo "FAIL $n  (target/verify/$n.log)"; fi; }
step fmt cargo fmt --all -- --check
step clippy cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace > target/verify/test.log 2>&1; rc=$?
awk '/^test result/{p+=$4;f+=$6;i+=$8} END{printf "tests passed=%d failed=%d ignored=%d\n",p,f,i}' target/verify/test.log
echo "cargo test exit=$rc"
grep -E "FAILED|panicked|overflow" target/verify/test.log | head -5 || true
bash ci/check-prod-graph.sh > target/verify/graph.log 2>&1; g=$?
echo "graph exit=$g ok=$(grep -c '^ok' target/verify/graph.log) fail=$(grep -c FAIL target/verify/graph.log)"
echo "deny: $(cargo deny check 2>&1 | tail -1)"
if [ "${1:-}" = "--cov" ]; then
  cargo llvm-cov --workspace --summary-only 2>&1 | grep -E "lib.rs|auth.rs|conformance.rs|TOTAL" | awk '{print $1, $(NF-3)}' | grep -v Running
fi
bash scripts/digest.sh
