import sys, json, os, h5py, numpy as np
src, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
with h5py.File(src, "r") as f:
    tr, te = np.asarray(f["train"], dtype="<f4"), np.asarray(f["test"], dtype="<f4")
    metric = f.attrs.get("distance", "euclidean")
tr.tofile(f"{out}/train.f32"); te.tofile(f"{out}/test.f32")
json.dump({"name": os.path.basename(src).replace(".hdf5", ""), "dim": int(tr.shape[1]), "n": int(tr.shape[0]), "metric": str(metric)}, open(f"{out}/meta.json", "w"))
print(src, tr.shape, te.shape, metric)
