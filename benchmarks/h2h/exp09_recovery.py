import argparse
import json
import random
import re
import statistics
import subprocess
import threading
import time
import urllib.request

KEY = "bench-key-123456"
COLLECTION = "exp09"
READY_TIMEOUT_S = 900


def call(method, url, body=None, headers=None, timeout=60):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data, headers or {}, method=method)
    return json.loads(urllib.request.urlopen(req, timeout=timeout).read() or b"{}")


def retried(fn, tries=8, wait=5):
    for attempt in range(tries):
        try:
            return fn()
        except OSError:
            if attempt == tries - 1:
                raise
            time.sleep(wait)


def docker(*args, check=True):
    return subprocess.run(["docker", *args], capture_output=True, text=True, check=check)


def vectors(n, dim, seed):
    rng = random.Random(seed)
    return [[rng.gauss(0, 1) for _ in range(dim)] for _ in range(n)]


def fan_out(items, fn, threads):
    errors = []

    def work(part):
        try:
            for it in part:
                fn(it)
        except Exception as e:
            errors.append(repr(e))

    ths = [threading.Thread(target=work, args=(items[a::threads],)) for a in range(threads)]
    for t in ths:
        t.start()
    for t in ths:
        t.join()
    if errors:
        raise RuntimeError(errors[0])


class Container:
    def __init__(self, name, port, cpus, mem):
        self.name, self.port, self.cpus, self.mem = name, port, cpus, mem
        self.base = f"http://127.0.0.1:{port}"

    def create(self, image, inner_port, extra):
        docker("rm", "-f", self.name, check=False)
        docker("volume", "rm", "-f", self.name + "-data", check=False)
        docker("run", "-d", "--name", self.name, "--cpus", str(self.cpus), "--memory", self.mem,
               "-p", f"{self.port}:{inner_port}", "-v", f"{self.name}-data:/data", *extra, image)

    def start(self):
        docker("start", self.name)

    def kill(self):
        docker("kill", "-s", "KILL", self.name)
        docker("wait", self.name, check=False)

    def remove(self):
        docker("rm", "-f", self.name, check=False)
        docker("volume", "rm", "-f", self.name + "-data", check=False)


class AtlasC(Container):
    label = "atlas"
    BATCH = 500

    def __init__(self, binary, dim, port, cpus, mem):
        super().__init__("exp09-atlas", port, cpus, mem)
        self.binary, self.dim = binary, dim
        self.h = {"Authorization": f"Bearer {KEY}"}

    def create(self):
        docker("rm", "-f", self.name, check=False)
        docker("volume", "rm", "-f", self.name + "-data", check=False)
        docker("run", "-d", "--name", self.name, "--cpus", str(self.cpus), "--memory", self.mem,
               "-p", f"{self.port}:8080", "-v", f"{self.name}-data:/data",
               "-v", f"{self.binary}:/bin/atlas:ro", "-e", f"ATLAS_API_KEYS=bench={KEY}",
               "--entrypoint", "/bin/atlas", "rust:1.95",
               "serve", "--dir", "/data/db", "--addr", "0.0.0.0:8080", "--dim", str(self.dim))

    def prepare(self):
        pass

    def insert_all(self, vecs, threads):
        starts = list(range(0, len(vecs), self.BATCH))

        def one(s):
            items = [{"id": i, "vector": vecs[i]} for i in range(s, min(s + self.BATCH, len(vecs)))]
            retried(lambda: call("POST", self.base + "/v1/insert_batch", {"items": items}, self.h, 300))
        fan_out(starts, one, min(threads, 8))

    def settle(self, expect):
        deadline = time.time() + 300
        while time.time() < deadline:
            names = docker("exec", self.name, "ls", "/data/db", check=False).stdout.split()
            seqs = [int(m) for n in names for m in re.findall(r"\d{6,}", n)]
            if seqs and max(seqs) >= expect:
                return
            time.sleep(0.5)
        raise RuntimeError("atlas never checkpointed")

    def query_ok(self, probe):
        r = call("POST", self.base + "/v1/search", {"vector": probe, "k": 1}, self.h, 5)
        hits = r.get("hits") or r.get("results") or []
        return bool(hits) and int(hits[0].get("id", -1)) == 0

    def count(self):
        return call("GET", self.base + "/v1/count", None, self.h)["count"]


