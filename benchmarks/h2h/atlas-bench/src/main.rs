use atlas_core::{Metric, VectorId};
use atlas_index::AnnIndex;
use atlas_usearch::UsearchIndex;
use rayon::prelude::*;
use serde_json::{json, Value};
use std::time::Instant;

const K: usize = 10;
const FILTER_EF: usize = 512;
const WARM: usize = 200;

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

fn bucket(id: u64) -> u64 {
    mix(id ^ 0xF117E4) % 1000
}

fn arg(a: &[String], k: &str, d: &str) -> String {
    a.iter()
        .position(|x| x == k)
        .and_then(|i| a.get(i + 1))
        .cloned()
        .unwrap_or_else(|| d.into())
}

fn load_f32(path: &str) -> Vec<f32> {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn rd32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

struct Gt {
    nq: usize,
    k: usize,
    sels: Vec<u32>,
    blocks: Vec<Vec<i32>>,
}

fn load_gt(path: &str) -> Gt {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let (nq, k, nsel) = (
        rd32(&b, 0) as usize,
        rd32(&b, 4) as usize,
        rd32(&b, 8) as usize,
    );
    let mut off = 12;
    let sels: Vec<u32> = (0..nsel).map(|i| rd32(&b, off + 4 * i)).collect();
    off += 4 * nsel;
    let mut blocks = Vec::new();
    for _ in 0..=nsel {
        let blk = (0..nq * k)
            .map(|i| rd32(&b, off + 4 * i) as i32)
            .collect::<Vec<_>>();
        off += 4 * nq * k;
        blocks.push(blk);
    }
    Gt {
        nq,
        k,
        sels,
        blocks,
    }
}

fn vmhwm_kb() -> i64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        })
        .unwrap_or(-1)
}

fn recall(found: &[u64], truth: &[i32]) -> f64 {
    let t: Vec<u64> = truth
        .iter()
        .filter(|&&x| x >= 0)
        .map(|&x| x as u64)
        .collect();
    if t.is_empty() {
        return 1.0;
    }
    found.iter().filter(|f| t.contains(f)).count() as f64 / t.len() as f64
}

fn pct(mut ms: Vec<f64>) -> Value {
    ms.sort_by(|a, b| a.total_cmp(b));
    let p = |q: f64| ms[((ms.len() as f64 * q) as usize).min(ms.len() - 1)];
    json!({"p50_ms": p(0.5), "p95_ms": p(0.95), "p99_ms": p(0.99), "mean_ms": ms.iter().sum::<f64>() / ms.len() as f64})
}

fn seq_pass<F: Fn(&[f32]) -> Vec<u64>>(
    qs: &[f32],
    d: usize,
    nq: usize,
    gt: &[i32],
    k: usize,
    warm: usize,
    f: F,
) -> Value {
    for i in 0..warm.min(nq) {
        let _ = f(&qs[i * d..(i + 1) * d]);
    }
    let (mut lat, mut rec, mut ret) = (Vec::with_capacity(nq), 0.0, 0usize);
    let t0 = Instant::now();
    for i in 0..nq {
        let s = Instant::now();
        let r = f(&qs[i * d..(i + 1) * d]);
        lat.push(s.elapsed().as_secs_f64() * 1e3);
        rec += recall(&r, &gt[i * k..(i + 1) * k]);
        ret += r.len();
    }
    let wall = t0.elapsed().as_secs_f64();
    let mut v = pct(lat);
    v["recall_at_10"] = json!(rec / nq as f64);
    v["qps_seq"] = json!(nq as f64 / wall);
    v["avg_returned"] = json!(ret as f64 / nq as f64);
    v
}

fn median(reps: &[Value]) -> Value {
    let mut out = serde_json::Map::new();
    for key in [
        "p50_ms",
        "p95_ms",
        "p99_ms",
        "mean_ms",
        "recall_at_10",
        "qps_seq",
    ] {
        let mut xs: Vec<f64> = reps.iter().filter_map(|r| r[key].as_f64()).collect();
        xs.sort_by(|a, b| a.total_cmp(b));
        out.insert(key.into(), json!(xs[xs.len() / 2]));
    }
    Value::Object(out)
}

