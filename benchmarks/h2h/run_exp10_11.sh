#!/usr/bin/env bash
set -uo pipefail
export MSYS_NO_PATHCONV=1
cd "$(dirname "$0")/../.."
OUT="benchmarks/h2h/profiling/raw/exp10_11"
mkdir -p "$OUT"
BIN="$(pwd -W)/target/linux/atlas"
{
  echo "date: $(date -Is)"
  echo "digest: $(bash scripts/digest.sh | head -n 1)"
  echo "atlas_sha256: $(sha256sum target/linux/atlas | cut -d' ' -f1)"
  echo "docker_other_running: $(docker ps --format '{{.Names}}' | tr '\n' ' ')"
  echo "qdrant_image: $(docker image inspect qdrant/qdrant:latest --format '{{.Id}}')"
  echo "docker_cpus: $(docker info --format '{{.NCPU}}') mem: $(docker info --format '{{.MemTotal}}')"
} > "$OUT/host.txt"
i=0
for e in atlas qdrant qdrant atlas; do
  i=$((i+1))
  python benchmarks/h2h/exp10_11.py --engine "$e" --atlas-bin "$BIN" --kills 10 --out "$OUT/${i}_$e.json" 2>&1 | tee "$OUT/${i}_$e.log"
done
echo finished > "$OUT/done.txt"
