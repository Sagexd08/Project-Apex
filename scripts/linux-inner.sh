#!/usr/bin/env bash
# Runs INSIDE the rust container (copied to /work). Prints a compact summary; logs in /tmp/v.
set -uo pipefail
export PATH=/usr/local/cargo/bin:$PATH
cd /work
mkdir -p /tmp/v
echo "container $(find crates ci .github .gitattributes Cargo.toml Cargo.lock deny.toml -type f -not -path '*/target/*' | wc -l) files, digest=$(find crates ci .github .gitattributes Cargo.toml Cargo.lock deny.toml -type f -not -path '*/target/*' | sort | xargs md5sum | md5sum | cut -c1-32)"
rustup component add rustfmt clippy > /dev/null 2>&1
step() { local n=$1; shift; if "$@" > "/tmp/v/$n.log" 2>&1; then echo "ok   $n"; else echo "FAIL $n  (/tmp/v/$n.log)"; fi; }
step fmt cargo fmt --all -- --check
step clippy cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast > /tmp/v/test.log 2>&1; rc=$?
awk '/^test result/{p+=$4;f+=$6;i+=$8} END{printf "tests passed=%d failed=%d ignored=%d\n",p,f,i}' /tmp/v/test.log
echo "cargo test exit=$rc"
grep -E "FAILED|panicked|overflow" /tmp/v/test.log | head -5 || true
bash ci/check-prod-graph.sh > /tmp/v/graph.log 2>&1; g=$?
echo "graph exit=$g ok=$(grep -c '^ok' /tmp/v/graph.log) fail=$(grep -c FAIL /tmp/v/graph.log)"
apt-get update -qq > /dev/null 2>&1; apt-get install -y -qq strace > /dev/null 2>&1
strace -f -e trace=fsync,fdatasync,openat -o /tmp/v/k.txt cargo test -p atlas-engine --test kill > /tmp/v/kill.log 2>&1
echo "kill(strace) exit=$? $(grep -E '^test result' /tmp/v/kill.log | head -1)"
grep -A1 -E 'openat\(AT_FDCWD, "/tmp/atlas-kill-[0-9]+", O_RDONLY\|O_CLOEXEC\)' /tmp/v/k.txt | head -6
echo "kernel $(uname -r); $(rustc --version)"