fn ids(hits: Vec<atlas_index::Hit>) -> Vec<u64> {
    hits.into_iter().map(|h| h.id.0).collect()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = arg(&a, "--dir", "");
    let out = arg(&a, "--out", "atlas.json");
    let nq_arg: usize = arg(&a, "--queries", "10000").parse().expect("queries");
    let reps: usize = arg(&a, "--reps", "3").parse().expect("reps");
    let fq_arg: usize = arg(&a, "--fq", "10000").parse().expect("fq");
    let smoke = a.iter().any(|x| x == "--smoke");
    let efs: Vec<usize> = if smoke {
        vec![64, 512]
    } else {
        vec![64, 128, 256, 512, 1024]
    };
    let meta: Value =
        serde_json::from_slice(&std::fs::read(format!("{dir}/meta.json")).expect("meta"))
            .expect("meta json");
    let d = meta["dim"].as_u64().expect("dim") as usize;
    let metric = if meta["metric"] == "euclidean" {
        Metric::Euclidean
    } else {
        Metric::Cosine
    };
    let base = load_f32(&format!("{dir}/train.f32"));
    let queries_all = load_f32(&format!("{dir}/test.f32"));
    let gt = load_gt(&format!("{dir}/gt.bin"));
    let n = base.len() / d;
    let nq = nq_arg.min(gt.nq);
    let fq = fq_arg.min(nq);
    let queries = &queries_all[..nq * d];
    let hbin = std::fs::read(format!("{dir}/h.bin")).expect("h.bin");
    for id in 0..n {
        let v = u16::from_le_bytes([hbin[2 * id], hbin[2 * id + 1]]) as u64;
        assert_eq!(v, bucket(id as u64), "filter hash mismatch at id {id}");
    }

    let mut idx = UsearchIndex::with_metric(d, metric);
    let t = Instant::now();
    for (i, v) in base.chunks_exact(d).enumerate() {
        idx.insert(VectorId(i as u64), v, Value::Null)
            .expect("insert");
    }
    let build = t.elapsed().as_secs_f64();
    let hw_build = vmhwm_kb();
    let graph_bytes = idx.graph_serialized_len();
    eprintln!("built n={n} in {build:.1}s");

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .expect("pool");
    let mut sweep = Vec::new();
    for &ef in &efs {
        idx.set_ef_search(ef);
        let mut rs = Vec::new();
        for _ in 0..reps {
            rs.push(seq_pass(queries, d, nq, &gt.blocks[0], gt.k, WARM, |q| {
                ids(idx.search(q, K, metric).expect("search"))
            }));
        }
        let t = Instant::now();
        pool.install(|| {
            (0..nq).into_par_iter().for_each(|i| {
                let _ = idx.search(&queries[i * d..(i + 1) * d], K, metric);
            })
        });
        let conc = nq as f64 / t.elapsed().as_secs_f64();
        let med = median(&rs);
        eprintln!(
            "atlas ef={ef} recall={:.4} p95={:.3}ms",
            med["recall_at_10"], med["p95_ms"]
        );
        sweep.push(json!({"ef": ef, "reps": rs, "median": med, "qps_conc8": conc}));
    }

    idx.set_ef_search(FILTER_EF);
    let mut filtered = Vec::new();
    for (si, &pm) in gt.sels.iter().enumerate() {
        let pm64 = pm as u64;
        let allowed = (0..n as u64).filter(|&i| bucket(i) < pm64).count();
        let route = if atlas_usearch::use_exact_scan(n, allowed, FILTER_EF) {
            "exact pre-filter scan over allowed ids"
        } else {
            "native filtered HNSW walk"
        };
        let allow = move |id: VectorId| bucket(id.0) < pm64;
        let r = seq_pass(queries, d, fq, &gt.blocks[1 + si], gt.k, 20, |q| {
            ids(idx.search_filtered(q, K, metric, &allow).expect("filtered"))
        });
        let leaks: usize = (0..nq.min(200))
            .map(|i| {
                ids(idx
                    .search_filtered(&queries[i * d..(i + 1) * d], K, metric, &allow)
                    .expect("filtered"))
                .iter()
                .filter(|&&x| bucket(x) >= pm64)
                .count()
            })
            .sum();
        eprintln!(
            "atlas sel={}permille recall={:.3} p95={:.1}ms route={route}",
            pm, r["recall_at_10"], r["p95_ms"]
        );
        let mut rec = r;
        rec["selectivity_permille"] = json!(pm);
        rec["allowed"] = json!(allowed);
        rec["ef"] = json!(FILTER_EF);
        rec["algorithm"] = json!(route);
        rec["leaks_in_200q"] = json!(leaks);
        filtered.push(rec);
    }

    let res = json!({
        "engine": "atlas-usearch-adapter",
        "mode": "embedded library via AnnIndex trait, no WAL, no API",
        "params": {"M": 32, "ef_construction": 200, "metric": format!("{metric:?}"), "build_threads": 1},
        "n": n, "dim": d, "queries": nq, "filtered_queries": fq, "reps": reps, "warmup": WARM,
        "build_seconds": build, "ingest_vps": n as f64 / build,
        "vmhwm_kb_after_build": hw_build, "vmhwm_kb_end": vmhwm_kb(),
        "graph_serialized_bytes": graph_bytes,
        "disk_note": "graph is derived data and never persisted by Atlas; graph_serialized_bytes is the size if saved",
        "sweep": sweep, "filtered": filtered,
    });
    std::fs::write(&out, serde_json::to_string_pretty(&res).expect("json")).expect("write");
}
