#!/usr/bin/env bash
set -euo pipefail
export MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL="*"
cd "$(dirname "$0")/../.."
docker rm -f apex-wheel > /dev/null 2>&1 || true
docker create --name apex-wheel rust:1.95 sleep infinity > /dev/null
docker start apex-wheel > /dev/null
tar --exclude=target --exclude=.git --exclude=film --exclude=benchmarks --exclude=site -cf /tmp/apex-wh.tar -C . .
docker cp "$(cygpath -w /tmp/apex-wh.tar)" apex-wheel:/tmp/s.tar
docker exec apex-wheel bash -c 'mkdir -p /work && tar -xf /tmp/s.tar -C /work && apt-get update -qq && apt-get install -y -qq python3-venv python3-dev > /dev/null && python3 -m venv /venv && /venv/bin/pip install -q maturin numpy && cd /work/adapters/atlas-py && /venv/bin/maturin build --release -o /wheels 2>&1 | tail -3 && /venv/bin/pip install -q /wheels/*.whl && /venv/bin/python -c "import atlas_ann; print(\"import ok\")"'
