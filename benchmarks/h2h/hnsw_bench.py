import argparse
import json
import os
import time
import numpy as np
import hnswlib
from common import (EFS, FILTER_EF, K, SELS, conc_qps, load_gt, load_prepared, median_rep,
                    seq_pass, vmhwm_kb)

ap = argparse.ArgumentParser()
ap.add_argument("dir")
ap.add_argument("out")
ap.add_argument("--queries", type=int, default=10000)
ap.add_argument("--reps", type=int, default=3)
ap.add_argument("--fq", type=int, default=10000)
ap.add_argument("--smoke", action="store_true")
ap.add_argument("--M", type=int, default=32)
ap.add_argument("--efc", type=int, default=200)
ap.add_argument("--threads", type=int, default=1)
a = ap.parse_args()
efs = [64, 512] if a.smoke else EFS
meta, X, Q = load_prepared(a.dir)
nq, _, sels, blocks = load_gt(f"{a.dir}/gt.bin")
nq = min(a.queries, nq)
Q = Q[:nq]
fq = min(a.fq, nq)
h = np.fromfile(f"{a.dir}/h.bin", dtype="<u2")
n, dim = X.shape
space = "l2" if meta["metric"] == "euclidean" else "ip"
idx = hnswlib.Index(space=space, dim=dim)
idx.init_index(max_elements=n, ef_construction=a.efc, M=a.M, random_seed=100)
idx.set_num_threads(a.threads)
t = time.perf_counter()
idx.add_items(X, np.arange(n))
build = time.perf_counter() - t
hw_build = vmhwm_kb()
path = f"/tmp/hnsw_{os.getpid()}.bin"
idx.save_index(path)
disk = os.path.getsize(path)
os.remove(path)
idx.set_num_threads(1)
res = {"engine": "hnswlib", "params": {"M": a.M, "ef_construction": a.efc, "space": space, "build_threads": a.threads},
       "n": n, "dim": dim, "queries": nq, "filtered_queries": fq, "reps": a.reps, "warmup": 200,
       "build_seconds": build, "ingest_vps": n / build, "vmhwm_kb_after_build": hw_build,
       "disk_bytes": disk, "sweep": []}
for ef in efs:
    idx.set_ef(ef)

    def fn(q):
        return idx.knn_query(q.reshape(1, -1), k=K)[0][0].tolist()

    reps = [seq_pass(fn, Q, blocks[0][:nq]) for _ in range(a.reps)]
    idx.set_num_threads(8)
    cq = conc_qps(fn, list(Q), 8)
    idx.set_num_threads(1)
    med = median_rep(reps)
    res["sweep"].append({"ef": ef, "reps": reps, "median": med, "qps_conc8": cq})
    print(f"hnswlib ef={ef} recall={med['recall_at_10']:.4f} p95={med['p95_ms']:.3f}ms", flush=True)
idx.set_ef(FILTER_EF)
res["filtered"] = []
for si, (label, pm) in enumerate(SELS):
    def allow(i, pm=pm):
        return bool(h[i] < pm)

    def fn(q, allow=allow):
        try:
            return idx.knn_query(q.reshape(1, -1), k=K, filter=allow)[0][0].tolist()
        except RuntimeError:
            return []

    r = seq_pass(fn, Q[:fq], blocks[1 + si][:fq], warm=20)
    leaks = sum(1 for q in Q[:200] for i in fn(q) if h[i] >= pm)
    res["filtered"].append({"selectivity": label, "ef": FILTER_EF, "algorithm": "hnswlib native filtered walk",
                            **r, "leaks_in_200q": leaks})
    print(f"hnswlib sel={label} recall={r['recall_at_10']:.3f} p95={r['p95_ms']:.1f}ms ret={r['avg_returned']:.1f}", flush=True)
res["vmhwm_kb_end"] = vmhwm_kb()
json.dump(res, open(a.out, "w"), indent=1)
