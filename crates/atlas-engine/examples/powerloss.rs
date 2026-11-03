use atlas_core::{
    AuthContext, Config, Durability, EffectiveScope, KeyScope, Metric, PermSet, Principal,
    RecoverMode, TenantId, VectorId,
};
use atlas_engine::Engine;
use serde_json::json;
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::exit;

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

fn read_acks(path: &Path) -> BTreeSet<u64> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let complete = match text.rfind('\n') {
        Some(i) => &text[..=i],
        None => "",
    };
    complete
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

fn write(dir: &Path, ack_log: &Path, sync: bool) -> ! {
    let mut cfg = Config::new(4);
    if sync {
        cfg.durability = Durability::SyncPerCommit;
    }
    let eng = Engine::open(dir, cfg).unwrap_or_else(|e| {
        eprintln!("open failed: {e:?} (run verify first if this is a restart)");
        exit(2)
    });
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(ack_log)
        .expect("open ack_log");
    let sc = scope();
    let mut id = read_acks(ack_log).last().copied().unwrap_or(0) + 1;
    println!(
        "WRITING mode={} engine_dir={} ack_log={} start_id={id}. Cut power now when ready.",
        if sync { "sync" } else { "group" },
        dir.display(),
        ack_log.display()
    );
    loop {
        eng.insert(&sc, VectorId(id), &[1.0; 4], json!({ "i": id }))
            .expect("insert");
        writeln!(log, "{id}").expect("ack_log write");
        log.sync_all().expect("ack_log fsync");
        if id % 200 == 0 {
            println!("acked {id}");
        }
        id += 1;
    }
}

fn verify(dir: &Path, ack_log: &Path) -> ! {
    let acked = read_acks(ack_log);
    let max_acked = acked.last().copied().unwrap_or(0);
    let mut repaired = false;
    let eng = match Engine::open(dir, Config::reopen()) {
        Ok(e) => e,
        Err(e) if e.code() == "RECOVERY_REQUIRED" || e.code() == "WAL_CORRUPT" => {
            repaired = true;
            let mut c = Config::reopen();
            c.recover_mode = RecoverMode::Repair;
            Engine::open(dir, c).unwrap_or_else(|e| {
                println!("POWERLOSS_VERIFY FAIL: repair open failed {e:?}");
                exit(1)
            })
        }
        Err(e) => {
            println!("POWERLOSS_VERIFY FAIL: open error {e:?}");
            exit(1)
        }
    };
    let k = max_acked as usize + 100_000;
    let got: BTreeSet<u64> = eng
        .search(&scope(), &[1.0; 4], k, Metric::Dot)
        .expect("search")
        .iter()
        .map(|h| h.id.0)
        .collect();
    let missing: Vec<u64> = acked.difference(&got).copied().take(10).collect();
    let n_missing = acked.difference(&got).count();
    let max_got = got.last().copied().unwrap_or(0);
    let gaps = (1..=max_got).filter(|i| !got.contains(i)).count();
    let pass = n_missing == 0 && gaps == 0;
    println!(
        "POWERLOSS_VERIFY acked={} recovered={} max_acked={max_acked} max_recovered={max_got} missing={n_missing} {missing:?} gaps={gaps} needed_repair={repaired} {}",
        acked.len(),
        got.len(),
        if pass { "PASS" } else { "FAIL" }
    );
    if acked.is_empty() {
        println!("NOTE: ack_log is empty; nothing was tested.");
        exit(1)
    }
    exit(if pass { 0 } else { 1 })
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let sync = !a.windows(2).any(|w| w[0] == "--mode" && w[1] == "group");
    match (a.get(1).map(String::as_str), a.get(2), a.get(3)) {
        (Some("write"), Some(d), Some(l)) => write(Path::new(d), Path::new(l), sync),
        (Some("verify"), Some(d), Some(l)) => verify(Path::new(d), Path::new(l)),
        _ => {
            eprintln!("usage: powerloss write|verify <engine_dir> <ack_log> [--mode sync|group]");
            exit(2)
        }
    }
}
