//! S13 adapter profiler (diagnostic only; never a production benchmark).
//! Separates, for one dataset and several ef values, single-thread per-query cost of:
//!   floor  : raw USearch search at ef=K (FFI + fixed per-call cost + minimal traversal)
//!   native : raw USearch search at ef (traversal + FFI + Matches alloc), no rescoring
//!   rescore: adapter rescoring replica alone (flat lookup + metadata clone + exact score + sort)
//!   full   : native + rescore (== UsearchIndex::search body), with recall
//! Graph is built in parallel and cached (search cost does not depend on build threads).
//! The rescore replica mirrors adapters/atlas-usearch/src/lib.rs `rescore`; when n <= REAL_MAX
//! the real `UsearchIndex::search` is also timed as a cross-check of the replica.
use atlas_core::{Metric, VectorId};
use atlas_index::{score, AnnIndex, FlatIndex, Hit};
use atlas_usearch::UsearchIndex;
use rayon::prelude::*;
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

const K: usize = 10;
const REAL_MAX: usize = 100_000;
const OVER: usize = 50;
const FIXED_N: usize = 2000;

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
// SAFETY: forwards to System unchanged; only adds relaxed counters.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(l.size() as u64, Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
}
#[global_allocator]
static A: Counting = Counting;

fn arg(a: &[String], k: &str, d: &str) -> String {
    a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned().unwrap_or(d.into())
}

