use rayon::prelude::*;
use serde_json::{json, Value};
use std::time::Instant;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

const K: usize = 10;
const SELS: [(&str, u64); 5] = [
    ("10%", 100),
    ("5%", 50),
    ("3%", 30),
    ("1%", 10),
    ("0.1%", 1),
];
const HYBRID_EXACT_MAX_PERMILLE: usize = 100;

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}
fn passes(id: u64, permille: u64) -> bool {
    mix(id ^ 0xF117_E4) % 1000 < permille
}
fn arg(a: &[String], k: &str, d: &str) -> String {
    a.iter()
        .position(|x| x == k)
        .and_then(|i| a.get(i + 1))
        .cloned()
        .unwrap_or(d.into())
}
fn load(path: &str, d: usize) -> Vec<f32> {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert_eq!(b.len() % (4 * d), 0);
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}
fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.iter_mut().for_each(|x| *x /= n);
}
fn dist(angular: bool, a: &[f32], b: &[f32]) -> f32 {
    if angular {
        1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()
    } else {
        a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
    }
}
fn exact(
    angular: bool,
    d: usize,
    base: &[f32],
    q: &[f32],
    ids: impl Iterator<Item = u64>,
) -> Vec<u64> {
    let mut top: Vec<(f32, u64)> = Vec::with_capacity(K + 1);
    for id in ids {
        let s = dist(angular, q, &base[id as usize * d..(id as usize + 1) * d]);
        if top.len() < K || s < top[K - 1].0 {
            let pos = top.partition_point(|&(x, i)| (x, i) < (s, id));
            top.insert(pos, (s, id));
            top.truncate(K);
        }
    }
    top.into_iter().map(|(_, i)| i).collect()
}
fn recall(found: &[u64], truth: &[u64]) -> f64 {
    if truth.is_empty() {
        return 1.0;
    }
    found.iter().filter(|k| truth.contains(k)).count() as f64 / truth.len() as f64
}
fn pct(mut ms: Vec<f64>) -> Value {
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| ms[((ms.len() as f64 * q) as usize).min(ms.len() - 1)];
    json!({"p50_ms": p(0.5), "p95_ms": p(0.95), "p99_ms": p(0.99), "mean_ms": ms.iter().sum::<f64>() / ms.len() as f64})
}
fn timed<F: FnMut(&[f32]) -> Vec<u64>>(qs: &[Vec<f32>], truth: &[Vec<u64>], mut f: F) -> Value {
    for q in qs.iter().take(20) {
        let _ = f(q);
    }
    let (mut lat, mut rec) = (Vec::new(), 0.0);
    for (q, t) in qs.iter().zip(truth) {
        let s = Instant::now();
        let r = f(q);
        lat.push(s.elapsed().as_secs_f64() * 1e3);
        rec += recall(&r, t);
    }
    let total: f64 = lat.iter().sum();
    let mut v = pct(lat);
    v["recall_at_10"] = json!(rec / qs.len() as f64);
    v["qps_single_thread"] = json!(qs.len() as f64 / (total / 1e3));
    v
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = arg(&a, "--dir", "");
    let nq: usize = arg(&a, "--queries", "1000").parse().unwrap();
    let out = arg(&a, "--out", "result.json");
    let meta: Value =
        serde_json::from_slice(&std::fs::read(format!("{dir}/meta.json")).unwrap()).unwrap();
    let d = meta["dim"].as_u64().unwrap() as usize;
    let angular = meta["metric"] == "angular";
    let mut base = load(&format!("{dir}/train.f32"), d);
    let mut test = load(&format!("{dir}/test.f32"), d);
    if angular {
        base.chunks_mut(d).for_each(normalize);
        test.chunks_mut(d).for_each(normalize);
    }
    let n = base.len() / d;
    let queries: Vec<Vec<f32>> = test.chunks(d).take(nq).map(|c| c.to_vec()).collect();
    eprintln!(
        "{dir}: n={n} d={d} angular={angular} queries={}",
        queries.len()
    );

    let t = Instant::now();
    let truth: Vec<Vec<u64>> = queries
        .par_iter()
        .map(|q| exact(angular, d, &base, q, 0..n as u64))
        .collect();
    let gt_secs = t.elapsed().as_secs_f64();
    let ftruth: Vec<Vec<Vec<u64>>> = SELS
        .iter()
        .map(|&(_, pm)| {
            queries
                .par_iter()
                .map(|q| {
                    exact(
                        angular,
                        d,
                        &base,
                        q,
                        (0..n as u64).filter(|&i| passes(i, pm)),
                    )
                })
                .collect()
        })
        .collect();
    eprintln!("ground truth {gt_secs:.0}s");

    let m: usize = arg(&a, "--m", "16").parse().unwrap();
    let ef_add: usize = arg(&a, "--efc", "128").parse().unwrap();
    let ef_list: Vec<usize> = arg(&a, "--efs", "32,64,128,256")
        .split(',')
        .map(|x| x.parse().unwrap())
        .collect();
    let o = IndexOptions {
        dimensions: d,
        metric: if angular {
            MetricKind::Cos
        } else {
            MetricKind::L2sq
        },
        quantization: ScalarKind::F32,
        connectivity: m,
        expansion_add: ef_add,
        expansion_search: 128,
        multi: false,
    };
    let index = Index::new(&o).unwrap();
    index.reserve(n).unwrap();
    let t = Instant::now();
    base.par_chunks(d)
        .enumerate()
        .for_each(|(i, v)| index.add(i as u64, v).unwrap());
    let build_s = t.elapsed().as_secs_f64();
    eprintln!("built in {build_s:.1}s");

    let mut sweep = vec![];
    for &ef in &ef_list {
        index.change_expansion_search(ef);
        let mut r = timed(&queries, &truth, |q| index.search(q, K).unwrap().keys);
        r["expansion_search"] = json!(ef);
        eprintln!(
            "ef={ef} recall={:.4} p95={:.3}ms",
            r["recall_at_10"], r["p95_ms"]
        );
        sweep.push(r);
    }
    index.change_expansion_search(128);
    let mut filtered = vec![];
    for (si, &(label, pm)) in SELS.iter().enumerate() {
        let native = timed(&queries, &ftruth[si], |q| {
            index
                .filtered_search(q, K, move |k| passes(k, pm))
                .unwrap()
                .keys
        });
        let hybrid = timed(&queries, &ftruth[si], |q| {
            let allowed: Vec<u64> = (0..n as u64).filter(|&i| passes(i, pm)).collect();
            if allowed.len() * 1000 <= n * HYBRID_EXACT_MAX_PERMILLE {
                exact(angular, d, &base, q, allowed.into_iter())
            } else {
                index
                    .filtered_search(q, K, move |k| passes(k, pm))
                    .unwrap()
                    .keys
            }
        });
        let leaks: usize = queries
            .iter()
            .map(|q| {
                index
                    .filtered_search(q, K, move |k| passes(k, pm))
                    .unwrap()
                    .keys
                    .iter()
                    .filter(|&&k| !passes(k, pm))
                    .count()
            })
            .sum();
        eprintln!(
            "sel={label} native r={:.3} p95={:.1}ms | hybrid r={:.3} p95={:.1}ms leaks={leaks}",
            native["recall_at_10"], native["p95_ms"], hybrid["recall_at_10"], hybrid["p95_ms"]
        );
        filtered.push(
            json!({"selectivity": label, "native": native, "hybrid": hybrid, "leaks": leaks}),
        );
    }
    let res = json!({
        "dataset": meta["name"], "n": n, "dim": d, "metric": meta["metric"], "queries": queries.len(),
        "params": {"M": m, "ef_construction": ef_add, "k": K, "scalar": "f32", "threads_build": rayon::current_num_threads()},
        "ground_truth": "exact brute force computed here (not the dataset-supplied neighbours)",
        "source_digest": arg(&a, "--digest", "unset"),
        "build_seconds": build_s, "unfiltered_sweep": sweep, "filtered": filtered,
    });
    std::fs::create_dir_all("results").ok();
    std::fs::write(&out, serde_json::to_string_pretty(&res).unwrap()).unwrap();
}
