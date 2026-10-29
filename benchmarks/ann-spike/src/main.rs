use rayon::prelude::*;
use serde_json::{json, Value};
use std::time::Instant;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

const DIM: usize = 768;
const K: usize = 10;
const CENTERS: usize = 1024;
const SIGMA: f32 = 0.35;
const QUERY_ID_BASE: u64 = 10_000_000_000;
const CHUNK: usize = 16_384;
const SELS: [(&str, u64); 5] = [
    ("100%", 1000),
    ("50%", 500),
    ("10%", 100),
    ("1%", 10),
    ("0.1%", 1),
];

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        mix(self.0.wrapping_sub(0x9E3779B97F4A7C15))
    }
    fn unit(&mut self) -> f32 {
        ((self.next() >> 40) as f32 + 1.0) / 16_777_217.0
    }
    fn gauss_pair(&mut self) -> (f32, f32) {
        let r = (-2.0 * self.unit().ln()).sqrt();
        let t = 2.0 * std::f32::consts::PI * self.unit();
        (r * t.cos(), r * t.sin())
    }
}

fn make_centers(seed: u64) -> Vec<Vec<f32>> {
    (0..CENTERS)
        .map(|c| {
            let mut r = Rng(mix(seed ^ 0xC3C3 ^ c as u64));
            (0..DIM / 2)
                .flat_map(|_| {
                    let (a, b) = r.gauss_pair();
                    [a, b]
                })
                .collect()
        })
        .collect()
}

fn gen_vec(seed: u64, centers: &[Vec<f32>], id: u64, out: &mut [f32]) {
    let mut r = Rng(mix(seed ^ id.wrapping_mul(0xD1B54A32D192ED03)));
    let c = &centers[(mix(id ^ 0xABCD) % CENTERS as u64) as usize];
    for i in 0..DIM / 2 {
        let (a, b) = r.gauss_pair();
        out[2 * i] = c[2 * i] + SIGMA * a;
        out[2 * i + 1] = c[2 * i + 1] + SIGMA * b;
    }
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    out.iter_mut().for_each(|x| *x /= norm);
}

fn gen_chunk(seed: u64, centers: &[Vec<f32>], start: u64, count: usize) -> Vec<f32> {
    let mut buf = vec![0f32; count * DIM];
    buf.par_chunks_mut(DIM)
        .enumerate()
        .for_each(|(i, o)| gen_vec(seed, centers, start + i as u64, o));
    buf
}

fn passes(id: u64, permille: u64) -> bool {
    mix(id ^ 0xF117E5) % 1000 < permille
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 16];
    for (ca, cb) in a.chunks_exact(16).zip(b.chunks_exact(16)) {
        for i in 0..16 {
            acc[i] += ca[i] * cb[i];
        }
    }
    acc.iter().sum()
}

#[derive(Clone)]
struct Top {
    n: usize,
    s: [f32; K],
    id: [u64; K],
}
impl Top {
    fn new() -> Self {
        Top {
            n: 0,
            s: [f32::MIN; K],
            id: [0; K],
        }
    }
    fn offer(&mut self, s: f32, id: u64) {
        if self.n == K && s <= self.s[K - 1] {
            return;
        }
        let mut i = self.n.min(K - 1);
        if self.n < K {
            self.n += 1;
        }
        while i > 0 && self.s[i - 1] < s {
            self.s[i] = self.s[i - 1];
            self.id[i] = self.id[i - 1];
            i -= 1;
        }
        self.s[i] = s;
        self.id[i] = id;
    }
}

type GroundTruth = Vec<[Vec<u64>; 5]>;

fn ground_truth(seed: u64, centers: &[Vec<f32>], queries: &[Vec<f32>], n: u64) -> GroundTruth {
    const TILE: usize = 16;
    let mut state: Vec<Vec<[Top; 5]>> = queries
        .chunks(TILE)
        .map(|t| {
            t.iter()
                .map(|_| std::array::from_fn(|_| Top::new()))
                .collect()
        })
        .collect();
    let mut start = 0u64;
    while start < n {
        let count = (CHUNK as u64).min(n - start) as usize;
        let chunk = gen_chunk(seed, centers, start, count);
        let masks: Vec<u8> = (0..count)
            .map(|i| {
                SELS.iter().enumerate().fold(0u8, |m, (b, (_, pm))| {
                    if passes(start + i as u64, *pm) {
                        m | (1 << b)
                    } else {
                        m
                    }
                })
            })
            .collect();
        state
            .par_iter_mut()
            .zip(queries.par_chunks(TILE))
            .for_each(|(tops, qs)| {
                for (vi, v) in chunk.chunks_exact(DIM).enumerate() {
                    let id = start + vi as u64;
                    for (qi, q) in qs.iter().enumerate() {
                        let s = dot(q, v);
                        for b in 0..5 {
                            if masks[vi] & (1 << b) != 0 {
                                tops[qi][b].offer(s, id);
                            }
                        }
                    }
                }
            });
        start += count as u64;
    }
    state
        .into_iter()
        .flatten()
        .map(|tops| std::array::from_fn(|b| tops[b].id[..tops[b].n].to_vec()))
        .collect()
}

