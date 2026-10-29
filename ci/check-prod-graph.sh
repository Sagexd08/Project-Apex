#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
fail=0
HA=ci/hook-absence/Cargo.toml
NAMES="fail_next_flush_handle fail_next_truncate_handle fail_next_zero_handle partial_write_handle fail_next_marker_handle die_after_marker_handle flush_delay_handle set_before_gate_hook open_with_fingerprint len is_empty for_test commit_hold_handle poison_detail poison_layer_counts bypass_engine_poison_check"
WAL_NAMES="fail_next_flush_handle fail_next_truncate_handle fail_next_zero_handle partial_write_handle fail_next_marker_handle die_after_marker_handle flush_delay_handle commit_hold_handle"

prod=$(cargo tree -e features,normal,build --workspace --prefix none)
for feat in test-auth test-support; do
  out=$(echo "$prod" | grep -E "(^|[^a-z-])$feat([^a-z-]|$)" || true)
  if [ -n "$out" ]; then echo "FAIL: $feat in production graph:"; echo "$out"; fail=1; else echo "ok: production graph has no $feat"; fi
done

for feat in preserve_order indexmap; do
  out=$(echo "$prod" | grep -E "(^|[^a-z_-])$feat([^a-z_-]|$)" || true)
  if [ -n "$out" ]; then echo "FAIL: $feat in production graph:"; echo "$out"; fail=1; else echo "ok: production graph has no $feat"; fi
done

dev=$(cargo tree -e features --workspace --prefix none)
for feat in test-auth test-support; do
  if echo "$dev" | grep -qE "(^|[^a-z-])$feat([^a-z-]|$)"; then echo "ok: negative control: $feat appears with dev edges"; else echo "FAIL: negative control: $feat not found with dev edges"; fail=1; fi
done

check_hook() {
  local feat=$1 n=$2 label=$3 rc=0 err
  err=$(cargo check --manifest-path "$HA" --features "$feat" 2>&1) || rc=$?
  if [ "$rc" -eq 0 ]; then echo "FAIL: $label compiles without hooks"; fail=1
  elif echo "$err" | grep -qE "no [a-z ]+ named .$n. "; then echo "ok: $label absent without hooks"
  else echo "FAIL: $label: wrong compile error:"; echo "$err"; fail=1; fi
  if cargo check --manifest-path "$HA" --features "hooks,$feat" >/dev/null 2>&1; then echo "ok: $label present with hooks"; else echo "FAIL: $label does not compile with hooks"; fail=1; fi
}
for n in $NAMES; do check_hook "use_$n" "$n" "$n"; done
for n in $WAL_NAMES; do check_hook "use_wal_$n" "$n" "Wal::$n"; done

cargo build --release -p atlas-engine || fail=1
exit $fail
