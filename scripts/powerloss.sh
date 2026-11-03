#!/usr/bin/env bash
# Power-loss harness driver. See docs/POWER_LOSS_RUNBOOK.md.
#   bash scripts/powerloss.sh write  <engine_dir> <ack_log> [--mode sync|group]   # then CUT POWER
#   bash scripts/powerloss.sh verify <engine_dir> <ack_log>                       # after reboot
#   bash scripts/powerloss.sh selftest                                            # mechanical check, NOT power loss
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -q -p atlas-engine --example powerloss
BIN="target/release/examples/powerloss"
[ -x "$BIN" ] || BIN="$BIN.exe"
case "${1:-}" in
  write | verify) shift; exec "$BIN" "${1:-}" "${2:-}" "${3:-}" "${4:-}" "${5:-}" 2>&1 ;;
  selftest)
    D="$(mktemp -d)"; echo "selftest dir: $D (kill -9 only; this is NOT power-loss evidence)"
    "$BIN" write "$D/engine" "$D/acks.log" --mode sync >"$D/w.out" 2>&1 &
    PID=$!; sleep 4; kill -9 "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true
    "$BIN" verify "$D/engine" "$D/acks.log"
    # tamper check: drop an acked id from the recovered side by pointing the log at a later id; verify must FAIL
    echo 99999999 >>"$D/acks.log"
    if "$BIN" verify "$D/engine" "$D/acks.log" >/dev/null 2>&1; then echo "SELFTEST FAIL: verifier accepted a lost write"; exit 1; fi
    echo "SELFTEST PASS: verifier passes a clean kill and rejects a lost acknowledged write"
    ;;
  *) echo "usage: powerloss.sh write|verify <engine_dir> <ack_log> | selftest"; exit 2 ;;
esac