fn percentiles(mut ms: Vec<f64>) -> Value {
    ms.sort_by(|a, b| a.total_cmp(b));
    let p = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
    json!({"p50_ms": p(0.5), "p95_ms": p(0.95), "p99_ms": p(0.99), "n": ms.len()})
}

fn recall(found: &[u64], truth: &[u64]) -> f64 {
    if truth.is_empty() {
        return 1.0;
    }
    found.iter().filter(|k| truth.contains(k)).count() as f64 / truth.len() as f64
}

fn rss_mb() -> (f64, f64) {
    #[cfg(windows)]
    {
        let cmd = format!(
            "$p=Get-Process -Id {}; \"$($p.WorkingSet64) $($p.PeakWorkingSet64)\"",
            std::process::id()
        );
        if let Ok(o) = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &cmd])
            .output()
        {
            let t = String::from_utf8_lossy(&o.stdout).to_string();
            let v: Vec<f64> = t
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            if v.len() == 2 {
                return (v[0] / 1048576.0, v[1] / 1048576.0);
            }
        }
        (0.0, 0.0)
    }
    #[cfg(target_os = "linux")]
    {
        let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let kb = |k: &str| {
            s.lines()
                .find(|l| l.starts_with(k))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
                .unwrap_or(0.0)
                / 1024.0
        };
        (kb("VmRSS:"), kb("VmHWM:"))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        (0.0, 0.0)
    }
}

fn rss_json(label: &str) -> Value {
    let (cur, peak) = rss_mb();
    json!({"phase": label, "rss_mb": cur, "peak_rss_mb": peak})
}

fn opts(conn: usize, ea: usize, es: usize) -> IndexOptions {
    IndexOptions {
        dimensions: DIM,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F32,
        connectivity: conn,
        expansion_add: ea,
        expansion_search: es,
        multi: false,
    }
}

fn build(o: &IndexOptions, seed: u64, centers: &[Vec<f32>], n: usize) -> (Index, f64) {
    let index = Index::new(o).expect("index new");
    index.reserve(n).expect("reserve");
    let mut add_secs = 0.0;
    let mut start = 0usize;
    while start < n {
        let count = CHUNK.min(n - start);
        let chunk = gen_chunk(seed, centers, start as u64, count);
        let t = Instant::now();
        chunk
            .par_chunks(DIM)
            .enumerate()
            .for_each(|(i, v)| index.add((start + i) as u64, v).expect("add"));
        add_secs += t.elapsed().as_secs_f64();
        start += count;
        if (start / CHUNK) % 16 == 0 {
            eprintln!("  built {start}/{n} ({add_secs:.0}s in add)");
        }
    }
    (index, add_secs)
}

fn search_pass(
    index: &Index,
    queries: &[Vec<f32>],
    gt: &GroundTruth,
    variant: usize,
    filter_permille: Option<u64>,
) -> (f64, Value, Vec<(Vec<u64>, Vec<f32>)>) {
    let run = |q: &Vec<f32>| match filter_permille {
        None => index.search(q, K).expect("search"),
        Some(pm) => index
            .filtered_search(q, K, move |k| passes(k, pm))
            .expect("fsearch"),
    };
    for q in queries.iter().take(50) {
        let _ = run(q);
    }
    let mut lat = Vec::with_capacity(queries.len());
    let mut rec = 0.0;
    let mut raw = Vec::with_capacity(queries.len());
    for (i, q) in queries.iter().enumerate() {
        let t = Instant::now();
        let m = run(q);
        lat.push(t.elapsed().as_secs_f64() * 1e3);
        rec += recall(&m.keys, &gt[i][variant]);
        raw.push((m.keys, m.distances));
    }
    (rec / queries.len() as f64, percentiles(lat), raw)
}

fn concurrent_pass(index: &Index, queries: &[Vec<f32>], threads: usize, reps: usize) -> Value {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    let t = Instant::now();
    let lat: Vec<f64> = pool.install(|| {
        (0..queries.len() * reps)
            .into_par_iter()
            .map(|i| {
                let t = Instant::now();
                let _ = index
                    .search(&queries[i % queries.len()], K)
                    .expect("search");
                t.elapsed().as_secs_f64() * 1e3
            })
            .collect()
    });
    let wall = t.elapsed().as_secs_f64();
    let mut v = percentiles(lat);
    v["threads"] = json!(threads);
    v["qps"] = json!((queries.len() * reps) as f64 / wall);
    v
}

