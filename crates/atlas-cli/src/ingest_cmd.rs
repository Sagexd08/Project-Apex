//! `atlas ingest <path>... [--addr HOST:PORT] [--key SECRET]` : read Markdown/text files (recursing
//! into directories) and send them to a running server's `/v1/lcf/ingest` in batches.
//! The key can also come from ATLAS_KEY. std-only HTTP/1.1 client (plain http, local use).
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

const BATCH_DOCS: usize = 100;
const BATCH_BYTES: usize = 4 * 1024 * 1024;
const EXTS: [&str; 3] = ["md", "markdown", "txt"];

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn collect(p: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if p.is_dir() {
        let mut ents: Vec<PathBuf> = std::fs::read_dir(p)
            .map_err(|e| format!("{}: {e}", p.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        ents.sort();
        for e in ents {
            collect(&e, out)?;
        }
    } else if p
        .extension()
        .and_then(|x| x.to_str())
        .map(|x| EXTS.contains(&x.to_lowercase().as_str()))
        .unwrap_or(false)
    {
        out.push(p.to_path_buf());
    }
    Ok(())
}

fn post(addr: &str, key: &str, body: &str) -> Result<Value, String> {
    let mut s = TcpStream::connect(addr).map_err(|e| format!("connect {addr}: {e}"))?;
    let req = format!(
        "POST /v1/lcf/ingest HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {key}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, payload) = text.split_once("\r\n\r\n").ok_or("malformed response")?;
    let v: Value = serde_json::from_str(payload).unwrap_or(json!({"error": "BAD_RESPONSE"}));
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!("{} {}", head.lines().next().unwrap_or(""), v));
    }
    Ok(v)
}

#[derive(Default)]
struct Totals {
    n: [u64; 4], // objects new/unchanged, relationships new/unchanged
    unresolved: Vec<Value>,
    conflicts: Vec<Value>,
    errors: Vec<Value>,
}

fn flush(addr: &str, key: &str, batch: &mut Vec<Value>, t: &mut Totals) -> Result<(), String> {
    if batch.is_empty() {
        return Ok(());
    }
    let r = post(
        addr,
        key,
        &json!({"documents": std::mem::take(batch)}).to_string(),
    )?;
    let keys = [
        "nodes_written",
        "nodes_unchanged",
        "relationships_written",
        "relationships_unchanged",
    ];
    for (i, k) in keys.iter().enumerate() {
        t.n[i] += r[k].as_u64().unwrap_or(0);
    }
    t.unresolved
        .extend(r["unresolved"].as_array().cloned().unwrap_or_default());
    t.conflicts
        .extend(r["conflicts"].as_array().cloned().unwrap_or_default());
    t.errors
        .extend(r["errors"].as_array().cloned().unwrap_or_default());
    Ok(())
}

pub fn run(args: &[String]) -> Result<(), String> {
    let addr = flag(args, "--addr").unwrap_or_else(|| "127.0.0.1:8080".into());
    let key = flag(args, "--key")
        .or_else(|| std::env::var("ATLAS_KEY").ok())
        .ok_or("--key or ATLAS_KEY is required")?;
    let mut paths: Vec<&String> = vec![];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--addr" || a == "--key" {
            it.next();
        } else {
            paths.push(a);
        }
    }
    if paths.is_empty() {
        return Err(
            "usage: atlas ingest <file-or-dir>... [--addr HOST:PORT] [--key SECRET]".into(),
        );
    }
    let mut files = vec![];
    for p in paths {
        collect(Path::new(p), &mut files)?;
    }
    if files.is_empty() {
        return Err("no .md/.markdown/.txt files found".into());
    }
    let (mut t, mut batch, mut bytes) = (Totals::default(), Vec::<Value>::new(), 0usize);
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        bytes += text.len();
        batch.push(json!({"path": f.to_string_lossy().replace('\\', "/"), "text": text}));
        if batch.len() >= BATCH_DOCS || bytes >= BATCH_BYTES {
            flush(&addr, &key, &mut batch, &mut t)?;
            bytes = 0;
        }
    }
    flush(&addr, &key, &mut batch, &mut t)?;
    println!(
        "ingested {} file(s): objects {} new / {} unchanged, relationships {} new / {} unchanged",
        files.len(),
        t.n[0],
        t.n[1],
        t.n[2],
        t.n[3]
    );
    if !t.unresolved.is_empty() {
        println!("{} relationship(s) point at ids that do not exist yet (ingest the targets, then re-run):", t.unresolved.len());
        for u in t.unresolved.iter().take(20) {
            println!(
                "  {} -[{}]-> {}   ({})",
                u["from"], u["rel"], u["to"], u["document"]
            );
        }
    }
    for c in &t.conflicts {
        println!("CONFLICT {c}");
    }
    for e in &t.errors {
        println!("ERROR {e}");
    }
    if t.conflicts.is_empty() && t.errors.is_empty() {
        Ok(())
    } else {
        Err("some documents were not ingested".into())
    }
}
