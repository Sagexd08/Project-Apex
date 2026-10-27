//! Interleaved, repeated timing of the exact scan against the cascade. Synthetic data only.
//!
//! Every query is timed once per kernel in a rotating order, repeated `reps` times, so slow drift
//! (CPU frequency, other processes) hits every kernel alike. The per-repetition p50 of each kernel
//! is divided by the exact scan's p50 from the same repetition, and a bootstrap over repetitions
//! gives a 95% interval for the median ratio. Writes results/timing_interleaved.json.
//!
//! prism-timing [--n 20000,100000,400000] [--reps 20] [--queries 200]

use prism::rng::SplitMix64;
use prism::search::{calibrate, search_cascade_lut, search_exact_fast, search_exact_stream, search_flat_lut, Calib, Gate};
use prism::store::{Epoch, Snapshot, Store};
use prism::util::{centres, dataset_digest, mixture, obj, pct, src_digest_lf, J};
use std::time::Instant;

const D: usize = 256;
const K: usize = 10;
const SCHED: [usize; 3] = [1, 2, 4];
const WARMUP: usize = 20;
const BOOTSTRAP: usize = 4000;
const NAMES: [&str; 5] = ["exact_scan_opt", "exact_scan_stream", "flat4_lut_rerank100", "cascade_lut_z2", "cascade_lut_z3"];
/// Kernels 0 and 1 are exact scans; the baseline of each repetition is the faster of the two.
const RECALL_QUERIES: usize = 50;

fn run_ids(i: usize, ep: &Epoch, snap: &Snapshot, q: &[f32], calib: &Calib) -> Vec<(u64, f32)> {
    match i {
        0 => search_exact_fast(snap, q, K),
        1 => search_exact_stream(snap, q, K),
        2 => search_flat_lut(ep, snap, q, K, 4, 100).0,
        3 => search_cascade_lut(ep, snap, q, K, &SCHED, Gate::Global(2.0), calib).0,
        _ => search_cascade_lut(ep, snap, q, K, &SCHED, Gate::Global(3.0), calib).0,
    }
}

fn run_kernel(i: usize, ep: &Epoch, snap: &Snapshot, q: &[f32], calib: &Calib) -> usize {
    run_ids(i, ep, snap, q, calib).len()
}

/// Mean recall@K of kernel `i` against the exact scan over the first queries.
fn recall(i: usize, ep: &Epoch, snap: &Snapshot, qs: &[Vec<f32>], calib: &Calib) -> f64 {
    let n = RECALL_QUERIES.min(qs.len());
    let hits: usize = qs[..n]
        .iter()
        .map(|q| {
            let truth = run_ids(1, ep, snap, q, calib);
            run_ids(i, ep, snap, q, calib).iter().filter(|h| truth.iter().any(|t| t.0 == h.0)).count()
        })
        .sum();
    hits as f64 / (n * K) as f64
}

fn build_store(data: &[Vec<f32>]) -> Store {
    let store = Store::new(Epoch::new(0xA71A5, D, SCHED.to_vec()));
    for (c, chunk) in data.chunks(2000).enumerate() {
        store.commit(chunk.iter().enumerate().map(|(i, v)| ((c * 2000 + i) as u64, v.clone())).collect());
    }
    store.compact(usize::MAX);
    store
}

fn num(x: f64) -> J {
    J::Num(x)
}

fn bootstrap_ci(ratios: &[f64], rng: &mut SplitMix64) -> (f64, f64) {
    let meds: Vec<f64> = (0..BOOTSTRAP)
        .map(|_| {
            let sample: Vec<f64> = (0..ratios.len()).map(|_| ratios[rng.below(ratios.len())]).collect();
            pct(&sample, 50.0)
        })
        .collect();
    (pct(&meds, 2.5), pct(&meds, 97.5))
}

fn verdict(lo: f64, hi: f64) -> &'static str {
    if hi < 1.0 {
        "faster than the exact scan (95% interval excludes 1)"
    } else if lo > 1.0 {
        "slower than the exact scan (95% interval excludes 1)"
    } else {
        "inconclusive (95% interval includes 1)"
    }
}

