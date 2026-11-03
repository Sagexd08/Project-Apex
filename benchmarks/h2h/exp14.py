import argparse
import json
import statistics
import threading
import time

from exp09_recovery import AtlasC, QdrantC
from exp10_11 import present_ids, wait_ready, write_one
from exp09_recovery import vectors


def window(eng, vecs, base, writers, seconds):
    acked = []
    lock = threading.Lock()
    stop = threading.Event()

    def writer(w):
        i = base + w
        while not stop.is_set():
            try:
                write_one(eng, i, vecs[i % len(vecs)])
            except Exception:
                return
            with lock:
                acked.append(i)
            i += writers

    ths = [threading.Thread(target=writer, args=(w,)) for w in range(writers)]
    t0 = time.perf_counter()
    for t in ths:
        t.start()
    time.sleep(seconds)
    stop.set()
    for t in ths:
        t.join()
    elapsed = time.perf_counter() - t0
    n = len(acked)
    eng.kill()
    eng.start()
    wait_ready(eng, vecs[0])
    present = present_ids(eng, sorted(acked))
    return {"acked": n, "seconds": elapsed, "per_s": n / elapsed, "missing": len(set(acked) - present)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--engine", choices=["atlas", "qdrant"], required=True)
    ap.add_argument("--atlas-bin", default="")
    ap.add_argument("--base-n", type=int, default=20000)
    ap.add_argument("--windows", type=int, default=5)
    ap.add_argument("--seconds", type=float, default=30)
    ap.add_argument("--writers", type=int, default=16)
    ap.add_argument("--dim", type=int, default=64)
    ap.add_argument("--port", type=int, default=18111)
    ap.add_argument("--cpus", type=float, default=4)
    ap.add_argument("--mem", default="6g")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    vecs = vectors(a.base_n, a.dim, 7)
    if a.engine == "atlas":
        eng = AtlasC(a.atlas_bin, a.dim, a.port, a.cpus, a.mem)
    else:
        eng = QdrantC(a.dim, a.port, a.cpus, a.mem)
    res = {"engine": a.engine, "base_n": a.base_n, "writers": a.writers, "seconds": a.seconds,
           "cpus": a.cpus, "mem": a.mem, "windows": []}
    try:
        eng.create()
        eng.prepare()
        wait_ready(eng, vecs[0]) if a.engine == "atlas" else None
        eng.insert_all(vecs, 8)
        eng.settle(a.base_n)
        base = 50_000_000
        for w in range(a.windows):
            r = window(eng, vecs, base, a.writers, a.seconds)
            base += 5_000_000
            res["windows"].append(r)
            print(json.dumps(r), flush=True)
    finally:
        eng.remove()
    per = [w["per_s"] for w in res["windows"]]
    res["median_per_s"] = statistics.median(per)
    res["missing_total"] = sum(w["missing"] for w in res["windows"])
    with open(a.out, "w") as f:
        json.dump(res, f, indent=1)
    print("median_per_s", res["median_per_s"], "missing", res["missing_total"])


if __name__ == "__main__":
    main()
