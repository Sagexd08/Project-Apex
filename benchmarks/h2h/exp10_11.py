import argparse
import json
import random
import statistics
import threading
import time

from exp09_recovery import AtlasC, QdrantC, call, retried, vectors, COLLECTION


def present_ids(eng, ids):
    present = set()
    if eng.label == "atlas":
        lock = threading.Lock()
        errs = []

        def one(i):
            try:
                call("POST", eng.base + "/v1/fetch", {"id": i}, eng.h, 30)
                return i
            except Exception as e:
                if getattr(e, "code", 0) == 404:
                    return None
                raise

        def work(part):
            try:
                for i in part:
                    r = retried(lambda i=i: one(i))
                    if r is not None:
                        with lock:
                            present.add(r)
            except Exception as e:
                errs.append(repr(e))

        ths = [threading.Thread(target=work, args=(ids[a::16],)) for a in range(16)]
        for t in ths:
            t.start()
        for t in ths:
            t.join()
        if errs:
            raise RuntimeError(errs[0])
        return present
    for s in range(0, len(ids), 500):
        part = ids[s:s + 500]
        r = retried(lambda part=part: call(
            "POST", f"{eng.base}/collections/{COLLECTION}/points",
            {"ids": part, "with_payload": False, "with_vector": False}, None, 120))
        present.update(int(x["id"]) for x in r["result"])
    return present


def write_one(eng, i, vec):
    if eng.label == "atlas":
        call("POST", eng.base + "/v1/insert", {"id": i, "vector": vec}, eng.h, 10)
    else:
        call("PUT", f"{eng.base}/collections/{COLLECTION}/points?wait=true",
             {"points": [{"id": i, "vector": vec}]}, None, 10)


def wait_ready(eng, probe, timeout=600):
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            eng.query_ok(probe)
            return
        except Exception:
            time.sleep(0.05)
    raise RuntimeError("never ready")


def exp10(eng, vecs, kills, writers, seed):
    rng = random.Random(seed)
    probe = vecs[0]
    results = []
    next_id = 1
    for k in range(kills):
        acked = []
        lock = threading.Lock()
        stop = threading.Event()
        base = next_id

        def writer(w, base=base):
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
        for t in ths:
            t.start()
        time.sleep(rng.uniform(0.7, 2.0))
        eng.kill()
        stop.set()
        for t in ths:
            t.join()
        next_id = (max(acked) + writers + 1) if acked else next_id + writers
        eng.start()
        wait_ready(eng, probe)
        present = present_ids(eng, sorted(acked))
        results.append({"kill": k, "acked": len(acked), "missing": len(set(acked) - present)})
        print(json.dumps(results[-1]), flush=True)
    return results


def exp11(eng, vecs, n):
    lat = []
    timeouts = 0
    base = 10_000_000
    rng = random.Random(4242)
    dim = len(vecs[0])
    for j in range(n):
        i = base + j
        v = [rng.gauss(0, 1) for _ in range(dim)]
        write_one(eng, i, v)
        t_ack = time.perf_counter()
        found = False
        while time.perf_counter() - t_ack < 10:
            try:
                if eng.label == "atlas":
                    r = call("POST", eng.base + "/v1/search", {"vector": v, "k": 1}, eng.h, 10)
                    hits = r.get("hits") or r.get("results") or []
                    top = int(hits[0].get("id", -1)) if hits else -1
                else:
                    r = call("POST", f"{eng.base}/collections/{COLLECTION}/points/search",
                             {"vector": v, "limit": 1}, None, 10)["result"]
                    top = int(r[0]["id"]) if r else -1
            except Exception:
                top = -1
            if top == i:
                found = True
                break
        if found:
            lat.append((time.perf_counter() - t_ack) * 1000)
        else:
            timeouts += 1
    return lat, timeouts


def pct(xs, p):
    s = sorted(xs)
    return s[min(len(s) - 1, int(len(s) * p))]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--engine", choices=["atlas", "qdrant"], required=True)
    ap.add_argument("--atlas-bin", default="")
    ap.add_argument("--base-n", type=int, default=20000)
    ap.add_argument("--kills", type=int, default=10)
    ap.add_argument("--writers", type=int, default=16)
    ap.add_argument("--inserts", type=int, default=10000)
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
    res = {"engine": a.engine, "base_n": a.base_n, "writers": a.writers,
           "cpus": a.cpus, "mem": a.mem}
    try:
        eng.create()
        eng.prepare()
        for _ in range(400):
            try:
                eng.query_ok(vecs[0])
                break
            except Exception as e:
                if getattr(e, "code", 0) in (400, 404):
                    break
                time.sleep(0.1)
        eng.insert_all(vecs, 8)
        eng.settle(a.base_n)
        lat, timeouts = exp11(eng, vecs, a.inserts)
        res["exp11_ms"] = {"n": len(lat), "timeouts": timeouts, "p50": statistics.median(lat),
                           "p95": pct(lat, 0.95), "p99": pct(lat, 0.99), "max": max(lat)}
        print("exp11", json.dumps(res["exp11_ms"]), flush=True)
        res["exp10"] = exp10(eng, vecs, a.kills, a.writers, 10)
        res["exp10_missing_total"] = sum(r["missing"] for r in res["exp10"])
        res["exp10_acked_total"] = sum(r["acked"] for r in res["exp10"])
    finally:
        eng.remove()
    with open(a.out, "w") as f:
        json.dump(res, f, indent=1)
    print("done missing", res.get("exp10_missing_total"), "acked", res.get("exp10_acked_total"))


if __name__ == "__main__":
    main()