fn arg(args: &[String], k: &str, d: &str) -> String {
    args.iter()
        .position(|a| a == k)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or(d.into())
}

fn write_json(path: &str, v: &Value) {
    if let Some(p) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(p);
    }
    std::fs::write(path, serde_json::to_string_pretty(v).unwrap()).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("full");
    if mode == "loadprobe" {
        let idx = Index::new(&opts(16, 128, 64)).unwrap();
        match idx.load(&arg(&args, "--file", "")) {
            Ok(()) => println!("LOAD_OK size={}", idx.size()),
            Err(e) => println!("LOAD_ERR {e}"),
        }
        return;
    }
    let n: usize = arg(&args, "--n", "1000000").parse().unwrap();
    let nq: usize = arg(&args, "--queries", "1000").parse().unwrap();
    let seed: u64 = arg(&args, "--seed", "42").parse().unwrap();
    let out = arg(&args, "--out", "results/results.json");
    let centers = make_centers(seed);
    let queries: Vec<Vec<f32>> = (0..nq)
        .map(|q| {
            let mut v = vec![0f32; DIM];
            gen_vec(seed, &centers, QUERY_ID_BASE + q as u64, &mut v);
            v
        })
        .collect();

    let mut res = json!({
        "mode": mode, "n": n, "dim": DIM, "k": K, "queries": nq, "seed": seed,
        "data": {"generator": "clustered gaussian mixture", "centers": CENTERS, "sigma": SIGMA, "normalized": true, "metric": "cosine"},
        "threads_available": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "rss": [rss_json("start")],
    });

    eprintln!("ground truth over {n} vectors, {nq} queries ...");
    let t = Instant::now();
    let gt = ground_truth(seed, &centers, &queries, n as u64);
    res["ground_truth_secs"] = json!(t.elapsed().as_secs_f64());
    res["rss"]
        .as_array_mut()
        .unwrap()
        .push(rss_json("after_ground_truth"));
    eprintln!("  done in {:.1}s", t.elapsed().as_secs_f64());

    if mode == "sweep" {
        let mut rows = vec![];
        for conn in [8usize, 16, 32] {
            for ea in [64usize, 128, 256] {
                let (index, secs) = build(&opts(conn, ea, 64), seed, &centers, n);
                for es in [16usize, 32, 64, 128, 256] {
                    index.change_expansion_search(es);
                    let (r, lat, _) = search_pass(&index, &queries, &gt, 0, None);
                    eprintln!("conn={conn} ea={ea} es={es}: recall@10={r:.4}");
                    rows.push(
                        json!({"connectivity": conn, "expansion_add": ea, "expansion_search": es,
                        "build_secs": secs, "recall_at_10": r, "latency_single_thread": lat,
                        "index_memory_mb": index.memory_usage() as f64 / 1048576.0}),
                    );
                }
            }
        }
        res["sweep"] = json!(rows);
        res["rss"].as_array_mut().unwrap().push(rss_json("end"));
        write_json(&out, &res);
        return;
    }

    let (conn, ea) = (
        arg(&args, "--connectivity", "16").parse().unwrap(),
        arg(&args, "--expansion-add", "128").parse().unwrap(),
    );
    let (index, add_secs) = build(&opts(conn, ea, 64), seed, &centers, n);
    res["index"] = json!({"connectivity": conn, "expansion_add": ea, "quantization": "f32",
        "build_add_secs_parallel": add_secs, "insert_throughput_vps_parallel": n as f64 / add_secs,
        "size": index.size(), "memory_usage_mb": index.memory_usage() as f64 / 1048576.0});
    res["rss"]
        .as_array_mut()
        .unwrap()
        .push(rss_json("after_build"));
    write_json(&out, &res);

    {
        let m = 20_000usize;
        let small = Index::new(&opts(conn, ea, 64)).unwrap();
        small.reserve(m).unwrap();
        let chunk = gen_chunk(seed, &centers, 0, m);
        let t = Instant::now();
        for (i, v) in chunk.chunks_exact(DIM).enumerate() {
            small.add(i as u64, v).unwrap();
        }
        res["insert_throughput_vps_single_thread_20k"] =
            json!(m as f64 / t.elapsed().as_secs_f64());
    }

    let mut sweep = vec![];
    let mut chosen: Option<usize> = None;
    for es in [16usize, 32, 64, 128, 256, 512] {
        index.change_expansion_search(es);
        let (r, lat, _) = search_pass(&index, &queries, &gt, 0, None);
        eprintln!("es={es}: recall@10={r:.4} {lat}");
        if chosen.is_none() && r >= 0.95 {
            chosen = Some(es);
        }
        sweep
            .push(json!({"expansion_search": es, "recall_at_10": r, "latency_single_thread": lat}));
    }
    res["expansion_search_sweep"] = json!(sweep);
    let ef = chosen.unwrap_or(512);
    res["chosen_expansion_search"] = json!(ef);
    res["chosen_reaches_recall_0_95"] = json!(chosen.is_some());
    index.change_expansion_search(ef);
    res["concurrent"] = json!([
        concurrent_pass(&index, &queries, 4, 3),
        concurrent_pass(&index, &queries, 12, 3)
    ]);
    res["rss"]
        .as_array_mut()
        .unwrap()
        .push(rss_json("after_search"));
    write_json(&out, &res);

    let mut filtered = vec![];
    for fes in [ef, 512usize] {
        index.change_expansion_search(fes);
        for (b, (label, pm)) in SELS.iter().enumerate().skip(1) {
            let (r, lat, _) = search_pass(&index, &queries, &gt, b, Some(*pm));
            eprintln!("filtered {label} es={fes}: recall@10={r:.4}");
            filtered.push(json!({"selectivity": label, "expansion_search": fes, "recall_at_10": r, "latency_single_thread": lat}));
        }
    }
    res["filtered"] = json!(filtered);
    index.change_expansion_search(ef);
    write_json(&out, &res);

    let (_, _, before) = search_pass(&index, &queries, &gt, 0, None);
    let path = format!("{}.usearch", out.trim_end_matches(".json"));
    let t = Instant::now();
    index.save(&path).expect("save");
    let save_secs = t.elapsed().as_secs_f64();
    let file_mb = std::fs::metadata(&path)
        .map(|m| m.len() as f64 / 1048576.0)
        .unwrap_or(0.0);
    drop(index);
    let loaded = Index::new(&opts(conn, ea, ef)).unwrap();
    let t = Instant::now();
    loaded.load(&path).expect("load");
    let load_secs = t.elapsed().as_secs_f64();
    loaded.change_expansion_search(ef);
    let (_, _, after_load) = search_pass(&loaded, &queries, &gt, 0, None);
    res["rss"]
        .as_array_mut()
        .unwrap()
        .push(rss_json("after_load"));
    drop(loaded);
    let viewed = Index::new(&opts(conn, ea, ef)).unwrap();
    let t = Instant::now();
    viewed.view(&path).expect("view");
    let view_secs = t.elapsed().as_secs_f64();
    viewed.change_expansion_search(ef);
    let (_, _, after_view) = search_pass(&viewed, &queries, &gt, 0, None);
    res["persistence"] = json!({"file_mb": file_mb, "save_secs": save_secs, "load_secs": load_secs, "view_secs": view_secs,
        "identical_after_load": before == after_load, "identical_after_view": before == after_view});
    drop(viewed);

    let probe_src = format!("{}.probe", path);
    {
        let small = Index::new(&opts(conn, ea, ef)).unwrap();
        small.reserve(5_000).unwrap();
        let chunk = gen_chunk(seed, &centers, 0, 5_000);
        for (i, v) in chunk.chunks_exact(DIM).enumerate() {
            small.add(i as u64, v).unwrap();
        }
        small.save(&probe_src).unwrap();
    }
    let good = std::fs::read(&probe_src).unwrap();
    let mut probes = vec![];
    let exe = std::env::current_exe().unwrap();
    for (name, bytes) in [
        ("intact", good.clone()),
        ("truncated_half", good[..good.len() / 2].to_vec()),
        ("truncated_to_header_64B", good[..64].to_vec()),
        ("empty_file", vec![]),
        ("middle_bytes_flipped", {
            let mut b = good.clone();
            let m = b.len() / 2;
            for x in &mut b[m..m + 4096] {
                *x ^= 0xFF;
            }
            b
        }),
    ] {
        let f = format!("{probe_src}.{name}");
        std::fs::write(&f, bytes).unwrap();
        let o = std::process::Command::new(&exe)
            .args(["loadprobe", "--file", &f])
            .output()
            .unwrap();
        probes.push(json!({"case": name, "exit_ok": o.status.success(),
            "stdout": String::from_utf8_lossy(&o.stdout).trim().to_string(),
            "crashed": !o.status.success()}));
        let _ = std::fs::remove_file(&f);
    }
    let _ = std::fs::remove_file(&probe_src);
    res["damaged_file_probes"] = json!(probes);
    res["rss"].as_array_mut().unwrap().push(rss_json("end"));
    write_json(&out, &res);
    eprintln!("wrote {out}");
}
