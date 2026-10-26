#!/usr/bin/env bash
# Linux verification in Docker (Docker Desktop VM: process-kill evidence only, not power loss, not macOS).
# Usage: bash scripts/verify-linux.sh [--keep]   Logs are copied to target/verify-linux/.
set -uo pipefail
export MSYS_NO_PATH_CONV=1 MSYS2_ARG_CONV_EXCL="*"
cd "$(dirname "$0")/.."
echo "host: $(bash scripts/digest.sh)"
docker rm -f apex-linux > /dev/null 2>&1 || true
docker create --name apex-linux rust:1.95 sleep infinity > /dev/null && docker start apex-linux > /dev/null || { echo "FAIL docker"; exit 1; }
tar --exclude=target --exclude=.git -cf /tmp/apex-snap.tar -C . .
docker cp "$(cygpath -w /tmp/apex-snap.tar)" apex-linux:/tmp/apex-snap.tar
docker exec apex-linux bash -c 'mkdir -p /work && tar -xf /tmp/apex-snap.tar -C /work'
docker exec apex-linux bash /work/scripts/linux-inner.sh
mkdir -p target/verify-linux && docker cp apex-linux:/tmp/v/. target/verify-linux/ > /dev/null 2>&1 || true
[ "${1:-}" = "--keep" ] || docker rm -f apex-linux > /dev/null
