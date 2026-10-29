import json
import os
import struct
import sys
import time
import numpy as np
from common import K, SELS, hbucket

src, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
meta = json.load(open(f"{src}/meta.json"))
dim = meta["dim"]
X = np.fromfile(f"{src}/train.f32", dtype="<f4").reshape(-1, dim)
Q = np.fromfile(f"{src}/test.f32", dtype="<f4").reshape(-1, dim)
ang = meta["metric"] == "angular"
if ang:
    X = X / np.maximum(np.linalg.norm(X, axis=1, keepdims=True), 1e-12)
    Q = Q / np.maximum(np.linalg.norm(Q, axis=1, keepdims=True), 1e-12)
X = np.ascontiguousarray(X, dtype="<f4")
Q = np.ascontiguousarray(Q, dtype="<f4")
X.tofile(f"{out}/train.f32")
Q.tofile(f"{out}/test.f32")
json.dump(meta, open(f"{out}/meta.json", "w"))
n, nq = len(X), len(Q)
h = hbucket(n)
h.astype("<u2").tofile(f"{out}/h.bin")


def topk(ids):
    Xs = X[ids]
    xn = None if ang else (Xs * Xs).sum(1)
    res = np.full((nq, K), -1, dtype=np.int32)
    for q0 in range(0, nq, 500):
        qb = Q[q0:q0 + 500]
        bs = np.full((len(qb), K), -np.inf, dtype=np.float32)
        bi = np.zeros((len(qb), K), dtype=np.int64)
        for c0 in range(0, len(ids), 200_000):
            c = Xs[c0:c0 + 200_000]
            s = qb @ c.T
            if not ang:
                s = 2 * s - xn[c0:c0 + 200_000][None, :]
            kk = min(K, s.shape[1])
            p = np.argpartition(-s, kk - 1, axis=1)[:, :kk]
            ps = np.take_along_axis(s, p, 1)
            pi = p + c0
            ms = np.concatenate([bs, ps], 1)
            mi = np.concatenate([bi, pi], 1)
            sel = np.argsort(-ms, axis=1, kind="stable")[:, :K]
            bs = np.take_along_axis(ms, sel, 1)
            bi = np.take_along_axis(mi, sel, 1)
        res[q0:q0 + len(qb)] = np.where(np.isfinite(bs), ids[bi], -1)
    return res


t = time.time()
blocks = [topk(np.arange(n))]
for _, pm in SELS:
    blocks.append(topk(np.nonzero(h < pm)[0]))
with open(f"{out}/gt.bin", "wb") as f:
    f.write(struct.pack("<3I", nq, K, len(SELS)))
    f.write(struct.pack(f"<{len(SELS)}I", *[pm for _, pm in SELS]))
    for b in blocks:
        f.write(b.astype("<i4").tobytes())
print(f"{src}: n={n} dim={dim} nq={nq} metric={meta['metric']} gt in {time.time()-t:.0f}s")
