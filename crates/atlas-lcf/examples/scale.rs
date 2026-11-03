//! Scale run through the real write path: N nodes and about 3N relationships written with
//! concurrent writers into a real engine on disk, then a restart (index rebuild from durable
//! records) and query latency. Usage: cargo run --release -p atlas-lcf --example scale -- <N> [dir]
use atlas_core::{AuthContext, Config, KeyScope, PermSet, Principal, TenantId};
use atlas_engine::Engine;
use atlas_lcf::{cnr::Opts, Edge, Lcf, LcfState, Node};
use std::time::Instant;

/// ATLAS_INDEX=hnsw measures the graph index; default is the exact scan.
fn open_engine(dir: &std::path::Path, cfg: Config) -> Engine {
    if std::env::var("ATLAS_INDEX").as_deref() == Ok("hnsw") {
        Engine::open_with_index(dir, cfg, atlas_index::HnswIndex::factory()).unwrap()
    } else {
        Engine::open(dir, cfg).unwrap()
    }
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() as f64 - 1.0) * p) as usize]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|x| x.parse().ok()).unwrap_or(10_000);
    let dir = args
        .get(2)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("atlas-scale-{}", std::process::id()))
        });
    let _ = std::fs::remove_dir_all(&dir);
    let t = TenantId::parse("scale").unwrap();
    let sc = AuthContext::for_test(
        Principal {
            tenant: t.clone(),
            perms: PermSet::ALL,
        },
        KeyScope {
            tenant: t,
            allowed: PermSet::ALL,
        },
    )
    .effective();
    let words = [
        "latency",
        "recovery",
        "checkpoint",
        "replay",
        "index",
        "filter",
        "planner",
        "segment",
        "tenant",
        "wal",
        "vector",
        "graph",
        "evidence",
        "decision",
        "experiment",
        "failure",
        "cache",
        "memory",
        "compaction",
        "version",
    ];
    let text = |i: usize| {
        format!(
            "{} {} {} item {i}",
            words[i % 20],
            words[(i * 7 + 3) % 20],
            words[(i * 13 + 5) % 20]
        )
    };
    let mk = |i: usize| Node {
        key: format!("n{i}"),
        kind: ["decision", "experiment", "observation", "fact"][i % 4].into(),
        title: text(i),
        body: format!("{} {}", text(i + 1), text(i + 2)),
        source: "scale:synthetic".into(),
        status: "observed".into(),
        ts: 1_700_000_000 + i as i64,
        ver: 0,
    };
    let mk_edge = |a: usize, b: usize, rel: &str| Edge {
        rel: rel.into(),
        from: format!("n{a}"),
        to: format!("n{b}"),
        source: "scale:synthetic".into(),
        ts: 1,
        status: "observed".into(),
        ver: 0,
    };
    let writers = 16;
    let t0 = Instant::now();
    {
        let e = open_engine(&dir, Config::new(64));
        let st = LcfState::default();
        let l = Lcf::new(&e, &st);
        std::thread::scope(|s| {
            for w in 0..writers {
                let (l, sc, mk) = (&l, &sc, &mk);
                s.spawn(move || {
                    for i in (w..n).step_by(writers) {
                        l.put_node(sc, &mk(i)).unwrap();
                    }
                });
            }
        });
        let nodes_s = t0.elapsed().as_secs_f64();
        let t1 = Instant::now();
        let rels = ["SUPPORTS", "DERIVED_FROM", "CAUSED_BY"];
        std::thread::scope(|s| {
            for w in 0..writers {
                let (l, sc, mk_edge) = (&l, &sc, &mk_edge);
                s.spawn(move || {
                    for i in (w..n).step_by(writers) {
                        for (j, rel) in rels.iter().enumerate() {
                            let to = (i * (j + 2) + 17 + j) % n;
                            if to != i {
                                let _ = l.put_edge(sc, &mk_edge(i, to, rel));
                            }
                        }
                    }
                });
            }
        });
        let edges_s = t1.elapsed().as_secs_f64();
        let st = l.stats(&sc).unwrap();
        println!("write: {n} nodes in {nodes_s:.1}s ({:.0}/s), {} relationships in {edges_s:.1}s ({:.0}/s), {writers} writers, records {}", n as f64 / nodes_s, st["edges"], st["edges"].as_f64().unwrap() / edges_s, st["records"]);
    }
    let size: u64 = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();
    let t2 = Instant::now();
    let e = open_engine(&dir, Config::reopen());
    let open_s = t2.elapsed().as_secs_f64();
    let st = LcfState::default();
    let l = Lcf::new(&e, &st);
    let t3 = Instant::now();
    let ver = l.current_version(&sc).unwrap();
    println!(
        "restart: engine open {open_s:.2}s, index rebuild {:.2}s, version {ver}, on-disk {:.1} MB",
        t3.elapsed().as_secs_f64(),
        size as f64 / 1e6
    );
    let qs = [
        "why did the checkpoint decision happen",
        "evidence for replay failure",
        "experiment about index latency",
        "what superseded the planner",
        "tenant memory version",
    ];
    let mut lat = vec![];
    for r in 0..60 {
        let t = Instant::now();
        let res = l.query(&sc, qs[r % qs.len()], Opts::default()).unwrap();
        lat.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(res["stats"]["nodes"].as_u64().unwrap() <= 40);
    }
    let mut a = lat.clone();
    println!(
        "query (60 runs, 3 hops, <=40 nodes): p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        pct(&mut a, 0.5),
        pct(&mut a, 0.95),
        pct(&mut a, 1.0)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
