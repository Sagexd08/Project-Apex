//! Slice 3: process-termination crash loop (NOT power loss: a killed process leaves the OS
//! page cache intact, so this proves nothing about unflushed OS/drive caches).
//!
//! A child process (this test binary re-invoked) runs concurrent writers over 8 hot ids and
//! prints `ACK <seq> <id> <thread> <iter>` only after `insert` returned Ok. The parent kills it
//! hard (TerminateProcess / SIGKILL) at a randomized point, reopens, and checks:
//! every acked write survives, no invented ids, same-id order never regresses behind an acked
//! write, seq stays monotonic, and reopening twice yields identical state.
//! `ATLAS_CRASH_ITERS` overrides the default of 100 iterations.
mod common;
use atlas_core::{Config, Metric, VectorId};
use atlas_engine::Engine;
use common::single_tenant_full;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering::SeqCst;
use std::sync::Arc;

const HOT_IDS: u64 = 8;
const DEFAULT_ITERS: u64 = 100;

fn env_u64(k: &str, d: u64) -> u64 {
    std::env::var(k)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d)
}

/// Child body; a no-op unless re-invoked by the parent tests below.
#[test]
fn crash_child() {
    let Ok(dir) = std::env::var("ATLAS_CRASH_DIR") else {
        return;
    };
    let mut cfg = Config::new(4);
    cfg.max_batch = env_u64("ATLAS_CRASH_BATCH", 128) as usize;
    let eng = Arc::new(Engine::open(dir, cfg).unwrap());
    eng.flush_delay_handle()
        .store(env_u64("ATLAS_CRASH_DELAY_MS", 0), SeqCst);
    println!("READY");
    let hs: Vec<_> = (0..env_u64("ATLAS_CRASH_THREADS", 1))
        .map(|t| {
            let eng = eng.clone();
            std::thread::spawn(move || {
                for i in 0u64.. {
                    let id = (i * 7 + t) % HOT_IDS;
                    let md = json!({"t": t, "i": i});
                    match eng.insert(&single_tenant_full(), VectorId(id), &[t as f32; 4], md) {
                        Ok(seq) => println!("ACK {seq} {id} {t} {i}"),
                        Err(_) => return,
                    }
                }
            })
        })
        .collect();
    for h in hs {
        let _ = h.join();
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, m: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % m
    }
}

struct Params {
    threads: u64,
    batch: u64,
    delay_ms: u64,
    target_acks: usize,
    extra_sleep_ms: u64,
}

type Ack = (u64, u64, u64, u64); // seq, id, thread, iter

fn snapshot(eng: &Engine) -> HashMap<u64, Value> {
    eng.search(&single_tenant_full(), &[0.0; 4], 1000, Metric::Dot)
        .unwrap()
        .into_iter()
        .map(|h| (h.id.0, h.metadata))
        .collect()
}