fn load_f32(path: &str) -> Vec<f32> {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// First GT block (unfiltered): nq*k i32 after the header.
fn load_gt(path: &str) -> (usize, usize, Vec<i32>) {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let (nq, k, nsel) = (rd32(&b, 0) as usize, rd32(&b, 4) as usize, rd32(&b, 8) as usize);
    let off = 12 + 4 * nsel;
    (nq, k, (0..nq * k).map(|i| rd32(&b, off + 4 * i) as i32).collect())
}

fn recall(found: &[u64], truth: &[i32]) -> f64 {
    let t: Vec<u64> = truth.iter().filter(|&&x| x >= 0).map(|&x| x as u64).collect();
    if t.is_empty() {
        return 1.0;
    }
    found.iter().filter(|f| t.contains(f)).count() as f64 / t.len() as f64
}

fn opts(d: usize, m: MetricKind, q: ScalarKind, ef: usize) -> IndexOptions {
    IndexOptions {
        dimensions: d,
        metric: m,
        quantization: q,
        connectivity: 32,
        expansion_add: 200,
        expansion_search: ef,
        multi: false,
    }
}

fn build_or_load(path: &str, base: &[f32], d: usize, m: MetricKind, q: ScalarKind) -> Index {
    let g = Index::new(&opts(d, m, q, 512)).expect("index");
    if std::path::Path::new(path).exists() {
        g.load(path).expect("load");
        eprintln!("loaded {path}");
        return g;
    }
    let n = base.len() / d;
    g.reserve(n).expect("reserve");
    let t = Instant::now();
    base.par_chunks_exact(d).enumerate().for_each(|(i, v)| g.add(i as u64, v).expect("add"));
    eprintln!("built n={n} (parallel) in {:.1}s", t.elapsed().as_secs_f64());
    g.save(path).expect("save");
    g
}

struct Pass {
    p50: f64,
    p95: f64,
    p99: f64,
    mean: f64,
    recall: f64,
    allocs_q: f64,
    bytes_q: f64,
}

/// Timed single-thread passes over nq queries (after a warm-up); returns the median-mean rep.
fn pass<F: FnMut(&[f32]) -> Vec<u64>>(
    qs: &[f32],
    d: usize,
    nq: usize,
    gt: &[i32],
    gk: usize,
    reps: usize,
    mut f: F,
) -> Pass {
    for i in 0..200.min(nq) {
        let _ = f(&qs[i * d..(i + 1) * d]);
    }
    let mut runs: Vec<Pass> = (0..reps)
        .map(|_| {
            let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
            let (mut lat, mut rec) = (Vec::with_capacity(nq), 0.0);
            let mut alloc_spent = (0u64, 0u64);
            for i in 0..nq {
                let s = Instant::now();
                let r = f(&qs[i * d..(i + 1) * d]);
                lat.push(s.elapsed().as_secs_f64() * 1e3);
                // recall() allocates; subtract its allocations so counts reflect the callee only
                let (x0, y0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
                rec += recall(&r, &gt[i * gk..(i + 1) * gk]);
                alloc_spent.0 += ALLOCS.load(Relaxed) - x0;
                alloc_spent.1 += BYTES.load(Relaxed) - y0;
            }
            let (a1, b1) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
            lat.sort_by(|a, b| a.total_cmp(b));
            let p = |q: f64| lat[((nq as f64 * q) as usize).min(nq - 1)];
            Pass {
                p50: p(0.5),
                p95: p(0.95),
                p99: p(0.99),
                mean: lat.iter().sum::<f64>() / nq as f64,
                recall: rec / nq as f64,
                allocs_q: (a1 - a0 - alloc_spent.0) as f64 / nq as f64,
                bytes_q: (b1 - b0 - alloc_spent.1) as f64 / nq as f64,
            }
        })
        .collect();
    runs.sort_by(|a, b| a.mean.total_cmp(&b.mean));
    runs.swap_remove(reps / 2)
}

fn js(p: &Pass) -> Value {
    json!({"mean_ms": p.mean, "p50_ms": p.p50, "p95_ms": p.p95, "p99_ms": p.p99,
           "recall_at_10": p.recall, "allocs_per_query": p.allocs_q, "bytes_alloc_per_query": p.bytes_q})
}

/// 8-thread QPS over 3x the query set (timing wall clock of the whole parallel region).
fn par_qps<F: Fn(&Index, &[f32]) + Sync>(g: &Index, qs: &[f32], d: usize, nq: usize, f: F) -> f64 {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(8).build().expect("pool");
    let run = || (0..3 * nq).into_par_iter().for_each(|i| f(g, &qs[(i % nq) * d..(i % nq + 1) * d]));
    pool.install(|| run());
    let t = Instant::now();
    pool.install(|| run());
    (3 * nq) as f64 / t.elapsed().as_secs_f64()
}

fn rescore(flat: &FlatIndex, q: &[f32], k: usize, metric: Metric, keys: &[u64]) -> Vec<Hit> {
    let mut hits: Vec<Hit> = keys
        .iter()
        .filter_map(|&key| {
            let (v, m) = flat.get(VectorId(key))?;
            Some(Hit { id: VectorId(key), score: score(metric, q, v), metadata: m.clone() })
        })
        .collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    hits.truncate(k);
    hits
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = arg(&a, "--dir", "");
    let out = arg(&a, "--out", "profile.json");
    let nq_arg: usize = arg(&a, "--queries", "2000").parse().expect("queries");
    let reps: usize = arg(&a, "--reps", "3").parse().expect("reps");
    let efs: Vec<usize> =
        arg(&a, "--ef", "64,256,512").split(',').map(|x| x.parse().expect("ef")).collect();
    let want_ip = a.iter().any(|x| x == "--ip");
    let meta: Value =
        serde_json::from_slice(&std::fs::read(format!("{dir}/meta.json")).expect("meta"))
            .expect("json");
    let d = meta["dim"].as_u64().expect("dim") as usize;
    let euclid = meta["metric"] == "euclidean";
    let (metric, mk) =
        if euclid { (Metric::Euclidean, MetricKind::L2sq) } else { (Metric::Cosine, MetricKind::Cos) };
    let base = load_f32(&format!("{dir}/train.f32"));
    let qall = load_f32(&format!("{dir}/test.f32"));
    let (gnq, gk, gt) = load_gt(&format!("{dir}/gt.bin"));
    let n = base.len() / d;
    let nq = nq_arg.min(gnq);
    let qs = &qall[..nq * d];
    eprintln!(
        "accel compiled={} available={}",
        usearch::hardware_acceleration_compiled(),
        usearch::hardware_acceleration_available()
    );

    let g = build_or_load(&format!("{dir}/profile-{mk:?}.usearch"), &base, d, mk, ScalarKind::F32);
    let mut flat = FlatIndex::new(d);
    for (i, v) in base.chunks_exact(d).enumerate() {
        flat.insert(VectorId(i as u64), v, Value::Null).expect("flat");
    }
    let g_ip = (want_ip && !euclid)
        .then(|| build_or_load(&format!("{dir}/profile-IP.usearch"), &base, d, MetricKind::IP, ScalarKind::F32));
    let g16 = a.iter().any(|x| x == "--f16").then(|| {
        build_or_load(&format!("{dir}/profile-{mk:?}-F16.usearch"), &base, d, mk, ScalarKind::F16)
    });
    let mut real = (n <= REAL_MAX).then(|| {
        let mut r = UsearchIndex::with_metric(d, metric);
        for (i, v) in base.chunks_exact(d).enumerate() {
            r.insert(VectorId(i as u64), v, Value::Null).expect("real insert");
        }
        r
    });

    let mut rows = Vec::new();
    g.change_expansion_search(K);
    let floor = pass(qs, d, nq, &gt, gk, reps, |q| g.search(q, K).expect("s").keys);
    for &ef in &efs {
        g.change_expansion_search(ef);
        let native = pass(qs, d, nq, &gt, gk, reps, |q| g.search(q, K).expect("s").keys);
        let keys: Vec<Vec<u64>> =
            (0..nq).map(|i| g.search(&qs[i * d..(i + 1) * d], K).expect("s").keys).collect();
        let mut qi = 0usize;
        let resc = pass(qs, d, nq, &gt, gk, reps, |q| {
            let r = rescore(&flat, q, K, metric, &keys[qi % nq]);
            qi += 1;
            r.into_iter().map(|h| h.id.0).collect()
        });
        let full = pass(qs, d, nq, &gt, gk, reps, |q| {
            let m = g.search(q, K).expect("s");
            rescore(&flat, q, K, metric, &m.keys).into_iter().map(|h| h.id.0).collect()
        });
        let mut row = json!({"ef": ef, "floor_ef10": js(&floor), "native": js(&native),
            "rescore_only": js(&resc), "full_replica": js(&full)});
        if let Some(r) = real.as_mut() {
            r.set_ef_search(ef);
            let r = &*r;
            row["real_adapter"] = js(&pass(qs, d, nq, &gt, gk, reps, |q| {
                r.search(q, K, metric).expect("s").into_iter().map(|h| h.id.0).collect()
            }));
        }
        // exact-rescore candidate budget: ask the graph for OVER candidates (free: ef list is
        // already computed), rescore exactly from the f32 flat copy, keep K.
        let over = |gx: &Index, q: &[f32]| -> Vec<u64> {
            let m = gx.search(q, OVER).expect("s");
            rescore(&flat, q, K, metric, &m.keys).into_iter().map(|h| h.id.0).collect()
        };
        row["f32_over_rescore"] = js(&pass(qs, d, nq, &gt, gk, reps, |q| over(&g, q)));
        if let Some(g16) = g16.as_ref() {
            g16.change_expansion_search(ef);
            row["f16_native"] = js(&pass(qs, d, nq, &gt, gk, reps, |q| g16.search(q, K).expect("s").keys));
            row["f16_over_rescore"] = js(&pass(qs, d, nq, &gt, gk, reps, |q| over(g16, q)));
        }
        row["qps8_native"] = json!(par_qps(&g, qs, d, nq, |gx, q| drop(gx.search(q, K))));
        row["qps8_full_replica"] = json!(par_qps(&g, qs, d, nq, |gx, q| {
            let m = gx.search(q, K).expect("s");
            drop(rescore(&flat, q, K, metric, &m.keys));
        }));
        if let Some(gi) = g_ip.as_ref() {
            gi.change_expansion_search(ef);
            row["native_ip"] = js(&pass(qs, d, nq, &gt, gk, reps, |q| gi.search(q, K).expect("s").keys));
        }
        eprintln!(
            "ef={ef} native={:.3}ms rescore={:.3}ms full={:.3}ms recall={:.4}",
            native.mean, resc.mean, full.mean, full.recall
        );
        rows.push(row);
    }
    // fixed per-call cost: tiny index so traversal is negligible (FFI + context + Matches alloc)
    let tiny = Index::new(&opts(d, mk, ScalarKind::F32, K)).expect("tiny");
    tiny.reserve(FIXED_N).expect("r");
    for (i, v) in base.chunks_exact(d).take(FIXED_N).enumerate() {
        tiny.add(i as u64, v).expect("add");
    }
    let fixed = pass(qs, d, nq, &gt, gk, reps, |q| tiny.search(q, K).expect("s").keys);
    let rec = json!({"fixed_overhead_tiny_index": js(&fixed), "dataset": dir, "n": n, "dim": d, "nq": nq, "reps": reps,
        "metric": format!("{metric:?}"),
        "accel_compiled": usearch::hardware_acceleration_compiled(),
        "accel_available": usearch::hardware_acceleration_available(),
        "note": "single thread, per-query aggregate timing; 'native' (no rescore) is diagnostic only",
        "rows": rows});
    std::fs::write(&out, serde_json::to_vec_pretty(&rec).expect("ser")).expect("write");
}
