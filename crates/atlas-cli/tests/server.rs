use atlas_cli::{serve, Server, StaticKeys};
use atlas_core::{Authenticator, Config};
use atlas_engine::Engine;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

const ALICE: &str = "alice-secret-key";
const BOB: &str = "bob-secret-key-1";
const DIM: usize = 4;

struct Running {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Running {
    fn start(dir: &PathBuf) -> Running {
        let keys = StaticKeys::parse(&format!("alice={ALICE},bob={BOB}")).unwrap();
        let engine = Engine::open(dir, Config::new(DIM)).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = Arc::new(Server {
            engine: Arc::new(engine),
            auth: Authenticator::new(keys),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = Arc::clone(&stop);
        let thread = Some(std::thread::spawn(move || serve(server, listener, s2)));
        Running { addr, stop, thread }
    }

    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("atlas-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

fn raw(addr: SocketAddr, head_and_body: &[u8]) -> (u16, String) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.write_all(head_and_body).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).to_string();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

fn call(addr: SocketAddr, method: &str, path: &str, key: Option<&str>, body: &str) -> (u16, Value) {
    let auth = key
        .map(|k| format!("Authorization: Bearer {k}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\n{auth}Content-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let (status, text) = raw(addr, req.as_bytes());
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn insert(addr: SocketAddr, key: &str, id: u64, v: [f32; DIM]) -> (u16, Value) {
    let body = json!({ "id": id, "vector": v, "metadata": { "n": id } }).to_string();
    call(addr, "POST", "/v1/insert", Some(key), &body)
}

#[test]
fn health_console_and_auth_gate() {
    let dir = tmp("auth");
    let srv = Running::start(&dir);
    assert_eq!(
        raw(srv.addr, b"GET /healthz HTTP/1.1\r\n\r\n"),
        (200, "ok".into())
    );
    let (status, page) = raw(srv.addr, b"GET / HTTP/1.1\r\n\r\n");
    assert!(status == 200 && page.contains("Atlas console"));
    assert_eq!(call(srv.addr, "GET", "/v1/count", None, "").0, 401);
    assert_eq!(
        call(srv.addr, "GET", "/v1/count", Some("wrong-wrong-key"), "").0,
        401
    );
    assert_eq!(
        call(srv.addr, "GET", "/v1/count", Some(ALICE), "").1,
        json!({ "count": 0 })
    );
    srv.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn crud_roundtrip_and_ranking() {
    let dir = tmp("crud");
    let srv = Running::start(&dir);
    assert_eq!(insert(srv.addr, ALICE, 1, [1.0, 0.0, 0.0, 0.0]).0, 200);
    assert_eq!(insert(srv.addr, ALICE, 2, [0.0, 1.0, 0.0, 0.0]).0, 200);
    let q = json!({ "vector": [0.9, 0.1, 0.0, 0.0], "k": 2 }).to_string();
    let (st, res) = call(srv.addr, "POST", "/v1/search", Some(ALICE), &q);
    assert_eq!(st, 200);
    assert_eq!(res["hits"][0]["id"], 1);
    assert_eq!(res["hits"][0]["metadata"], json!({ "n": 1 }));
    let f = json!({ "vector": [0.9, 0.1, 0.0, 0.0], "k": 2, "allow_ids": [2] }).to_string();
    let (_, res) = call(srv.addr, "POST", "/v1/search", Some(ALICE), &f);
    assert_eq!(res["hits"].as_array().unwrap().len(), 1);
    assert_eq!(res["hits"][0]["id"], 2);
    let (_, got) = call(srv.addr, "POST", "/v1/fetch", Some(ALICE), r#"{"id":1}"#);
    assert_eq!(got["vector"], json!([1.0, 0.0, 0.0, 0.0]));
    let del = call(srv.addr, "POST", "/v1/delete", Some(ALICE), r#"{"id":1}"#);
    assert_eq!(del.1, json!({ "deleted": true }));
    assert_eq!(
        call(srv.addr, "POST", "/v1/fetch", Some(ALICE), r#"{"id":1}"#).0,
        404
    );
    assert_eq!(
        call(srv.addr, "GET", "/v1/count", Some(ALICE), "").1,
        json!({ "count": 1 })
    );
    srv.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tenants_cannot_see_each_other() {
    let dir = tmp("iso");
    let srv = Running::start(&dir);
    insert(srv.addr, ALICE, 7, [1.0, 1.0, 0.0, 0.0]);
    let q = json!({ "vector": [1.0, 1.0, 0.0, 0.0], "k": 10 }).to_string();
    let (_, res) = call(srv.addr, "POST", "/v1/search", Some(BOB), &q);
    assert_eq!(res["hits"], json!([]));
    assert_eq!(
        call(srv.addr, "POST", "/v1/fetch", Some(BOB), r#"{"id":7}"#).0,
        404
    );
    let del = call(srv.addr, "POST", "/v1/delete", Some(BOB), r#"{"id":7}"#);
    assert_eq!(del.1, json!({ "deleted": false }));
    assert_eq!(
        call(srv.addr, "GET", "/v1/count", Some(ALICE), "").1,
        json!({ "count": 1 })
    );
    let spoof = json!({ "id": 8, "vector": [1.0, 0.0, 0.0, 0.0], "tenant": "alice" }).to_string();
    call(srv.addr, "POST", "/v1/insert", Some(BOB), &spoof);
    assert_eq!(
        call(srv.addr, "GET", "/v1/count", Some(ALICE), "").1,
        json!({ "count": 1 })
    );
    srv.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn idempotent_op_id_and_conflict() {
    let dir = tmp("op");
    let srv = Running::start(&dir);
    let body =
        |id: u64| json!({ "id": id, "vector": [1.0, 0.0, 0.0, 0.0], "op_id": "op-1" }).to_string();
    let a = call(srv.addr, "POST", "/v1/insert", Some(ALICE), &body(5));
    let b = call(srv.addr, "POST", "/v1/insert", Some(ALICE), &body(5));
    assert_eq!((a.0, b.0), (200, 200));
    assert_eq!(a.1["seq"], b.1["seq"]);
    assert_eq!(
        call(srv.addr, "POST", "/v1/insert", Some(ALICE), &body(6)).0,
        409
    );
    assert_eq!(
        call(srv.addr, "GET", "/v1/count", Some(ALICE), "").1,
        json!({ "count": 1 })
    );
    srv.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hostile_input_is_rejected_not_fatal() {
    let dir = tmp("bad");
    let srv = Running::start(&dir);
    let post = |p: &str, b: &str| call(srv.addr, "POST", p, Some(ALICE), b).0;
    assert_eq!(post("/v1/insert", r#"{"id":1,"vector":[1,2]}"#), 400);
    assert_eq!(post("/v1/search", r#"{"vector":[1,0,0,0],"k":0}"#), 400);
    assert_eq!(post("/v1/search", r#"{"vector":[1,0,0,0],"k":1001}"#), 400);
    assert_eq!(
        post("/v1/search", r#"{"vector":[1,0,0,0],"metric":"nope"}"#),
        400
    );
    assert_eq!(post("/v1/insert", "{not json"), 400);
    assert_eq!(post("/v1/nope", "{}"), 404);
    let deep = format!("{}1{}", "[".repeat(5000), "]".repeat(5000));
    let nested = format!(r#"{{"id":1,"vector":[1,0,0,0],"metadata":{deep}}}"#);
    assert_eq!(post("/v1/insert", &nested), 400);
    let big = format!(
        "POST /v1/insert HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        9 * 1024 * 1024
    );
    assert_eq!(raw(srv.addr, big.as_bytes()).0, 413);
    let chunked = b"POST /v1/insert HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n";
    assert_eq!(raw(srv.addr, chunked).0, 400);
    assert_eq!(raw(srv.addr, &vec![b'a'; 40_000]).0, 413);
    assert_eq!(call(srv.addr, "GET", "/v1/count", Some(ALICE), "").0, 200);
    srv.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn concurrent_writers_all_land_and_survive_restart() {
    let dir = tmp("conc");
    let srv = Running::start(&dir);
    let addr = srv.addr;
    let workers: Vec<_> = (0..8u64)
        .map(|w| {
            std::thread::spawn(move || {
                for i in 0..25u64 {
                    let id = w * 1000 + i;
                    assert_eq!(insert(addr, ALICE, id, [id as f32, 1.0, 0.0, 0.0]).0, 200);
                }
            })
        })
        .collect();
    workers.into_iter().for_each(|w| w.join().unwrap());
    assert_eq!(
        call(addr, "GET", "/v1/count", Some(ALICE), "").1,
        json!({ "count": 200 })
    );
    srv.shutdown();
    let again = Running::start(&dir);
    let n = call(again.addr, "GET", "/v1/count", Some(ALICE), "").1;
    assert_eq!(n, json!({ "count": 200 }));
    again.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn key_parsing_rejects_weak_or_malformed_specs() {
    assert!(StaticKeys::parse("").is_err());
    assert!(StaticKeys::parse("alice=short").is_err());
    assert!(StaticKeys::parse("no-equals-sign").is_err());
    assert!(StaticKeys::parse("bad tenant!=longenoughsecret").is_err());
    assert!(StaticKeys::parse("alice=longenoughsecret").is_ok());
    assert!(StaticKeys::parse("alice=samesecret1234,bob=samesecret1234").is_err());
    assert!(StaticKeys::parse("alice=samesecret1234,bob=othersecret5678").is_ok());
}

#[test]
fn duplicate_content_length_is_rejected() {
    let dir = tmp("dup-len");
    let srv = Running::start(&dir);
    let req = format!("POST /v1/count HTTP/1.1\r\nauthorization: Bearer {ALICE}\r\ncontent-length: 2\r\ncontent-length: 2\r\n\r\n{{}}");
    let (status, _) = raw(srv.addr, req.as_bytes());
    assert_eq!(status, 400);
    srv.shutdown();
}

#[test]
fn slow_drip_request_is_cut_off_by_total_deadline() {
    let dir = tmp("slowloris");
    let srv = Running::start(&dir);
    let mut s = TcpStream::connect(srv.addr).unwrap();
    let started = std::time::Instant::now();
    let head = b"POST /v1/insert HTTP/1.1\r\nauthorization: Bearer x\r\n";
    let limit = atlas_cli::http::REQUEST_DEADLINE + std::time::Duration::from_secs(6);
    let mut cut = false;
    for b in head.iter().cycle().take(200) {
        if s.write_all(&[*b]).is_err() {
            cut = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
        if started.elapsed() > limit {
            break;
        }
    }
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out);
    assert!(
        cut || text.starts_with("HTTP/1.1 408"),
        "connection still open after deadline: {text}"
    );
    assert!(started.elapsed() < limit + std::time::Duration::from_secs(2));
    srv.shutdown();
}

#[test]
fn metadata_filter_search_over_http() {
    let dir = tmp("filter-http");
    let srv = Running::start(&dir);
    for (id, color, n) in [(1u64, "red", 5), (2, "blue", 7), (3, "red", 50)] {
        let body = json!({"id": id, "vector": [1.0, 0.0, 0.0, id as f32], "metadata": {"c": color, "n": n}});
        let (st, _) = call(
            srv.addr,
            "POST",
            "/v1/insert",
            Some(ALICE),
            &body.to_string(),
        );
        assert_eq!(st, 200);
    }
    let q = json!({"vector": [1.0, 0.0, 0.0, 1.0], "k": 5,
        "filter": {"and": [{"eq": ["c", "red"]}, {"range": {"field": "n", "lt": 10}}]}});
    let (st, out) = call(srv.addr, "POST", "/v1/search", Some(ALICE), &q.to_string());
    assert_eq!(st, 200);
    let ids: Vec<u64> = out["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, vec![1]);
    let bad = json!({"vector": [1.0, 0.0, 0.0, 1.0], "filter": {"nope": 1}});
    let (st, out) = call(
        srv.addr,
        "POST",
        "/v1/search",
        Some(ALICE),
        &bad.to_string(),
    );
    assert_eq!((st, out["error"].as_str()), (400, Some("INVALID_FILTER")));
    let both =
        json!({"vector": [1.0, 0.0, 0.0, 1.0], "filter": {"eq": ["c", "red"]}, "allow_ids": [1]});
    let (st, _) = call(
        srv.addr,
        "POST",
        "/v1/search",
        Some(ALICE),
        &both.to_string(),
    );
    assert_eq!(st, 400);
    let (st, out) = call(srv.addr, "POST", "/v1/search", Some(BOB), &q.to_string());
    assert_eq!(st, 200);
    assert!(out["hits"].as_array().unwrap().is_empty());
    srv.shutdown();
}