fn run_once(tag: &str, p: &Params) -> Result<(usize, usize), String> {
    let dir = std::env::temp_dir().join(format!("atlas-crash-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["crash_child", "--exact", "--nocapture", "--test-threads=1"])
        .env("ATLAS_CRASH_DIR", &dir)
        .env("ATLAS_CRASH_THREADS", p.threads.to_string())
        .env("ATLAS_CRASH_BATCH", p.batch.to_string())
        .env("ATLAS_CRASH_DELAY_MS", p.delay_ms.to_string())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn: {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        for l in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(l);
        }
    });
    let mut acked: Vec<Ack> = Vec::new();
    let parse = |line: &str, acked: &mut Vec<Ack>| {
        let f: Vec<&str> = line.split(' ').collect();
        if f.len() == 5 && f[0] == "ACK" {
            if let Ok(v) = f[1..]
                .iter()
                .map(|x| x.parse::<u64>())
                .collect::<Result<Vec<_>, _>>()
            {
                acked.push((v[0], v[1], v[2], v[3]));
            }
        }
    };
    let mut ready = false;
    while !ready || acked.len() < p.target_acks {
        match rx.recv_timeout(std::time::Duration::from_secs(30)) {
            // the libtest harness prefixes the first line: "test crash_child ... READY"
            Ok(l) if l.ends_with("READY") => ready = true,
            Ok(l) => parse(&l, &mut acked),
            Err(e) => {
                let _ = child.kill();
                return Err(format!("no output from child: {e}"));
            }
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(p.extra_sleep_ms));
    child.kill().map_err(|e| format!("kill: {e}"))?; // TerminateProcess / SIGKILL
    let _ = reader.join(); // drains everything the child printed before dying
    for l in rx.try_iter() {
        parse(&l, &mut acked); // a partially printed final line fails to parse: not an ack
    }
    let _ = child.wait();

    let eng = Engine::open(&dir, Config::reopen()).map_err(|e| format!("reopen: {e}"))?;
    let state = snapshot(&eng);
    let seqs: HashSet<u64> = acked.iter().map(|a| a.0).collect();
    if seqs.len() != acked.len() {
        return Err("duplicate acked seq".into());
    }
    if state.keys().any(|id| *id >= HOT_IDS) {
        return Err("invented id".into());
    }
    let tags: HashMap<(u64, u64), u64> = acked.iter().map(|a| ((a.2, a.3), a.0)).collect();
    let mut latest: HashMap<u64, &Ack> = HashMap::new();
    for a in &acked {
        if latest.get(&a.1).is_none_or(|l| a.0 > l.0) {
            latest.insert(a.1, a);
        }
    }
    for (id, a) in &latest {
        let md = state
            .get(id)
            .ok_or_else(|| format!("acked id {id} (seq {}) missing", a.0))?;
        let key = (
            md["t"].as_u64().unwrap_or(u64::MAX),
            md["i"].as_u64().unwrap_or(u64::MAX),
        );
        if let Some(seq) = tags.get(&key) {
            if seq < &a.0 {
                return Err(format!(
                    "id {id}: stale acked seq {seq} visible behind acked seq {}",
                    a.0
                ));
            }
        }
    }
    let present = state.len();
    let max_acked = acked.iter().map(|a| a.0).max().unwrap_or(0);
    // restart idempotency + monotonic seq
    let next = eng
        .insert(
            &single_tenant_full(),
            VectorId(0),
            &[9.0; 4],
            json!({"t": 99, "i": 0}),
        )
        .map_err(|e| format!("post-recovery insert: {e}"))?;
    if next <= max_acked {
        return Err(format!(
            "seq not monotonic: next {next} <= acked {max_acked}"
        ));
    }
    drop(eng);
    let again = Engine::open(&dir, Config::reopen()).map_err(|e| format!("second reopen: {e}"))?;
    let mut expected = state;
    expected.insert(0, json!({"t": 99, "i": 0}));
    if snapshot(&again) != expected {
        return Err("state differs after second reopen".into());
    }
    drop(again);
    let third = Engine::open(&dir, Config::reopen()).map_err(|e| format!("third reopen: {e}"))?;
    if snapshot(&third) != expected {
        return Err("state differs after third reopen".into());
    }
    Ok((acked.len(), present))
}

#[test]
fn kill_during_pending_group_commit_never_exposes_invalid_records() {
    // 4 writers, 300 ms flush delay: killed while a batch is queued/being flushed.
    let p = Params {
        threads: 4,
        batch: 128,
        delay_ms: 300,
        target_acks: 0,
        extra_sleep_ms: 120,
    };
    let (acked, present) = run_once("pending", &p).unwrap();
    println!("pending-commit kill: acked={acked} present_ids={present}");
}

#[test]
fn crash_loop_reports_pass_fail_counts() {
    let iters = env_u64("ATLAS_CRASH_ITERS", DEFAULT_ITERS);
    let mut failures = Vec::new();
    let (mut acks, mut pending_kills) = (0usize, 0u64);
    for i in 0..iters {
        let mut r = Lcg(i + 1);
        let delay_ms = [0, 0, 5, 30][r.next(4) as usize];
        let p = Params {
            threads: 1 + r.next(4),
            batch: [1, 4, 16, 128][r.next(4) as usize],
            delay_ms,
            target_acks: 1 + r.next(if delay_ms > 0 { 8 } else { 40 }) as usize,
            extra_sleep_ms: r.next(15),
        };
        if delay_ms > 0 {
            pending_kills += 1;
        }
        match run_once(&format!("loop{i}"), &p) {
            Ok((a, _)) => acks += a,
            Err(e) => failures.push(format!(
                "iter {i} (threads={} batch={} delay={}ms target={} sleep={}ms): {e}",
                p.threads, p.batch, p.delay_ms, p.target_acks, p.extra_sleep_ms
            )),
        }
    }
    println!(
        "crash loop: iters={iters} passed={} failed={} acked_writes_checked={acks} slow_flush_iters={pending_kills}",
        iters - failures.len() as u64,
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "crash loop failures:\n{}",
        failures.join("\n")
    );
}
