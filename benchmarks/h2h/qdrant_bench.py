import argparse
import json
import time
import numpy as np
from qdrant_client import QdrantClient, models
from common import (EFS, FILTER_EF, K, SELS, conc_qps, load_gt, load_prepared, median_rep,
                    seq_pass)

ap = argparse.ArgumentParser()
ap.add_argument("dir")
ap.add_argument("out")
ap.add_argument("--host", default="h2h-qdrant")
ap.add_argument("--queries", type=int, default=10000)
ap.add_argument("--reps", type=int, default=3)
ap.add_argument("--fq", type=int, default=10000)
ap.add_argument("--smoke", action="store_true")
ap.add_argument("--M", type=int, default=32)
ap.add_argument("--efc", type=int, default=200)
a = ap.parse_args()
efs = [64, 512] if a.smoke else EFS
meta, X, Q = load_prepared(a.dir)
nq, _, sels, blocks = load_gt(f"{a.dir}/gt.bin")
nq = min(a.queries, nq)
Q = Q[:nq]
fq = min(a.fq, nq)
h = np.fromfile(f"{a.dir}/h.bin", dtype="<u2")
n, dim = X.shape
client = QdrantClient(host=a.host, grpc_port=6334, prefer_grpc=True, timeout=600)
for _ in range(90):
    try:
        client.get_collections()
        break
    except Exception:
        time.sleep(2)
else:
    raise SystemExit("qdrant not reachable")
name = "bench"
if client.collection_exists(name):
    client.delete_collection(name)
dist = models.Distance.EUCLID if meta["metric"] == "euclidean" else models.Distance.COSINE
client.create_collection(
    name,
    vectors_config=models.VectorParams(size=dim, distance=dist, on_disk=False),
    hnsw_config=models.HnswConfigDiff(m=a.M, ef_construct=a.efc, on_disk=False),
    optimizers_config=models.OptimizersConfigDiff(indexing_threshold=1000),
)
client.create_payload_index(name, "h", models.PayloadSchemaType.INTEGER)
BATCH = 1000
t = time.perf_counter()
for s in range(0, n, BATCH):
    e = min(s + BATCH, n)
    client.upsert(name, points=models.Batch(ids=list(range(s, e)), vectors=X[s:e].tolist(),
                                            payloads=[{"h": int(v)} for v in h[s:e]]), wait=False)
upload = time.perf_counter() - t
deadline = time.perf_counter() + 4 * 3600
while True:
    info = client.get_collection(name)
    if info.status == models.CollectionStatus.GREEN and (info.indexed_vectors_count or 0) >= n:
        break
    if time.perf_counter() > deadline:
        raise SystemExit("qdrant indexing did not finish")
    time.sleep(2)
build = time.perf_counter() - t
res = {"engine": "qdrant", "params": {"m": a.M, "ef_construct": a.efc, "distance": str(dist), "transport": "gRPC"},
       "n": n, "dim": dim, "queries": nq, "filtered_queries": fq, "reps": a.reps, "warmup": 200,
       "upload_seconds": upload, "build_seconds": build, "ingest_vps": n / build, "sweep": []}


def run(vec, ef, flt=None):
    r = client.query_points(name, query=vec, limit=K, query_filter=flt,
                            search_params=models.SearchParams(hnsw_ef=ef, exact=False))
    return [p.id for p in r.points]


for ef in efs:
    def fn(q, ef=ef):
        return run(q.tolist(), ef)

    reps = [seq_pass(fn, Q, blocks[0][:nq]) for _ in range(a.reps)]
    cq = conc_qps(fn, list(Q), 8)
    med = median_rep(reps)
    res["sweep"].append({"ef": ef, "reps": reps, "median": med, "qps_conc8": cq})
    print(f"qdrant ef={ef} recall={med['recall_at_10']:.4f} p95={med['p95_ms']:.3f}ms", flush=True)
res["filtered"] = []
for si, (label, pm) in enumerate(SELS):
    flt = models.Filter(must=[models.FieldCondition(key="h", range=models.Range(lt=pm))])

    def fn(q, flt=flt):
        return run(q.tolist(), FILTER_EF, flt)

    r = seq_pass(fn, Q[:fq], blocks[1 + si][:fq], warm=20)
    leaks = sum(1 for q in Q[:200] for i in fn(q) if h[i] >= pm)
    res["filtered"].append({"selectivity": label, "ef": FILTER_EF, "algorithm": "qdrant native filtered search (payload index)",
                            **r, "leaks_in_200q": leaks})
    print(f"qdrant sel={label} recall={r['recall_at_10']:.3f} p95={r['p95_ms']:.1f}ms ret={r['avg_returned']:.1f}", flush=True)
json.dump(res, open(a.out, "w"), indent=1)
