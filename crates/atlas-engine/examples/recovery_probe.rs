use atlas_core::{
    AuthContext, Config, EffectiveScope, KeyScope, PermSet, Principal, TenantId, VectorId,
};
use atlas_engine::Engine;
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;

fn scope() -> EffectiveScope {
    let t = TenantId::parse("legacy").unwrap();
    AuthContext::for_test(
        Principal {
            tenant: t.clone(),
            perms: PermSet::ALL,
        },
        KeyScope {
            tenant: t,
            allowed: PermSet::ALL,
        },
    )
    .effective()
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);
    let dim = 64usize;
    let keep = std::env::var_os("RECOVERY_PROBE_DIR").map(std::path::PathBuf::from);
    let dir = keep.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("recovery-probe-{}", std::process::id()))
    });
    let reuse = keep.is_some() && dir.join("wal.log").exists();
    if keep.is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    if !reuse {
        let e = Arc::new(Engine::open(&dir, Config::new(dim)).unwrap());
        let t0 = Instant::now();
        let handles: Vec<_> = (0..32)
            .map(|w| {
                let e = e.clone();
                std::thread::spawn(move || {
                    let s = scope();
                    let mut x = 0x9E3779B97F4A7C15u64 ^ w as u64;
                    for i in (w..n).step_by(32) {
                        let v: Vec<f32> = (0..dim)
                            .map(|_| {
                                x ^= x << 13;
                                x ^= x >> 7;
                                x ^= x << 17;
                                (x >> 40) as f32 / 16_777_216.0 - 0.5
                            })
                            .collect();
                        e.insert(&s, VectorId(i as u64), &v, Value::Null).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        println!("load {n} in {:.1}s", t0.elapsed().as_secs_f64());
    }
    let wal = std::fs::metadata(dir.join("wal.log"))
        .map(|m| m.len())
        .unwrap_or(0);
    println!("wal_bytes {wal}");
    let t = Instant::now();
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    println!("open_from_wal_ms {:.0}", t.elapsed().as_secs_f64() * 1000.0);
    let t = Instant::now();
    e.checkpoint().unwrap();
    println!("checkpoint_ms {:.0}", t.elapsed().as_secs_f64() * 1000.0);
    drop(e);
    let t = Instant::now();
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    println!(
        "open_from_segment_ms {:.0}",
        t.elapsed().as_secs_f64() * 1000.0
    );
    println!("count {}", e.count(&scope()).unwrap());
    drop(e);
    if keep.is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}
