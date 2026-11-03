import sys, json, time, numpy as np, hnswlib
from common import load_prepared
d, out, th = sys.argv[1], sys.argv[2], int(sys.argv[3])
meta, X, _ = load_prepared(d)
n, dim = X.shape
idx = hnswlib.Index(space="l2" if meta["metric"] == "euclidean" else "ip", dim=dim)
idx.init_index(max_elements=n, ef_construction=200, M=32, random_seed=100)
idx.set_num_threads(th)
t = time.perf_counter(); idx.add_items(X, np.arange(n)); s = time.perf_counter() - t
json.dump({"engine": "hnswlib", "threads": th, "n": n, "build_seconds": s, "vps": n / s}, open(out, "w"), indent=1)
print(f"hnswlib threads={th} build={s:.1f}s")
