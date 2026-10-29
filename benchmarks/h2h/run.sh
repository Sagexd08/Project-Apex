#!/usr/bin/env bash
set -uo pipefail
export MSYS_NO_PATHCONV=1
cd "$(dirname "$0")/../.."
REPO="$(pwd -W 2>/dev/null || pwd)"
DATA="${H2H_DATA:-C:/Desktop/apex-data}"
CACHE="$DATA/h2h-cache"
IMG=atlas-h2h
NET=h2h
QNAME=h2h-qdrant
CPUS=8
MEM=9g
DS="${1:?dataset: fashion|nytimes|sift|glove}"
ENG="${2:?engine: gt|atlas|hnswlib|qdrant|all}"
shift 2
EXTRA=("$@")
OUT="benchmarks/h2h/results/$DS"
mkdir -p "$OUT" "$DATA/h2h-cache"

dr() {
  docker run --rm --cpus "$CPUS" --memory "$MEM" --network "$NET" \
    -v "$REPO:/work" -v "$DATA:/data:ro" -v "$CACHE:/cache" \
    -v h2h-cargo:/usr/local/cargo/registry -e CARGO_TARGET_DIR=/cache/target \
    "$IMG" "$@"
}

docker network inspect "$NET" > /dev/null 2>&1 || docker network create "$NET" > /dev/null
docker image inspect "$IMG" > /dev/null 2>&1 || docker build -t "$IMG" benchmarks/h2h

write_config() {
  local d src
  d="$(bash scripts/digest.sh | tail -1)"
  src="$(find adapters/atlas-usearch/src benchmarks/h2h -type f -not -path '*/target/*' -not -path '*/results/*' -not -path '*/cache/*' | sort | xargs md5sum | md5sum | cut -c1-32)"
  {
    echo "{"
    echo "  \"dataset\": \"$DS\","
    echo "  \"date_utc\": \"$(date -u +%FT%TZ)\","
    echo "  \"core_digest\": \"$d\","
    echo "  \"adapter_and_harness_digest\": \"$src\","
    echo "  \"docker_cpus\": $(docker info --format '{{.NCPU}}'),"
    echo "  \"docker_mem_bytes\": $(docker info --format '{{.MemTotal}}'),"
    echo "  \"docker_kernel\": \"$(docker info --format '{{.KernelVersion}}')\","
    echo "  \"engine_cpu_limit\": $CPUS, \"engine_mem_limit\": \"$MEM\","
    echo "  \"host_cpu\": \"$(powershell.exe -NoProfile -Command '(Get-CimInstance Win32_Processor).Name' 2>/dev/null | tr -d '\r')\","
    echo "  \"docker_other_running\": \"$(docker ps --format '{{.Names}}' | grep -v h2h- | tr '\n' ' ')\","
    echo "  \"docker_stats_snapshot\": \"$(docker stats --no-stream --format '{{.Name}} cpu={{.CPUPerc}} mem={{.MemUsage}}' | tr '\n' ';')\","
    echo "  \"qdrant_image_id\": \"$(docker image inspect qdrant/qdrant:latest --format '{{.Id}}')\","
    echo "  \"bench_image_id\": \"$(docker image inspect $IMG --format '{{.Id}}')\","
    echo "  \"pip_freeze\": \"$(dr pip freeze | tr '\n' ' ')\","
    echo "  \"rustc\": \"$(dr rustc --version)\","
    echo "  \"extra_args\": \"${EXTRA[*]}\""
    echo "}"
  } > "$OUT/config.json"
}

run_gt() { dr python gt.py "/data/$DS" "/cache/$DS" | tee "$OUT/gt.log"; }
run_hnsw() { dr python hnsw_bench.py "/cache/$DS" "/work/$OUT/hnswlib.json" "${EXTRA[@]}" 2>&1 | tee "$OUT/hnswlib.log"; }
run_atlas() {
  dr bash -c "cd atlas-bench && cargo run --release --locked -- --dir /cache/$DS --out /work/$OUT/atlas.json ${EXTRA[*]}" 2>&1 | tee "$OUT/atlas.log"
}
run_qdrant() {
  docker rm -f "$QNAME" > /dev/null 2>&1
  docker run -d --name "$QNAME" --network "$NET" --cpus "$CPUS" --memory "$MEM" qdrant/qdrant:latest > /dev/null
  sleep 8
  docker run --rm --cpus 4 --memory 6g --network "$NET" -v "$REPO:/work" -v "$CACHE:/cache" "$IMG" \
    python qdrant_bench.py "/cache/$DS" "/work/$OUT/qdrant.json" --host "$QNAME" "${EXTRA[@]}" 2>&1 | tee "$OUT/qdrant.log"
  {
    echo "vmhwm_qdrant_process: $(docker exec "$QNAME" sh -c 'for p in /proc/[0-9]*; do if [ "$(cat $p/comm)" = qdrant ]; then grep -E "VmHWM|VmRSS" $p/status; fi; done' | tr '\n' ' ')"
    echo "storage_bytes: $(docker exec "$QNAME" sh -c 'du -sb /qdrant/storage | cut -f1')"
    echo "version: $(docker exec "$QNAME" ./qdrant --version 2>&1 | head -1)"
  } > "$OUT/qdrant_container.txt"
  docker rm -f "$QNAME" > /dev/null 2>&1
}

[ "$ENG" = "gt" ] || write_config
case "$ENG" in
  gt) run_gt ;;
  atlas) run_atlas ;;
  hnswlib) run_hnsw ;;
  qdrant) run_qdrant ;;
  all) [ -f "$CACHE/$DS/gt.bin" ] || run_gt; run_atlas; run_hnsw; run_qdrant ;;
  *) echo "unknown engine $ENG"; exit 2 ;;
esac
