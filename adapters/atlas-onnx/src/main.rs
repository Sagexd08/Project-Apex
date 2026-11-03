use atlas_onnx::{Embedder, DIM};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const MAX_BODY: usize = 1 << 20;
const MODEL_NAME: &str = "all-MiniLM-L6-v2";

fn respond(s: &mut TcpStream, status: &str, body: &Value) {
    let text = body.to_string();
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        text.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(text.as_bytes());
}

fn read_request(s: &mut TcpStream) -> Result<(String, Vec<u8>), String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = s.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("closed".into());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_BODY {
            return Err("headers too large".into());
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let len: usize = head
        .lines()
        .find_map(|l| {
            l.to_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse().unwrap_or(0))
        })
        .unwrap_or(0);
    if len > MAX_BODY {
        return Err("body too large".into());
    }
    while buf.len() < head_end + len {
        let n = s.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok((head, buf[head_end..].to_vec()))
}

fn handle(mut s: TcpStream, e: &Embedder) {
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let Ok((head, body)) = read_request(&mut s) else {
        return respond(&mut s, "400 Bad Request", &json!({"error": "bad request"}));
    };
    if !head.starts_with("POST /v1/embeddings") {
        return respond(
            &mut s,
            "404 Not Found",
            &json!({"error": "POST /v1/embeddings only"}),
        );
    }
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return respond(&mut s, "400 Bad Request", &json!({"error": "invalid json"}));
    };
    let inputs: Vec<String> = match &v["input"] {
        Value::String(t) => vec![t.clone()],
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        _ => vec![],
    };
    if inputs.is_empty() {
        return respond(
            &mut s,
            "400 Bad Request",
            &json!({"error": "input must be a string or array of strings"}),
        );
    }
    let mut data = vec![];
    for (i, t) in inputs.iter().enumerate() {
        match e.embed(t) {
            Ok(vec) => data.push(json!({"object": "embedding", "index": i, "embedding": vec})),
            Err(err) => {
                return respond(
                    &mut s,
                    "500 Internal Server Error",
                    &json!({"error": err}),
                )
            }
        }
    }
    respond(
        &mut s,
        "200 OK",
        &json!({"object": "list", "model": MODEL_NAME, "data": data}),
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).map(String::as_str).unwrap_or("models");
    let addr = args.get(2).map(String::as_str).unwrap_or("127.0.0.1:11500");
    let embedder = match Embedder::load(Path::new(dir)) {
        Ok(e) => Arc::new(e),
        Err(err) => {
            eprintln!("cannot load model from {dir}: {err}");
            std::process::exit(1);
        }
    };
    let listener = TcpListener::bind(addr).unwrap_or_else(|e| {
        eprintln!("bind {addr}: {e}");
        std::process::exit(1);
    });
    println!("atlas-onnx: http://{addr}/v1/embeddings  model {MODEL_NAME}  dim {DIM}");
    for conn in listener.incoming().flatten() {
        let e = Arc::clone(&embedder);
        std::thread::spawn(move || handle(conn, &e));
    }
}
