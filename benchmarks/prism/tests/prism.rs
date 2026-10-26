use prism::quant;
use prism::rng::SplitMix64;
use prism::rot::Rotation;
use prism::search::{
    calibrate, residuals, score_all, score_all_ref, search_cascade, search_cascade_lut, search_exact,
    search_exact_fast, theory_sigma, Calib, Gate,
};
use prism::store::{Epoch, Store};
use prism::util::{centres, mixture};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn unit(rng: &mut SplitMix64, d: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..d).map(|_| rng.gauss() as f32).collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= n);
    v
}

fn batch(rng: &mut SplitMix64, d: usize, start: u64, n: usize) -> Vec<(u64, Vec<f32>)> {
    (0..n).map(|i| (start + i as u64, unit(rng, d))).collect()
}

#[test]
fn rotation_preserves_norm() {
    let mut rng = SplitMix64::new(1);
    for d in [8usize, 100, 256] {
        let rot = Rotation::new(d, 42);
        for _ in 0..20 {
            let v = unit(&mut rng, d);
            let r = rot.apply(&v);
            let n: f32 = r.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((n - 1.0).abs() < 1e-5, "norm {n}");
        }
    }
}

#[test]
fn nested_prefix_equals_direct_quantisation() {
    let mut rng = SplitMix64::new(2);
    let s = quant::scale(256);
    for _ in 0..20000 {
        let z = (rng.gauss() as f32) * s;
        let i4 = quant::idx4(z, s);
        for b in 1..=4 {
            assert_eq!(i4 >> (4 - b), quant::idx_direct(z, s, b), "z={z} b={b}");
        }
    }
    // planes decode to the same prefix
    let z: Vec<f32> = (0..128).map(|_| rng.gauss() as f32 * s).collect();
    let mut planes: [Vec<u64>; 4] = Default::default();
    quant::push_encoded(&z, s, &mut planes);
    for (i, &zi) in z.iter().enumerate() {
        for b in 1..=4 {
            assert_eq!(quant::decode_idx(&planes, 0, i, b), quant::idx_direct(zi, s, b) as usize);
        }
    }
}

#[test]
fn sigma_matches_formula_within_20_percent() {
    let d = 128;
    let mut rng = SplitMix64::new(3);
    let ep = Epoch::new(5, d, vec![1, 2, 4]);
    let store = Store::new(ep);
    store.commit(batch(&mut rng, d, 0, 3000));
    let qs: Vec<Vec<f32>> = (0..40).map(|_| unit(&mut rng, d)).collect();
    let c = calibrate(store.epoch(), &store.pin(), &qs, 1000);
    for b in [2usize, 4] {
        let th = theory_sigma(store.epoch().p, b);
        let m = c.sigma[b] as f64;
        assert!((m / th - 1.0).abs() < 0.2, "b={b} measured {m} theory {th}");
    }
}

#[test]
fn huge_z_cascade_equals_exact() {
    let d = 64;
    let mut rng = SplitMix64::new(4);
    let store = Store::new(Epoch::new(6, d, vec![1, 2, 4]));
    store.commit(batch(&mut rng, d, 0, 500));
    store.commit(batch(&mut rng, d, 500, 500));
    let snap = store.pin();
    let calib = Calib::theory(store.epoch());
    for _ in 0..10 {
        let q = unit(&mut rng, d);
        let (got, st) = search_cascade(store.epoch(), &snap, &q, 10, &[1, 2, 4], 1e6, &calib);
        assert_eq!(got, search_exact(&snap, &q, 10));
        assert_eq!(st.reranks, 1000);
    }
}

