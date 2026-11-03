#!/usr/bin/env bash
set -euo pipefail
export MSYS_NO_PATH_CONV=1 MSYS2_ARG_CONV_EXCL="*"
cd "$(dirname "$0")/../.."
docker rm -f apex-lbuild > /dev/null 2>&1 || true
docker create --name apex-lbuild rust:1.95 sleep infinity > /dev/null
docker start apex-lbuild > /dev/null
tar --exclude=target --exclude=.git --exclude=film --exclude=benchmarks -cf /tmp/apex-lb.tar -C . .
docker cp "$(cygpath -w /tmp/apex-lb.tar)" apex-lbuild:/tmp/s.tar
docker exec apex-lbuild bash -c 'mkdir -p /work && tar -xf /tmp/s.tar -C /work && cd /work && cargo build --release -p atlas-cli 2>&1 | tail -n 2'
mkdir -p target/linux
docker cp apex-lbuild:/work/target/release/atlas target/linux/atlas
docker rm -f apex-lbuild > /dev/null
sha256sum target/linux/atlas
