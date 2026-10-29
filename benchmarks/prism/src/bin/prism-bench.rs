
use prism::quant;
use prism::rng::SplitMix64;
use prism::search::{
    calibrate, exact_scores, pv_const, residuals, score_all, search_cascade, search_cascade_lut, search_exact,
    search_exact_fast, search_flat_fixed, search_flat_lut, theory_sigma, Calib, Gate, Hit, Stats,
};
use prism::store::{Epoch, Snapshot, Store};
use prism::util::{centres, fnv1a, isotropic, mean, mixture, obj, par_map, pct, J};
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

fn workload(data: &[Vec<f32>], qs: &Arc<Vec<Vec<f32>>>, calib: &Calib, writer_on: bool, secs: u64, lut: bool) -> (f64, f64, Vec<f64>) {
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
                    while t0.elapsed() < Duration::from_secs(secs) {
                        let q = &qs[i % qs.len()];
                        i += 1;
                        let start = Instant::now();
                        let snap = store.pin();
                        if lut {
                            std::hint::black_box(search_cascade_lut(store.epoch(), &snap, q, K, &SCHED, Gate::Global(2.0), calib));
                        } else {
                            std::hint::black_box(search_cascade(store.epoch(), &snap, q, K, &SCHED, 2.0, calib));
                        }
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

fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read dir").filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn checksum() -> u64 {
    let mut names = Vec::new();
    rs_files(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")), &mut names);
    names.sort();
    let mut all = Vec::new();
    for n in names {
        all.extend(std::fs::read(n).expect("read file"));
    }
    fnv1a(&all)
}

const DELTA: f32 = 1e-3;
const BS3: [usize; 3] = [1, 2, 4];
const TIME_Q: usize = 200;
const NB: usize = 8000;
const BIN: f32 = 0.002;
const TRIALS: usize = 7;
const TRIAL_SECS: u64 = 3;
const SWEEP_ZS: [f32; 5] = [1.5, 2.0, 2.5, 3.0, 4.0];

type Runner<'a> = Box<dyn Fn(&[f32]) -> usize + 'a>;

fn jstr(s: &str) -> J {
    J::Str(s.to_string())
}

fn time_runs(runs: &[(&str, Runner)], qs: &[Vec<f32>]) -> J {
    println!("== X1. kernel timing, single thread, {} queries after 20 warm-up", qs.len());
    let mut out = Vec::new();
    for (name, f) in runs {
        qs.iter().take(20).for_each(|q| {
            std::hint::black_box(f(q));
        });
        let lat: Vec<f64> = qs
            .iter()
            .map(|q| {
                let t = Instant::now();
                std::hint::black_box(f(q));
                ms_since(t)
            })
            .collect();
        println!("{name:22} p50 {:.3} ms  p95 {:.3} ms", pct(&lat, 50.0), pct(&lat, 95.0));
        out.push(obj(vec![("name", jstr(name)), ("p50_ms", num(pct(&lat, 50.0))), ("p95_ms", num(pct(&lat, 95.0)))]));
    }
    J::Arr(out)
}

type Row = Vec<(&'static str, J)>;

fn agg(res: &[(f64, Stats)]) -> (Row, String) {
    let col = |f: &dyn Fn(&Stats) -> f64| res.iter().map(|r| f(&r.1)).collect::<Vec<_>>();
    let rec: Vec<f64> = res.iter().map(|r| r.0).collect();
    let inc = col(&|s| s.bits_incr as f64);
    let rr = col(&|s| s.reranks as f64);
    let nst = res[0].1.survivors.len();
    let surv: Vec<f64> = (0..nst).map(|j| mean(&col(&|s| s.survivors[j] as f64))).collect();
    let miss = rec.iter().filter(|&&r| r < 1.0).count() as f64 / res.len() as f64;
    let line = format!(
        "recall {:.4} miss {miss:.3} bitsInc {:.2}M surv {:?} rr {:.1}/{:.0}",
        mean(&rec),
        mean(&inc) / 1e6,
        surv.iter().map(|x| x.round() as u64).collect::<Vec<_>>(),
        mean(&rr),
        pct(&rr, 95.0)
    );
    let items = vec![
        ("recall", num(mean(&rec))),
        ("miss_rate", num(miss)),
        ("bits_incr_mean", num(mean(&inc))),
        ("bits_incr_p95", num(pct(&inc, 95.0))),
        ("survivors_mean_per_stage", nums(&surv)),
        ("reranks_mean", num(mean(&rr))),
        ("reranks_p95", num(pct(&rr, 95.0))),
    ];
    (items, line)
}

fn run_gate(ep: &Epoch, snap: &Snapshot, qs: &[Vec<f32>], truth: &[Vec<Hit>], calib: &Calib, gate: Gate) -> (Row, String) {
    let res: Vec<(f64, Stats)> = par_map(qs.len(), |i| {
        let (h, s) = search_cascade_lut(ep, snap, &qs[i], K, &SCHED, gate, calib);
        (recall(&truth[i], &h), s)
    });
    agg(&res)
}

struct Acc {
    pw: [[f64; 4]; 3],
    hist: [Vec<u32>; 3],
    max: [f32; 3],
    viol: [u64; 3],
    viol_det: [u64; 3],
    hsum: [f64; 3],
    pairs: u64,
}

impl Acc {
    fn new() -> Self {
        Self {
            pw: [[0.0; 4]; 3],
            hist: [vec![0; NB + 1], vec![0; NB + 1], vec![0; NB + 1]],
            max: [0.0; 3],
            viol: [0; 3],
            viol_det: [0; 3],
            hsum: [0.0; 3],
            pairs: 0,
        }
    }

    fn add(&mut self, o: &Acc) {
        for b in 0..3 {
            for (x, y) in self.pw[b].iter_mut().zip(&o.pw[b]) {
                *x += y;
            }
            for (x, y) in self.hist[b].iter_mut().zip(&o.hist[b]) {
                *x += y;
            }
            self.max[b] = self.max[b].max(o.max[b]);
            self.viol[b] += o.viol[b];
            self.viol_det[b] += o.viol_det[b];
            self.hsum[b] += o.hsum[b];
        }
        self.pairs += o.pairs;
    }

    fn quantile(&self, b: usize, q: f64) -> f64 {
        let target = (q * self.pairs as f64).ceil() as u64;
        let mut c = 0u64;
        for (i, &n) in self.hist[b].iter().enumerate() {
            c += n as u64;
            if c >= target {
                return (i + 1) as f64 * BIN as f64;
            }
        }
        f64::INFINITY
    }

    fn kurtosis(&self, b: usize) -> f64 {
        let n = self.pairs as f64;
        let (m1, p2, p3, p4) = (self.pw[b][0] / n, self.pw[b][1] / n, self.pw[b][2] / n, self.pw[b][3] / n);
        let m2 = p2 - m1 * m1;
        let m4 = p4 - 4.0 * m1 * p3 + 6.0 * m1 * m1 * p2 - 3.0 * m1.powi(4);
        m4 / (m2 * m2) - 3.0
    }
}

fn pair_analysis(ep: &Epoch, snap: &Snapshot, qs: &[Vec<f32>], calib: &Calib) -> Acc {
    let resid: Vec<Vec<f32>> = BS3.iter().map(|&b| residuals(snap, b)).collect();
    let c = pv_const(ep.p, DELTA);
    let chunks = 48;
    let parts = par_map(chunks, |ci| {
        let mut a = Acc::new();
        for q in qs.iter().skip(ci).step_by(chunks) {
            let sc = score_all(ep, snap, q, &BS3);
            let ex = exact_scores(snap, q);
            a.pairs += ex.len() as u64;
            for (bi, &b) in BS3.iter().enumerate() {
                let sg = calib.sigma[b];
                for ((s, x), r) in sc[bi].iter().zip(&ex).zip(&resid[bi]) {
                    let e = s - x;
                    let ed = e as f64;
                    a.pw[bi][0] += ed;
                    a.pw[bi][1] += ed * ed;
                    a.pw[bi][2] += ed * ed * ed;
                    a.pw[bi][3] += ed * ed * ed * ed;
                    let z = e.abs() / sg;
                    a.hist[bi][((z / BIN) as usize).min(NB)] += 1;
                    a.max[bi] = a.max[bi].max(z);
                    a.hsum[bi] += (r * c) as f64;
                    a.viol[bi] += (e.abs() > r * c) as u64;
                    a.viol_det[bi] += (e.abs() > r * 1.0001 + 1e-6) as u64;
                }
            }
        }
        a
    });
    let mut tot = Acc::new();
    parts.iter().for_each(|p| tot.add(p));
    tot
}

fn errdist_json(a: &Acc, calib: &Calib) -> J {
    J::Arr(
        BS3.iter()
            .enumerate()
            .map(|(bi, &b)| {
                obj(vec![
                    ("bits", num(b as f64)),
                    ("pairs", num(a.pairs as f64)),
                    ("sigma_used", num(calib.sigma[b] as f64)),
                    ("mean_err_over_sigma", num(a.pw[bi][0] / a.pairs as f64 / calib.sigma[b] as f64)),
                    ("excess_kurtosis", num(a.kurtosis(bi))),
                    ("q99_9_abs_over_sigma", num(a.quantile(bi, 0.999))),
                    ("q99_99_abs_over_sigma", num(a.quantile(bi, 0.9999))),
                    ("max_abs_over_sigma", num(a.max[bi] as f64)),
                    ("gaussian_q99_9", num(3.29)),
                    ("gaussian_q99_99", num(3.89)),
                ])
            })
            .collect(),
    )
}

fn timed_p50(ep: &Epoch, snap: &Snapshot, qs: &[Vec<f32>], z: f32, calib: &Calib) -> (f64, usize) {
    let tq = TIME_Q.min(qs.len());
    let run = |q: &Vec<f32>| {
        std::hint::black_box(search_cascade_lut(ep, snap, q, K, &SCHED, Gate::Global(z), calib));
    };
    qs.iter().take(5).for_each(run);
    let lat: Vec<f64> = qs[..tq]
        .iter()
        .map(|q| {
            let t = Instant::now();
            run(q);
            ms_since(t)
        })
        .collect();
    (pct(&lat, 50.0), tq)
}

fn gates_json(acc: &Acc, calib: &Calib, rows: Vec<J>) -> J {
    let viol = BS3.iter().enumerate().map(|(bi, &b)| {
        let n = acc.pairs as f64;
        obj(vec![
            ("bits", num(b as f64)),
            ("per_vector_violations", num(acc.viol[bi] as f64)),
            ("pairs", num(n)),
            ("per_vector_violation_rate", num(acc.viol[bi] as f64 / n)),
            ("mean_half_width_h", num(acc.hsum[bi] / n)),
            ("mean_half_width_over_sigma", num(acc.hsum[bi] / n / calib.sigma[b] as f64)),
            ("deterministic_violations", num(acc.viol_det[bi] as f64)),
        ])
    });
    obj(vec![("delta", num(DELTA as f64)), ("per_vector_bound", J::Arr(viol.collect())), ("rows", J::Arr(rows))])
}

fn run_ds(name: &str, data: &[Vec<f32>], qs: &[Vec<f32>], pairs: bool) -> (J, Option<J>, Option<J>) {
    let store = build_store(data);
    let (ep, snap) = (store.epoch(), store.pin());
    let nq = qs.len();
    let truth: Vec<Vec<Hit>> = par_map(nq, |i| search_exact(&snap, &qs[i], K));
    let calib = calibrate(ep, &snap, &qs[..100.min(nq)], 1000);
    println!("== X2 {name}: n={} nq={nq} sigma_b {:?}", data.len(), &calib.sigma[1..]);
    let mut rows = Vec::new();
    for z in SWEEP_ZS {
        let (mut items, line) = run_gate(ep, &snap, qs, &truth, &calib, Gate::Global(z));
        let (t50, tq) = timed_p50(ep, &snap, qs, z, &calib);
        println!("z={z:.1} {line} t50 {t50:.2} ms (first {tq} q)");
        items.insert(0, ("z", num(z as f64)));
        items.push(("time_p50_ms", num(t50)));
        items.push(("time_queries", num(tq as f64)));
        rows.push(obj(items));
    }
    let sweep = obj(vec![
        ("n", num(data.len() as f64)),
        ("nq", num(nq as f64)),
        ("full_4bit_scan_bits", num((data.len() * ep.p * 4) as f64)),
        ("sigma_b", nums(&calib.sigma[1..].iter().map(|&x| x as f64).collect::<Vec<_>>())),
        ("cascade_lut", J::Arr(rows)),
    ]);
    if !pairs {
        return (sweep, None, None);
    }
    let acc = pair_analysis(ep, &snap, qs, &calib);
    for (bi, &b) in BS3.iter().enumerate() {
        let n = acc.pairs as f64;
        println!(
            "  err b={b}: kurt {:.2} q99.9 {:.2} q99.99 {:.2} max {:.2} sigma | per-vector bound violated {} of {} ({:.2e}), mean h {:.4}, deterministic violated {}",
            acc.kurtosis(bi), acc.quantile(bi, 0.999), acc.quantile(bi, 0.9999), acc.max[bi],
            acc.viol[bi], acc.pairs, acc.viol[bi] as f64 / n, acc.hsum[bi] / n, acc.viol_det[bi]
        );
    }
    let gates = [
        ("global_z3", Gate::Global(3.0)),
        ("global_z4", Gate::Global(4.0)),
        ("per_vector_delta1e-3", Gate::PerVector(DELTA)),
        ("deterministic", Gate::Deterministic),
    ];
    let mut grows = Vec::new();
    for (gname, g) in gates {
        let (mut items, line) = run_gate(ep, &snap, qs, &truth, &calib, g);
        println!("  gate {gname:22} {line}");
        items.insert(0, ("gate", jstr(gname)));
        grows.push(obj(items));
    }
    (sweep, Some(errdist_json(&acc, &calib)), Some(gates_json(&acc, &calib, grows)))
}

fn med(v: &[f64]) -> f64 {
    pct(v, 50.0)
}

fn concurrency_repeats(data: &[Vec<f32>], qs: &Arc<Vec<Vec<f32>>>, calib: &Calib) -> J {
    println!("== X5. concurrency repeats: {TRIALS} x (OFF,ON), {TRIAL_SECS}s each, 4 readers, LUT cascade z=2");
    let (mut ratios, mut qoff, mut qon, mut trials) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for t in 0..TRIALS {
        let (_, _, off) = workload(data, qs, calib, false, TRIAL_SECS, true);
        let (wps, _, on) = workload(data, qs, calib, true, TRIAL_SECS, true);
        let (p_off, p_on) = (pct(&off, 99.0), pct(&on, 99.0));
        let (q_off, q_on) = (off.len() as f64 / TRIAL_SECS as f64, on.len() as f64 / TRIAL_SECS as f64);
        println!("trial {t}: p99 OFF {p_off:.3} ON {p_on:.3} ratio {:.2} | qps OFF {q_off:.0} ON {q_on:.0} | writes {wps:.0} vec/s", p_on / p_off);
        ratios.push(p_on / p_off);
        qoff.push(q_off);
        qon.push(q_on);
        trials.push(obj(vec![
            ("p99_off_ms", num(p_off)),
            ("p99_on_ms", num(p_on)),
            ("p99_ratio", num(p_on / p_off)),
            ("p50_off_ms", num(pct(&off, 50.0))),
            ("p50_on_ms", num(pct(&on, 50.0))),
            ("qps_off", num(q_off)),
            ("qps_on", num(q_on)),
            ("writes_vectors_per_s", num(wps)),
        ]));
    }
    let (lo, hi) = (ratios.iter().cloned().fold(f64::MAX, f64::min), ratios.iter().cloned().fold(0.0, f64::max));
    println!("median p99 ratio {:.2} (min {lo:.2}, max {hi:.2}); median qps OFF {:.0} ON {:.0}", med(&ratios), med(&qoff), med(&qon));
    obj(vec![
        ("trials", J::Arr(trials)),
        ("median_p99_ratio", num(med(&ratios))),
        ("min_p99_ratio", num(lo)),
        ("max_p99_ratio", num(hi)),
        ("median_qps_off", num(med(&qoff))),
        ("median_qps_on", num(med(&qon))),
        ("readers", num(4.0)),
        ("secs_per_condition", num(TRIAL_SECS as f64)),
    ])
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

    println!("== 2. flat fixed rerank_n=100");
    let mut e2 = Vec::new();
    for b in [1usize, 2, 4] {
        let r = par_map(NQ, |i| recall(&truth[i], &search_flat_fixed(ep, &snap, &qs[i], K, b, 100).0));
        println!("bits={b} recall@10 {:.4}", mean(&r));
        e2.push(obj(vec![("bits", num(b as f64)), ("recall", num(mean(&r)))]));
    }

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

    println!("== 4. wall-clock per query, single thread (machine not idle)");
    let mut e4 = Vec::new();
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

    println!("== 5. concurrency ({WORKLOAD_SECS}s, 4 readers, cascade z=2)");
    let (_, _, off) = workload(&data, &qs, &calib, false, WORKLOAD_SECS, false);
    let (wps, cps, on) = workload(&data, &qs, &calib, true, WORKLOAD_SECS, false);
    let jitter = pct(&on, 99.0) / pct(&off, 99.0);
    println!(
        "writer OFF p50/95/99 {:.2}/{:.2}/{:.2} ms | ON {:.2}/{:.2}/{:.2} ms | writes {wps:.0} vec/s ({cps:.0} commits/s) | jitter p99 ratio {jitter:.2}",
        pct(&off, 50.0), pct(&off, 95.0), pct(&off, 99.0), pct(&on, 50.0), pct(&on, 95.0), pct(&on, 99.0)
    );

    let runs: Vec<(&str, Runner)> = vec![
        ("exact_scan_ref", Box::new(|q| search_exact(&snap, q, K).len())),
        ("exact_scan_opt", Box::new(|q| search_exact_fast(&snap, q, K).len())),
        ("flat4_lut_rerank100", Box::new(|q| search_flat_lut(ep, &snap, q, K, 4, 100).0.len())),
        ("cascade_lut_z2", Box::new(|q| search_cascade_lut(ep, &snap, q, K, &SCHED, Gate::Global(2.0), &calib).0.len())),
        ("cascade_lut_z3", Box::new(|q| search_cascade_lut(ep, &snap, q, K, &SCHED, Gate::Global(3.0), &calib).0.len())),
        ("cascade_scalar_z2", Box::new(|q| search_cascade(ep, &snap, q, K, &SCHED, 2.0, &calib).0.len())),
    ];
    let kernel = time_runs(&runs, &qs);
    drop(runs);
    let (mut ds_rows, mut ed, mut gt) = (Vec::new(), Vec::new(), Vec::new());
    let mut record = |name: &str, r: (J, Option<J>, Option<J>)| {
        ds_rows.push((name.to_string(), r.0));
        if let (Some(e), Some(g)) = (r.1, r.2) {
            ed.push((name.to_string(), e));
            gt.push((name.to_string(), g));
        }
    };
    record("a_clustered_in_distribution", run_ds("a_clustered_in_distribution", &data, &qs, true));
    let mut r2 = SplitMix64::new(101);
    let cq = centres(&mut r2, 50, D);
    let qs_ood = mixture(&mut r2, &cq, NQ, 0.6, 0.0);
    record("b_clustered_ood_queries", run_ds("b_clustered_ood_queries", &data, &qs_ood, true));
    let mut r3 = SplitMix64::new(102);
    let (iso, iso_q) = (isotropic(&mut r3, N, D), isotropic(&mut r3, NQ, D));
    record("c_isotropic_gaussian", run_ds("c_isotropic_gaussian", &iso, &iso_q, true));
    drop((iso, iso_q));
    let mut r4 = SplitMix64::new(103);
    let (ov, ov_q) = (mixture(&mut r4, &cs, N, 1.5, 0.0), mixture(&mut r4, &cs, NQ, 1.5, 0.0));
    record("d_overlapping_clusters_noise1.5", run_ds("d_overlapping_clusters_noise1.5", &ov, &ov_q, true));
    drop((ov, ov_q));
    let mut r5 = SplitMix64::new(104);
    let (big, big_q) = (mixture(&mut r5, &cs, 100_000, 0.6, 0.0), mixture(&mut r5, &cs, 200, 0.6, 0.0));
    record("e_clustered_n100000_nq200", run_ds("e_clustered_n100000_nq200", &big, &big_q, false));
    drop((big, big_q));
    let conc = concurrency_repeats(&data, &qs, &calib);
    let extra = obj(vec![
        ("note", jstr("kernel_timing: single thread, 1000 queries, 20 warm-up; datasets: time_p50 from first 200 queries (single thread), other metrics from all queries; e) uses nq=200; error_dist/gates cover all pairs of datasets a-d; per-vector h = r*sqrt(2 ln(2/delta)/P) with P the padded dim")),
        ("kernel_timing", kernel),
        ("datasets", J::Obj(ds_rows)),
        ("error_dist", J::Obj(ed)),
        ("gates", J::Obj(gt)),
        ("concurrency_repeats", conc),
    ]);

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
        ("src_fnv1a64_lf", J::Str(format!("{:016x}", prism::util::src_digest_lf(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")))))),
        ("dataset_fnv1a64", obj(vec![("corpus_and_queries_seed7", J::Str(format!("{:016x}", prism::util::dataset_digest(&[&data[..], &qs[..]]))))])),
        ("command", J::Str(std::env::args().collect::<Vec<_>>().join(" "))),
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
        ("extra", extra),
    ]);
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/results/results.json");
    std::fs::create_dir_all(concat!(env!("CARGO_MANIFEST_DIR"), "/results")).expect("mkdir results");
    std::fs::write(path, json.render() + "\n").expect("write results");
    println!("wrote {path}");
}
