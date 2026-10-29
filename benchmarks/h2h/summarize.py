import json
import os
import sys

root = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "results")
ENGINES = ["atlas", "hnswlib", "qdrant"]


def load(ds, eng):
    p = os.path.join(root, ds, f"{eng}.json")
    return json.load(open(p)) if os.path.exists(p) else None


datasets = sorted(d for d in os.listdir(root) if os.path.isdir(os.path.join(root, d)))
for ds in datasets:
    res = {e: load(ds, e) for e in ENGINES}
    if not any(res.values()):
        continue
    print(f"\n## {ds}\n")
    print("### Unfiltered recall@10 vs latency (median of repetitions, sequential single client)\n")
    print("| engine | ef | recall@10 | p50 ms | p95 ms | p99 ms | QPS seq | QPS 8 workers | reps |")
    print("|---|---|---|---|---|---|---|---|---|")
    for e, r in res.items():
        for s in (r or {}).get("sweep", []):
            m = s["median"]
            print(f"| {e} | {s['ef']} | {m['recall_at_10']:.4f} | {m['p50_ms']:.3f} | {m['p95_ms']:.3f} | "
                  f"{m['p99_ms']:.3f} | {m['qps_seq']:.0f} | {s['qps_conc8']:.0f} | {len(s['reps'])} |")
    print("\n### Filtered search at ef=512 (each engine's native filtered path)\n")
    print("| engine | selectivity | path | recall@10 | p50 ms | p95 ms | p99 ms | avg returned | leaks (200 q) |")
    print("|---|---|---|---|---|---|---|---|---|")
    for e, r in res.items():
        for f in (r or {}).get("filtered", []):
            sel = f.get("selectivity") or f"{f['selectivity_permille'] / 10}%"
            print(f"| {e} | {sel} | {f['algorithm']} | {f['recall_at_10']:.3f} | {f['p50_ms']:.2f} | "
                  f"{f['p95_ms']:.2f} | {f['p99_ms']:.2f} | {f['avg_returned']:.1f} | {f['leaks_in_200q']} |")
    print("\n### Build, memory, disk\n")
    print("| engine | build s | ingest vec/s | peak RSS after build MB | disk MB | notes |")
    print("|---|---|---|---|---|---|")
    for e, r in res.items():
        if not r:
            continue
        hw = r.get("vmhwm_kb_after_build")
        disk = r.get("disk_bytes", r.get("graph_serialized_bytes"))
        note = r.get("mode", "")
        if e == "qdrant":
            qc = os.path.join(root, ds, "qdrant_container.txt")
            if os.path.exists(qc):
                note = " ".join(open(qc).read().split("\n")).strip()
        rss = hw / 1024 if hw and hw > 0 else float("nan")
        mb = disk / 1e6 if disk else float("nan")
        print(f"| {e} | {r['build_seconds']:.1f} | {r['ingest_vps']:.0f} | {rss:.0f} | {mb:.1f} | {note} |")
