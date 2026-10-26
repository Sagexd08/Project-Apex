//! Hard-kill test: a child process inserts with acks and is killed mid-write;
//! every id it acknowledged on stdout must be present after reopen.
use atlas_core::{Config, Metric, VectorId};
use atlas_engine::Engine;
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

const CHILD_DIR: &str = "ATLAS_KILL_CHILD_DIR";

/// Child body; a no-op unless re-invoked by `acked_ids_survive_hard_kill`.
#[test]
fn child_writer() {
    let Ok(dir) = std::env::var(CHILD_DIR) else {
        return;
    };
    let eng = Engine::open(dir, Config::new(4)).unwrap();
    for id in 0u64.. {
        eng.insert(VectorId(id), &[id as f32; 4], json!(null))
            .unwrap();
        println!("ACK {id}");
    }
}

#[test]
fn acked_ids_survive_hard_kill() {
    let dir = std::env::temp_dir().join(format!("atlas-kill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["child_writer", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_DIR, &dir)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut acked = Vec::new();
    while acked.len() < 40 {
        let line = lines.next().expect("child exited early").unwrap();
        if let Some(id) = line.strip_prefix("ACK ") {
            acked.push(id.parse::<u64>().unwrap());
        }
    }
    child.kill().unwrap(); // TerminateProcess / SIGKILL
    for line in lines.map_while(Result::ok) {
        if let Some(id) = line.strip_prefix("ACK ") {
            acked.push(id.parse::<u64>().unwrap());
        }
    }
    child.wait().unwrap();

    let eng = Engine::open(&dir, Config::reopen()).unwrap();
    let present: std::collections::HashSet<u64> = eng
        .search(&[0.0; 4], eng.len(), Metric::Dot)
        .unwrap()
        .iter()
        .map(|h| h.id.0)
        .collect();
    assert!(acked.len() >= 40);
    for id in &acked {
        assert!(present.contains(id), "acked id {id} missing after kill");
    }
}
