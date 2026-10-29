#!/usr/bin/env bash
set -uo pipefail
export PATH=/usr/local/cargo/bin:$PATH
rustup component add rustfmt clippy > /dev/null 2>&1
cd /work/adapters/atlas-usearch
cargo fmt -- --check && echo fmt-ok
cargo clippy --all-targets -- -D warnings 2>&1 | tail -2
cargo test --release 2>&1 | grep -E "^test result|FAILED|panicked"
echo "kernel $(uname -r) $(rustc --version)"
