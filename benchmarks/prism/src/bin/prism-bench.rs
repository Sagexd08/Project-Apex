//! Prism experiments. Synthetic data only; writes results/results.json.

use prism::quant;
use prism::rng::SplitMix64;
use prism::search::{calibrate, search_cascade, search_exact, search_flat_fixed, theory_sigma, Calib, Hit, Stats};
use prism::store::{Epoch, Store};
use prism::util::{centres, fnv1a, mean, mixture, obj, par_map, pct, J};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const N: usize = 20_000;
const D: usize = 256;
const NQ: usize = 1000;
const K: usize = 10;
const SCHED: [usize; 3] = [1, 2, 4];
const ZS: [f32; 6] = [1.0, 1.5, 2.0, 2.5, 3.0, 4.0];
const WORKLOAD_SECS: u64 = 5;
const WRITE_BATCH: usize = 20;

fn num(x: f64) -> J {
    J::Num(x)
}

fn nums(v: &[f64]) -> J {
    J::Arr(v.iter().map(|&x| J::Num(x)).collect())
}

fn recall(truth: &[Hit], got: &[Hit]) -> f64 {
    got.iter().filter(|g| truth.iter().any(|t| t.0 == g.0)).count() as f64 / truth.len() as f64
}

fn build_store(data: &[Vec<f32>]) -> Store {
    let store = Store::new(Epoch::new(0xA71A5, D, SCHED.to_vec()));
    for (c, chunk) in data.chunks(2000).enumerate() {
        store.commit(chunk.iter().enumerate().map(|(i, v)| ((c * 2000 + i) as u64, v.clone())).collect());
    }
    store.compact(usize::MAX);
    store
}

fn ms_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn mse(ep: &Epoch, vs: &[Vec<f32>], b: usize) -> f64 {
    let s = ep.half_range();
    let errs: Vec<f64> = vs
        .iter()
        .map(|v| {
            let z = ep.rotate(v);
            z.iter().map(|&x| ((x - quant::recon(quant::idx4(x, s), b, s)) as f64).powi(2)).sum()
        })
        .collect();
    mean(&errs)
}