#[test]
fn compaction_does_not_change_results() {
    let d = 64;
    let mut rng = SplitMix64::new(7);
    let store = Store::new(Epoch::new(8, d, vec![1, 2, 4]));
    for i in 0..8 {
        store.commit(batch(&mut rng, d, i * 100, 100));
    }
    let calib = Calib::theory(store.epoch());
    let qs: Vec<Vec<f32>> = (0..20).map(|_| unit(&mut rng, d)).collect();
    let run = |snap: &prism::store::Snapshot| -> Vec<_> {
        qs.iter()
            .map(|q| search_cascade(store.epoch(), snap, q, 10, &[1, 2, 4], 2.0, &calib))
            .map(|(h, s)| (h, s.survivors, s.reranks))
            .collect()
    };
    let before_snap = store.pin();
    let before = run(&before_snap);
    let lsn = store.compact(1000);
    let after_snap = store.pin();
    assert_eq!(lsn, before_snap.lsn + 1);
    assert_eq!(after_snap.segments.len(), 1);
    assert_eq!(after_snap.len(), 800);
    assert_eq!(after_snap.segments[0].lsn_range(), (1, 8));
    assert_eq!(run(&after_snap), before);
    // old pinned snapshot is still intact
    assert_eq!(run(&before_snap), before);
}

