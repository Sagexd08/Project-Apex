use atlas_core::{Metric, VectorId};
use atlas_index::{AnnIndex, FlatIndex, HnswIndex};
use serde_json::json;
use std::time::Instant;

fn main() {
    let a: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    let (n, dim, ef) = (
        a.first().copied().unwrap_or(50_000),
        a.get(1).copied().unwrap_or(64),
        a.get(2).copied().unwrap_or(128),
    );
    let mut s = 12345u64;
    let mut rnd = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((s >> 40) as f32 / (1u64 << 24) as f32) - 0.5
    };
    let centers: Vec<Vec<f32>> = (0..256)
        .map(|_| (0..dim).map(|_| rnd() * 4.0).collect())
        .collect();
    let mut gen =
        |i: usize| -> Vec<f32> { centers[i % 256].iter().map(|c| c + rnd() * 0.7).collect() };
    let data: Vec<Vec<f32>> = (0..n).map(&mut gen).collect();
    let queries: Vec<Vec<f32>> = (0..300).map(|i| gen(i * 7 + 1)).collect();

    let mut flat = FlatIndex::new(dim);
    let mut h = HnswIndex::new(dim).with_ef_search(ef);
    let t = Instant::now();
    for (i, v) in data.iter().enumerate() {
        flat.insert(VectorId(i as u64), v, json!(null)).unwrap();
    }
    let t_flat = t.elapsed();
    let t = Instant::now();
    for (i, v) in data.iter().enumerate() {
        h.insert(VectorId(i as u64), v, json!(null)).unwrap();
    }
    let t_h = t.elapsed();

    let time = |ix: &dyn AnnIndex| -> (Vec<Vec<u64>>, f64, f64) {
        let mut lat = vec![];
        let mut res = vec![];
        for q in &queries {
            let t = Instant::now();
            let r = ix.search(q, 10, Metric::Cosine).unwrap();
            lat.push(t.elapsed().as_secs_f64() * 1e3);
            res.push(r.iter().map(|h| h.id.0).collect());
        }
        lat.sort_by(|a, b| a.total_cmp(b));
        (res, lat[lat.len() / 2], lat[lat.len() * 95 / 100])
    };
    let (want, f50, f95) = time(&flat);
    let (got, h50, h95) = time(&h);
    let hits: usize = want
        .iter()
        .zip(&got)
        .map(|(w, g)| w.iter().filter(|x| g.contains(x)).count())
        .sum();
    println!(
        "N={n} dim={dim} ef_search={ef} queries={} (synthetic clustered, single thread)",
        queries.len()
    );
    println!(
        "build  flat {:.2}s | hnsw {:.2}s ({:.0} inserts/s)",
        t_flat.as_secs_f64(),
        t_h.as_secs_f64(),
        n as f64 / t_h.as_secs_f64()
    );
    println!(
        "recall@10 hnsw vs exact: {:.4}",
        hits as f64 / (want.len() * 10) as f64
    );
    println!(
        "latency ms  flat p50 {f50:.2} p95 {f95:.2} | hnsw p50 {h50:.3} p95 {h95:.3}  (p50 speedup {:.0}x)",
        f50 / h50.max(1e-9)
    );
}