/// Returns (writes_vec_per_s, commits_per_s, reader latencies ms).
fn workload(data: &[Vec<f32>], qs: &Arc<Vec<Vec<f32>>>, calib: &Calib, writer_on: bool) -> (f64, f64, Vec<f64>) {
    let store = build_store(data);
    let pool = &data[..2000];
    let (stop, commits) = (AtomicBool::new(false), AtomicU64::new(0));
    let t0 = Instant::now();
    let lats: Vec<f64> = std::thread::scope(|sc| {
        if writer_on {
            sc.spawn(|| {
                let mut next = 1_000_000u64;
                while !stop.load(Ordering::Relaxed) {
                    let b = (0..WRITE_BATCH)
                        .map(|i| (next + i as u64, pool[(next as usize + i) % pool.len()].clone()))
                        .collect();
                    next += WRITE_BATCH as u64;
                    let n = commits.fetch_add(1, Ordering::Relaxed) + 1;
                    store.commit(b);
                    if n % 32 == 0 {
                        store.compact(1000);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
        }
        let readers: Vec<_> = (0..4usize)
            .map(|t| {
                let store = &store;
                sc.spawn(move || {
                    let mut out = Vec::new();
                    let mut i = t * 7;
                    while t0.elapsed() < Duration::from_secs(WORKLOAD_SECS) {
                        let q = &qs[i % qs.len()];
                        i += 1;
                        let start = Instant::now();
                        let snap = store.pin();
                        let r = search_cascade(store.epoch(), &snap, q, K, &SCHED, 2.0, calib);
                        std::hint::black_box(&r);
                        out.push(ms_since(start));
                    }
                    out
                })
            })
            .collect();
        let all: Vec<f64> = readers.into_iter().flat_map(|h| h.join().expect("reader")).collect();
        stop.store(true, Ordering::Relaxed);
        all
    });
    let secs = t0.elapsed().as_secs_f64();
    let c = commits.load(Ordering::Relaxed) as f64;
    (c * WRITE_BATCH as f64 / secs, c / secs, lats)
}

fn lat_json(l: &[f64]) -> J {
    obj(vec![
        ("queries", num(l.len() as f64)),
        ("p50_ms", num(pct(l, 50.0))),
        ("p95_ms", num(pct(l, 95.0))),
        ("p99_ms", num(pct(l, 99.0))),
    ])
}

fn checksum() -> u64 {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .expect("read src")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    names.sort();
    let mut all = Vec::new();
    for n in names {
        all.extend(std::fs::read(n).expect("read file"));
    }
    fnv1a(&all)
}

fn main() {
    let mut rng = SplitMix64::new(7);
    let cs = centres(&mut rng, 50, D);
    let data = mixture(&mut rng, &cs, N, 0.6, 0.0);
    let qs = Arc::new(mixture(&mut rng, &cs, NQ, 0.6, 0.0));
    let shifted = mixture(&mut rng, &cs, 1000, 0.6, 2.0);
    let store = build_store(&data);
    let (ep, snap) = (store.epoch(), store.pin());
    let truth: Vec<Vec<Hit>> = par_map(NQ, |i| search_exact(&snap, &qs[i], K));

    // 1. sigma and data-oblivious MSE
    let calib = calibrate(ep, &snap, &qs[..100], 1000);
    let mut e1 = Vec::new();
    println!("== 1. sigma_b (measured vs C/(sqrt(3P)2^b)) and MSE (orig vs shifted)");
    for b in [1usize, 2, 4] {
        let (th, m) = (theory_sigma(ep.p, b), calib.sigma[b] as f64);
        let (mo, ms) = (mse(ep, &data[..1000], b), mse(ep, &shifted, b));
        println!("b={b} sigma {m:.5} theory {th:.5} ratio {:.3} | mse orig {mo:.5} shifted {ms:.5}", m / th);
        e1.push(obj(vec![
            ("bits", num(b as f64)),
            ("sigma_measured", num(m)),
            ("sigma_theory", num(th)),
            ("mse_original", num(mo)),
            ("mse_shifted", num(ms)),
            ("mse_theory", num(ep.p as f64 * (ep.half_range() as f64).powi(2) / (3.0 * 4f64.powi(b as i32)))),
        ]));
    }

    // 2. flat fixed rerank baselines
    println!("== 2. flat fixed rerank_n=100");
    let mut e2 = Vec::new();
    for b in [1usize, 2, 4] {
        let r = par_map(NQ, |i| recall(&truth[i], &search_flat_fixed(ep, &snap, &qs[i], K, b, 100).0));
        println!("bits={b} recall@10 {:.4}", mean(&r));
        e2.push(obj(vec![("bits", num(b as f64)), ("recall", num(mean(&r)))]));
    }

    // 3. cascade z sweep
    println!("== 3. cascade [1,2,4]\n z    recall  miss   bitsPfx(M) bitsInc(M) surv1   surv2   surv3  rerank(mean/p95)");
    let mut e3 = Vec::new();
    for z in ZS {
        let res: Vec<(f64, Stats)> = par_map(NQ, |i| {
            let (h, s) = search_cascade(ep, &snap, &qs[i], K, &SCHED, z, &calib);
            (recall(&truth[i], &h), s)
        });
        let rec: Vec<f64> = res.iter().map(|r| r.0).collect();
        let col = |f: &dyn Fn(&Stats) -> f64| res.iter().map(|r| f(&r.1)).collect::<Vec<_>>();
        let pfx = col(&|s| s.bits_prefix as f64);
        let inc = col(&|s| s.bits_incr as f64);
        let rr = col(&|s| s.reranks as f64);
        let surv: Vec<f64> = (0..3).map(|j| mean(&col(&|s| s.survivors[j] as f64))).collect();
        let miss = rec.iter().filter(|&&r| r < 1.0).count() as f64 / NQ as f64;
        println!(
            "{z:.1}  {:.4}  {miss:.3}  {:.1}/{:.1}  {:.1}/{:.1}  {:.0} {:.0} {:.0}  {:.1}/{:.0}",
            mean(&rec), mean(&pfx) / 1e6, pct(&pfx, 95.0) / 1e6, mean(&inc) / 1e6, pct(&inc, 95.0) / 1e6,
            surv[0], surv[1], surv[2], mean(&rr), pct(&rr, 95.0)
        );
        e3.push(obj(vec![
            ("z", num(z as f64)),
            ("recall", num(mean(&rec))),
            ("miss_rate", num(miss)),
            ("bits_prefix_mean", num(mean(&pfx))),
            ("bits_prefix_p95", num(pct(&pfx, 95.0))),
            ("bits_incr_mean", num(mean(&inc))),
            ("bits_incr_p95", num(pct(&inc, 95.0))),
            ("survivors_mean_per_stage", nums(&surv)),
            ("reranks_mean", num(mean(&rr))),
            ("reranks_p95", num(pct(&rr, 95.0))),
        ]));
    }
    println!("(full 4-bit scan = {:.1}M bits/query)", (N * D * 4) as f64 / 1e6);

    // 4. wall clock, single thread, sequential
    println!("== 4. wall-clock per query, single thread (machine not idle)");
    let mut e4 = Vec::new();
    type Runner<'a> = Box<dyn Fn(&[f32]) -> usize + 'a>;
    let runs: Vec<(&str, Runner)> = vec![
        ("exact_scan", Box::new(|q| search_exact(&snap, q, K).len())),
        ("flat4_rerank100", Box::new(|q| search_flat_fixed(ep, &snap, q, K, 4, 100).0.len())),
        ("cascade_z2", Box::new(|q| search_cascade(ep, &snap, q, K, &SCHED, 2.0, &calib).0.len())),
        ("cascade_z3", Box::new(|q| search_cascade(ep, &snap, q, K, &SCHED, 3.0, &calib).0.len())),
    ];
    for (name, f) in &runs {
        let lat: Vec<f64> = qs
            .iter()
            .map(|q| {
                let t = Instant::now();
                std::hint::black_box(f(q));
                ms_since(t)
            })
            .collect();
        println!("{name:16} p50 {:.3} ms  p95 {:.3} ms", pct(&lat, 50.0), pct(&lat, 95.0));
        e4.push(obj(vec![("name", J::Str(name.to_string())), ("p50_ms", num(pct(&lat, 50.0))), ("p95_ms", num(pct(&lat, 95.0)))]));
    }

    // 5. concurrency
    println!("== 5. concurrency ({WORKLOAD_SECS}s, 4 readers, cascade z=2)");
    let (_, _, off) = workload(&data, &qs, &calib, false);
    let (wps, cps, on) = workload(&data, &qs, &calib, true);
    let jitter = pct(&on, 99.0) / pct(&off, 99.0);
    println!(
        "writer OFF p50/95/99 {:.2}/{:.2}/{:.2} ms | ON {:.2}/{:.2}/{:.2} ms | writes {wps:.0} vec/s ({cps:.0} commits/s) | jitter p99 ratio {jitter:.2}",
        pct(&off, 50.0), pct(&off, 95.0), pct(&off, 99.0), pct(&on, 50.0), pct(&on, 95.0), pct(&on, 99.0)
    );

    let rustc = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    let json = obj(vec![
        ("synthetic", J::Bool(true)),
        ("rustc", J::Str(rustc)),
        ("cpu_note", J::Str(std::env::var("PRISM_CPU").unwrap_or_else(|_| "unknown".into()))),
        ("src_fnv1a64", J::Str(format!("{:016x}", checksum()))),
        ("note", J::Str("measured on a non-idle machine; writer throttled to 1 batch/10ms so data grows ~+50% during the ON run".into())),
        ("params", obj(vec![("n", num(N as f64)), ("d", num(D as f64)), ("nq", num(NQ as f64)), ("k", num(K as f64)), ("seed", num(7.0))])),
        ("sigma_mse", J::Arr(e1)),
        ("flat_fixed_rerank100", J::Arr(e2)),
        ("cascade", J::Arr(e3)),
        ("wall_clock", J::Arr(e4)),
        ("concurrency", obj(vec![
            ("writer_off", lat_json(&off)),
            ("writer_on", lat_json(&on)),
            ("writes_vectors_per_s", num(wps)),
            ("commits_per_s", num(cps)),
            ("jitter_ratio_p99", num(jitter)),
        ])),
    ]);
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/results/results.json");
    std::fs::create_dir_all(concat!(env!("CARGO_MANIFEST_DIR"), "/results")).expect("mkdir results");
    std::fs::write(path, json.render() + "\n").expect("write results");
    println!("wrote {path}");
}