fn measure(n: usize, reps: usize, nq: usize) -> J {
    println!("== n = {n}, {reps} interleaved repetitions of {nq} queries");
    let mut rng = SplitMix64::new(7);
    let cs = centres(&mut rng, 50, D);
    let data = mixture(&mut rng, &cs, n, 0.6, 0.0);
    let qs = mixture(&mut rng, &cs, nq, 0.6, 0.0);
    let digest = dataset_digest(&[&data[..], &qs[..]]);
    let store = build_store(&data);
    let (ep, snap) = (store.epoch(), store.pin());
    let calib = calibrate(ep, &snap, &qs[..100.min(nq)], 1000);
    for k in 0..NAMES.len() {
        qs.iter().take(WARMUP).for_each(|q| {
            std::hint::black_box(run_kernel(k, ep, &snap, q, &calib));
        });
    }
    // lat[rep][kernel][query]
    let mut lat = vec![vec![Vec::with_capacity(nq); NAMES.len()]; reps];
    for (r, per_rep) in lat.iter_mut().enumerate() {
        for (qi, q) in qs.iter().enumerate() {
            for step in 0..NAMES.len() {
                let k = (step + r + qi) % NAMES.len();
                let t = Instant::now();
                std::hint::black_box(run_kernel(k, ep, &snap, q, &calib));
                per_rep[k].push(t.elapsed().as_secs_f64() * 1e3);
            }
        }
    }
    let p50s: Vec<Vec<f64>> = (0..NAMES.len()).map(|k| lat.iter().map(|rep| pct(&rep[k], 50.0)).collect()).collect();
    let p95s: Vec<Vec<f64>> = (0..NAMES.len()).map(|k| lat.iter().map(|rep| pct(&rep[k], 95.0)).collect()).collect();
    let mut boot = SplitMix64::new(99);
    let kernels: Vec<J> = (0..NAMES.len())
        .map(|k| {
            let ratios: Vec<f64> = (0..reps).map(|r| p50s[k][r] / p50s[0][r].min(p50s[1][r])).collect();
            let exact = k < 2;
            let (lo, hi) = if exact { (pct(&ratios, 2.5), pct(&ratios, 97.5)) } else { bootstrap_ci(&ratios, &mut boot) };
            let med = pct(&ratios, 50.0);
            let v = if exact { "exact scan (baseline is the faster of the two)" } else { verdict(lo, hi) };
            let rc = recall(k, ep, &snap, &qs, &calib);
            let (mn, mx) = (
                p50s[k].iter().cloned().fold(f64::INFINITY, f64::min),
                p50s[k].iter().cloned().fold(0.0, f64::max),
            );
            println!(
                "{:22} p50 {:.3} ms (min {mn:.3}, max {mx:.3})  ratio to faster exact {med:.3}  95% [{lo:.3}, {hi:.3}]  recall {rc:.4}  {v}",
                NAMES[k],
                pct(&p50s[k], 50.0),
            );
            obj(vec![
                ("name", J::Str(NAMES[k].into())),
                ("p50_ms_median", num(pct(&p50s[k], 50.0))),
                ("p50_ms_min", num(mn)),
                ("p50_ms_max", num(mx)),
                ("p95_ms_median", num(pct(&p95s[k], 50.0))),
                ("ratio_vs_faster_exact_median", num(med)),
                ("recall_at_k_first_queries", num(rc)),
                ("ratio_ci95_lo", num(lo)),
                ("ratio_ci95_hi", num(hi)),
                ("verdict", J::Str(v.into())),
            ])
        })
        .collect();
    obj(vec![
        ("n", num(n as f64)),
        ("reps", num(reps as f64)),
        ("queries", num(nq as f64)),
        ("dataset_fnv1a64", J::Str(format!("{digest:016x}"))),
        ("kernels", J::Arr(kernels)),
    ])
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let ns: Vec<usize> = arg(&args, "--n")
        .unwrap_or_else(|| "20000,100000,400000".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let reps: usize = arg(&args, "--reps").and_then(|s| s.parse().ok()).unwrap_or(20);
    let nq: usize = arg(&args, "--queries").and_then(|s| s.parse().ok()).unwrap_or(200);
    let sizes: Vec<J> = ns.iter().map(|&n| measure(n, reps, nq)).collect();
    let rustc = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
    let src = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    let json = obj(vec![
        ("synthetic", J::Bool(true)),
        ("tool", J::Str("prism-timing".into())),
        ("command", J::Str(args.join(" "))),
        ("rustc", J::Str(rustc)),
        ("cpu_note", J::Str(std::env::var("PRISM_CPU").unwrap_or_else(|_| "unknown".into()))),
        ("threads_available", num(threads as f64)),
        ("src_fnv1a64_lf", J::Str(format!("{:016x}", src_digest_lf(src)))),
        ("params", obj(vec![("d", num(D as f64)), ("k", num(K as f64)), ("seed", num(7.0)), ("warmup", num(WARMUP as f64)), ("bootstrap", num(BOOTSTRAP as f64))])),
        ("note", J::Str("single thread; kernels timed in a rotating order per query; ratio = kernel p50 / faster of the two exact-scan p50s from the same repetition; recall is against the exact scan on the first 50 queries; interval = bootstrap over repetitions".into())),
        ("sizes", J::Arr(sizes)),
    ]);
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/results");
    std::fs::create_dir_all(dir).expect("mkdir results");
    let path = format!("{dir}/timing_interleaved.json");
    std::fs::write(&path, json.render() + "\n").expect("write results");
    println!("wrote {path}");
}
