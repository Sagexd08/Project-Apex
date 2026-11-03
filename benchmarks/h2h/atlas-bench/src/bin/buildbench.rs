//! S14 build-throughput bench (diagnostic). Raw USearch graph build only (no Atlas flat store/WAL),
//! F32 vs F16, at a given thread count. Compare with build_bench.py (hnswlib) at the same threads.
use rayon::prelude::*;
use serde_json::json;
use std::time::Instant;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

fn arg(a: &[String], k: &str, d: &str) -> String {
    a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned().unwrap_or(d.into())
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = arg(&a, "--dir", "");
    let out = arg(&a, "--out", "build.json");
    let threads: usize = arg(&a, "--threads", "1").parse().expect("threads");
    let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(format!("{dir}/meta.json")).expect("meta")).expect("json");
    let d = meta["dim"].as_u64().expect("dim") as usize;
    let mk = if meta["metric"] == "euclidean" { MetricKind::L2sq } else { MetricKind::Cos };
    let b = std::fs::read(format!("{dir}/train.f32")).expect("train");
    let base: Vec<f32> = b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let n = base.len() / d;
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("pool");
    let mut rows = Vec::new();
    for (name, q) in [("f32", ScalarKind::F32), ("f16", ScalarKind::F16)] {
        let o = IndexOptions { dimensions: d, metric: mk, quantization: q, connectivity: 32, expansion_add: 200, expansion_search: 512, multi: false };
        let g = Index::new(&o).expect("new");
        g.reserve(n).expect("reserve");
        let t = Instant::now();
        pool.install(|| (0..n).into_par_iter().for_each(|i| g.add(i as u64, &base[i * d..(i + 1) * d]).expect("add")));
        let s = t.elapsed().as_secs_f64();
        rows.push(json!({"quant": name, "threads": threads, "n": n, "build_seconds": s, "vps": n as f64 / s,
            "memory_usage_bytes": g.memory_usage(), "serialized_bytes": g.serialized_length()}));
        eprintln!("{name} threads={threads} build={s:.1}s mem={}", g.memory_usage());
    }
    std::fs::write(out, serde_json::to_string_pretty(&rows).expect("j")).expect("w");
}
