mod common;
use atlas_core::{Config, VectorId};
use atlas_engine::Engine;
use atlas_storage::{write_meta, Wal};
use common::full;
use serde_json::json;
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

struct Counting;
static CUR: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let now = CUR.fetch_add(l.size(), SeqCst) + l.size();
            PEAK.fetch_max(now, SeqCst);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        CUR.fetch_sub(l.size(), SeqCst);
    }
}

#[global_allocator]
static A: Counting = Counting;

static SERIAL: Mutex<()> = Mutex::new(());
const TENANTS: usize = 4;
const BIG: usize = 1 << 20;

fn payload(i: usize, pad: usize) -> Vec<u8> {
    format!(
        r#"{{"tenant":"t{}","id":{i},"vector":[1.0,0.0,0.0,0.0],"metadata":{{"p":"{}"}},"op_id":"op-{i}"}}"#,
        i % TENANTS,
        "x".repeat(pad)
    )
    .into_bytes()
}

fn prewrite(dir: &Path, payloads: Vec<Vec<u8>>) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    write_meta(dir, 4).unwrap();
    let (wal, _) = Wal::open(&dir.join("wal.log"), &Config::new(4), Box::new(|_, _| {})).unwrap();
    let wal = Arc::new(wal);
    let queue = Arc::new(Mutex::new(payloads));
    let hs: Vec<_> = (0..32)
        .map(|_| {
            let (wal, queue) = (wal.clone(), queue.clone());
            std::thread::spawn(move || loop {
                let Some(p) = queue.lock().unwrap().pop() else {
                    return;
                };
                wal.append(p).unwrap();
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
}

struct Run {
    retained: usize,
    peak: usize,
    total_payload: usize,
}

fn run(name: &str, n: usize, pad: usize, bigs: usize) -> Run {
    let dir = std::env::temp_dir().join(format!("atlas-opmem-{name}-{}", std::process::id()));
    let mut payloads: Vec<Vec<u8>> = (0..n).map(|i| payload(i, pad)).collect();
    payloads.extend((n..n + bigs).map(|i| payload(i, BIG)));
    let total_payload = payloads.iter().map(Vec::len).sum();
    prewrite(&dir, payloads);

    let before = CUR.load(SeqCst);
    PEAK.store(before, SeqCst);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    let (after, peak) = (CUR.load(SeqCst), PEAK.load(SeqCst));
    assert_eq!(e.len(), n + bigs);

    let md = |p: usize| json!({ "p": "x".repeat(p) });
    let retry = |i: usize, p: usize| {
        let t = format!("t{}", i % TENANTS);
        e.insert_op(
            &full(&t),
            &format!("op-{i}"),
            VectorId(i as u64),
            &[1.0, 0.0, 0.0, 0.0],
            md(p),
        )
    };
    let (mid, retries) = (CUR.load(SeqCst), n.min(20_000));
    for i in 0..retries {
        retry(i, pad).unwrap_or_else(|er| panic!("retry {i}: {er}"));
    }
    for i in n..n + bigs {
        retry(i, BIG).unwrap_or_else(|er| panic!("big retry {i}: {er}"));
    }
    let grown = CUR.load(SeqCst).saturating_sub(mid);
    assert!(grown < 256 * 1024, "retries grew the heap by {grown} bytes");
    let (entries, retained) = (n + bigs, after - before);
    println!(
        "{name}: entries={entries} payload_total={total_payload}B retained={retained}B \
         peak_growth={}B retained/entry={:.0}B payload/entry={:.0}B",
        peak - before,
        retained as f64 / entries as f64,
        total_payload as f64 / entries as f64
    );
    Run {
        retained,
        peak: peak - before,
        total_payload,
    }
}

#[test]
fn op_map_memory_is_bounded_per_entry_and_retries_do_not_grow_it() {
    let _g = SERIAL.lock().unwrap();
    let (n, bigs) = (20_000, 5);
    let r = run("default", n, 150, bigs);
    let bound = 2 * r.total_payload + 1024 * (n + bigs);
    assert!(
        r.retained <= bound,
        "retained {} > bound {bound}",
        r.retained
    );
    assert!(
        r.peak <= bound + r.total_payload + (8 << 20),
        "peak {}",
        r.peak
    );
}

#[test]
#[ignore = "large run (1M entries, ~GBs of heap); prints measurements"]
fn op_map_memory_one_million_entries() {
    let _g = SERIAL.lock().unwrap();
    run("million", 1_000_000, 150, 0);
}