#[test]
fn concurrent_writer_and_readers_hold_invariants() {
    const B: usize = 20;
    let d = 64;
    let store = Store::new(Epoch::new(9, d, vec![1, 2, 4]));
    let stop = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_secs(2);
    std::thread::scope(|sc| {
        sc.spawn(|| {
            let mut rng = SplitMix64::new(11);
            let mut i = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let lsn = store.commit(batch(&mut rng, d, i * B as u64, B));
                i += 1;
                assert_eq!(lsn, i); // single writer, no compaction: lsn == commit count
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let readers: Vec<_> = (0..4u64)
            .map(|t| {
                let store = &store;
                sc.spawn(move || {
                    let mut rng = SplitMix64::new(100 + t);
                    let calib = Calib::theory(store.epoch());
                    let (mut last, mut searches) = (0u64, 0u32);
                    while Instant::now() < deadline {
                        let snap = store.pin();
                        assert!(snap.lsn >= last, "I3 violated");
                        last = snap.lsn;
                        // I2: exactly ids 0..lsn*B
                        let mut ids: Vec<u64> =
                            snap.segments.iter().flat_map(|s| s.ids().iter().copied()).collect();
                        ids.sort_unstable();
                        let want: Vec<u64> = (0..snap.lsn * B as u64).collect();
                        assert_eq!(ids, want, "I2 violated at lsn {}", snap.lsn);
                        // I1: repeatable
                        let q = unit(&mut rng, d);
                        let a = search_cascade(store.epoch(), &snap, &q, 10, &[1, 2, 4], 2.0, &calib);
                        let b = search_cascade(store.epoch(), &snap, &q, 10, &[1, 2, 4], 2.0, &calib);
                        assert_eq!(a.0, b.0, "I1 violated");
                        searches += 1;
                    }
                    searches
                })
            })
            .collect();
        let total: u32 = readers.into_iter().map(|h| h.join().expect("reader panicked")).sum();
        stop.store(true, Ordering::Relaxed);
        assert!(total > 0);
    });
    assert!(store.pin().lsn > 0);
}

#[test]
fn mixture_cascade_recall_is_high_at_z3() {
    let (d, n) = (128, 3000);
    let mut rng = SplitMix64::new(13);
    let cs = centres(&mut rng, 20, d);
    let data = mixture(&mut rng, &cs, n, 0.6, 0.0);
    let qs = mixture(&mut rng, &cs, 30, 0.6, 0.0);
    let store = Store::new(Epoch::new(14, d, vec![1, 2, 4]));
    store.commit(data.into_iter().enumerate().map(|(i, v)| (i as u64, v)).collect());
    let snap = store.pin();
    let calib = calibrate(store.epoch(), &snap, &qs, 500);
    let mut hit = 0;
    for q in &qs {
        let truth = search_exact(&snap, q, 10);
        let (got, _) = search_cascade(store.epoch(), &snap, q, 10, &[1, 2, 4], 3.0, &calib);
        hit += got.iter().filter(|g| truth.iter().any(|t| t.0 == g.0)).count();
    }
    assert!(hit as f64 / 300.0 > 0.95, "recall {}", hit as f64 / 300.0);
}

#[test]
fn lut_scores_equal_reference_decode() {
    let d = 256;
    let mut rng = SplitMix64::new(21);
    let store = Store::new(Epoch::new(22, d, vec![1, 2, 4]));
    store.commit(batch(&mut rng, d, 0, 300));
    store.commit(batch(&mut rng, d, 300, 200));
    let (ep, snap) = (store.epoch(), store.pin());
    for _ in 0..5 {
        let q = unit(&mut rng, d);
        let fast = score_all(ep, &snap, &q, &[1, 2, 4]);
        for (bi, b) in [1usize, 2, 4].into_iter().enumerate() {
            let reference = score_all_ref(ep, &snap, &q, b);
            assert_eq!(fast[bi].len(), 500);
            for (f, r) in fast[bi].iter().zip(&reference) {
                assert!((f - r).abs() < 1e-4, "b={b} lut {f} ref {r}");
            }
        }
    }
}

#[test]
fn exact_fast_matches_exact() {
    let d = 100; // not a multiple of 8 in the padded sense of chunks: exercises the tail path
    let mut rng = SplitMix64::new(23);
    let store = Store::new(Epoch::new(24, d, vec![1, 2, 4]));
    store.commit(batch(&mut rng, d, 0, 400));
    let snap = store.pin();
    for _ in 0..10 {
        let q = unit(&mut rng, d);
        let ids = |h: Vec<(u64, f32)>| h.into_iter().map(|x| x.0).collect::<Vec<_>>();
        assert_eq!(ids(search_exact_fast(&snap, &q, 10)), ids(search_exact(&snap, &q, 10)));
    }
}

#[test]
fn stored_residual_matches_recomputation() {
    let d = 128;
    let mut rng = SplitMix64::new(25);
    let store = Store::new(Epoch::new(26, d, vec![1, 2, 4]));
    let b0 = batch(&mut rng, d, 0, 50);
    store.commit(b0.clone());
    store.commit(batch(&mut rng, d, 50, 50));
    store.compact(usize::MAX); // residuals must survive a merge
    let (ep, snap) = (store.epoch(), store.pin());
    for b in 1..=4usize {
        let r = residuals(&snap, b);
        assert_eq!(r.len(), 100);
        for ((_, v), stored) in b0.iter().zip(&r) {
            let want = quant::residual(&ep.rotate(v), ep.half_range(), b);
            assert!((want - stored).abs() < 1e-4, "b={b}");
        }
    }
    let (r1, r4) = (residuals(&snap, 1), residuals(&snap, 4));
    assert!(r1.iter().zip(&r4).all(|(a, b)| a > b), "residual must shrink with more bits");
}

#[test]
fn deterministic_gate_never_misses_on_200_queries() {
    let (d, n) = (64, 2000);
    let mut rng = SplitMix64::new(27);
    let store = Store::new(Epoch::new(28, d, vec![1, 2, 4]));
    store.commit(batch(&mut rng, d, 0, n));
    let (ep, snap) = (store.epoch(), store.pin());
    let calib = Calib::theory(ep);
    for _ in 0..200 {
        let q = unit(&mut rng, d);
        let (got, _) = search_cascade_lut(ep, &snap, &q, 10, &[1, 2, 4], Gate::Deterministic, &calib);
        assert_eq!(got, search_exact(&snap, &q, 10));
    }
}

#[test]
fn lut_cascade_with_huge_z_equals_exact() {
    let d = 64;
    let mut rng = SplitMix64::new(29);
    let store = Store::new(Epoch::new(30, d, vec![1, 2, 4]));
    store.commit(batch(&mut rng, d, 0, 600));
    let (ep, snap) = (store.epoch(), store.pin());
    let calib = Calib::theory(ep);
    let q = unit(&mut rng, d);
    let (got, st) = search_cascade_lut(ep, &snap, &q, 10, &[1, 2, 4], Gate::Global(1e6), &calib);
    assert_eq!(got, search_exact(&snap, &q, 10));
    assert_eq!(st.reranks, 600);
}