class QdrantC(Container):
    label = "qdrant"
    BATCH = 500

    def __init__(self, dim, port, cpus, mem):
        super().__init__("exp09-qdrant", port, cpus, mem)
        self.dim = dim

    def create(self):
        docker("rm", "-f", self.name, check=False)
        docker("volume", "rm", "-f", self.name + "-data", check=False)
        docker("run", "-d", "--name", self.name, "--cpus", str(self.cpus), "--memory", self.mem,
               "-p", f"{self.port}:6333", "-v", f"{self.name}-data:/qdrant/storage",
               "qdrant/qdrant:latest")

    def prepare(self):
        for _ in range(200):
            try:
                call("PUT", f"{self.base}/collections/{COLLECTION}",
                     {"vectors": {"size": self.dim, "distance": "Cosine"}}, None, 5)
                return
            except Exception:
                time.sleep(0.25)
        raise RuntimeError("qdrant did not come up")

    def insert_all(self, vecs, threads):
        batches = list(range(0, len(vecs), self.BATCH))

        def one(s):
            pts = [{"id": i, "vector": vecs[i]} for i in range(s, min(s + self.BATCH, len(vecs)))]
            retried(lambda: call("PUT", f"{self.base}/collections/{COLLECTION}/points?wait=true", {"points": pts}, None, 900))
        fan_out(batches, one, min(threads, 4))

    def settle(self, expect):
        for _ in range(3600):
            st = call("GET", f"{self.base}/collections/{COLLECTION}")["result"]
            if st["status"] == "green" and st.get("optimizer_status") == "ok":
                return
            time.sleep(1)
        raise RuntimeError("qdrant never went green")

    def query_ok(self, probe):
        r = call("POST", f"{self.base}/collections/{COLLECTION}/points/search",
                 {"vector": probe, "limit": 1}, None, 5)["result"]
        return bool(r) and int(r[0]["id"]) == 0

    def count(self):
        return call("POST", f"{self.base}/collections/{COLLECTION}/points/count",
                    {"exact": True})["result"]["count"]


def restart_to_ready(eng, probe, expect):
    t0 = time.perf_counter()
    eng.start()
    while time.perf_counter() - t0 < READY_TIMEOUT_S:
        try:
            if eng.query_ok(probe):
                break
        except Exception:
            pass
        time.sleep(0.01)
    else:
        raise RuntimeError("never became ready")
    ms = (time.perf_counter() - t0) * 1000
    return ms, expect - eng.count()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--engine", choices=["atlas", "qdrant"], required=True)
    ap.add_argument("--atlas-bin", default="")
    ap.add_argument("--n", type=int, default=100000)
    ap.add_argument("--dim", type=int, default=64)
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--threads", type=int, default=32)
    ap.add_argument("--port", type=int, default=18111)
    ap.add_argument("--cpus", type=float, default=4)
    ap.add_argument("--mem", default="6g")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()

    vecs = vectors(a.n, a.dim, 7)
    probe = vecs[0]
    if a.engine == "atlas":
        eng = AtlasC(a.atlas_bin, a.dim, a.port, a.cpus, a.mem)
    else:
        eng = QdrantC(a.dim, a.port, a.cpus, a.mem)
    res = {"engine": a.engine, "n": a.n, "dim": a.dim, "cpus": a.cpus, "mem": a.mem,
           "reps": a.reps, "runs": []}
    try:
        eng.create()
        eng.prepare()
        for _ in range(400):
            try:
                eng.query_ok(probe)
                break
            except Exception as e:
                if getattr(e, "code", 0) in (400, 404):
                    break
                time.sleep(0.1)
        t0 = time.perf_counter()
        eng.insert_all(vecs, a.threads)
        res["load_seconds"] = time.perf_counter() - t0
        eng.settle(a.n)
        res["count_after_load"] = eng.count()
        print("loaded", res["count_after_load"], "in", round(res["load_seconds"], 1), "s", flush=True)
        for r in range(a.reps):
            eng.kill()
            ms, missing = restart_to_ready(eng, probe, a.n)
            res["runs"].append({"rep": r, "restart_to_ready_ms": ms, "acked_missing": missing})
            print(json.dumps(res["runs"][-1]), flush=True)
            eng.settle(a.n)
    finally:
        eng.remove()
    res["median_ms"] = statistics.median(x["restart_to_ready_ms"] for x in res["runs"])
    res["acked_missing_total"] = sum(x["acked_missing"] for x in res["runs"])
    with open(a.out, "w") as f:
        json.dump(res, f, indent=1)
    print("median_ms", res["median_ms"], "missing", res["acked_missing_total"])


if __name__ == "__main__":
    main()
