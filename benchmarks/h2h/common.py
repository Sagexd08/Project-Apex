import json
import struct
import time
import numpy as np

K = 10
SELS = [("10%", 100), ("5%", 50), ("1%", 10), ("0.1%", 1)]
EFS = [64, 128, 256, 512, 1024]
FILTER_EF = 512


def mix_np(z):
    z = z.astype(np.uint64) + np.uint64(0x9E3779B97F4A7C15)
    z = (z ^ (z >> np.uint64(30))) * np.uint64(0xBF58476D1CE4E5B9)
    z = (z ^ (z >> np.uint64(27))) * np.uint64(0x94D049BB133111EB)
    return z ^ (z >> np.uint64(31))


def hbucket(n):
    ids = np.arange(n, dtype=np.uint64)
    with np.errstate(over="ignore"):
        return (mix_np(ids ^ np.uint64(0xF117E4)) % np.uint64(1000)).astype(np.uint16)


def load_prepared(d):
    meta = json.load(open(f"{d}/meta.json"))
    dim = meta["dim"]
    train = np.fromfile(f"{d}/train.f32", dtype="<f4").reshape(-1, dim)
    test = np.fromfile(f"{d}/test.f32", dtype="<f4").reshape(-1, dim)
    return meta, train, test


def load_gt(path):
    b = open(path, "rb").read()
    nq, k, nsel = struct.unpack_from("<3I", b, 0)
    off = 12
    sels = list(struct.unpack_from(f"<{nsel}I", b, off))
    off += 4 * nsel
    blocks = []
    for _ in range(1 + nsel):
        blocks.append(np.frombuffer(b, dtype="<i4", count=nq * k, offset=off).reshape(nq, k))
        off += 4 * nq * k
    return nq, k, sels, blocks


def recall(found, truth):
    t = [x for x in truth if x >= 0]
    if not t:
        return 1.0
    return len(set(found) & set(t)) / len(t)


def pct(lat_ms):
    a = np.sort(np.asarray(lat_ms))

    def q(p):
        return float(a[min(int(len(a) * p), len(a) - 1)])

    return {"p50_ms": q(0.5), "p95_ms": q(0.95), "p99_ms": q(0.99), "mean_ms": float(a.mean())}


def seq_pass(search, queries, gt, warm=200):
    for q in queries[:warm]:
        search(q)
    lat, rec, ret = [], 0.0, 0
    t0 = time.perf_counter()
    for q, t in zip(queries, gt):
        s = time.perf_counter()
        r = search(q)
        lat.append((time.perf_counter() - s) * 1e3)
        rec += recall(r, t)
        ret += len(r)
    wall = time.perf_counter() - t0
    out = pct(lat)
    out.update(recall_at_10=rec / len(queries), qps_seq=len(queries) / wall, avg_returned=ret / len(queries))
    return out


def conc_qps(search, queries, workers=8):
    from concurrent.futures import ThreadPoolExecutor
    t0 = time.perf_counter()
    with ThreadPoolExecutor(workers) as ex:
        list(ex.map(search, queries))
    return len(queries) / (time.perf_counter() - t0)


def median_rep(reps):
    keys = [k for k in reps[0] if isinstance(reps[0][k], float)]
    return {k: float(np.median([r[k] for r in reps])) for k in keys}


def vmhwm_kb():
    for line in open("/proc/self/status"):
        if line.startswith("VmHWM"):
            return int(line.split()[1])
    return -1
