//! Embedding providers. The default is deterministic and offline (hashed bag-of-words, a lexical
//! proxy, not a semantic model). A real model is plugged in over plain HTTP (OpenAI-compatible
//! `/v1/embeddings`, e.g. a local Ollama, llama.cpp server or an internal gateway). There is no
//! silent fallback: if the provider fails, the write or query fails with UNAVAILABLE.
use crate::model::{fnv, tokens, DIM};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub trait Embedder: Send + Sync {
    fn name(&self) -> String;
    fn dim(&self) -> usize;
    fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
    fn strong_similarity(&self) -> f32 {
        0.75
    }
}

pub struct HashEmbedder;

impl Embedder for HashEmbedder {
    fn name(&self) -> String {
        format!("hash-bow-{DIM}")
    }
    fn dim(&self) -> usize {
        DIM
    }
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let mut v = vec![0f32; DIM];
        for t in tokens(text) {
            v[(fnv(&t) % DIM as u64) as usize] += 1.0;
        }
        normalize(&mut v);
        Ok(v)
    }
}

pub fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n == 0.0 {
        v[0] = 1.0;
    } else {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

/// Plain-HTTP OpenAI-compatible embedding client (no TLS: put a local proxy in front for https).
pub struct HttpEmbedder {
    addr: String,
    host: String,
    path: String,
    model: String,
    dim: usize,
    strong: f32,
}

impl HttpEmbedder {
    /// `url` like `http://127.0.0.1:11434/v1/embeddings`. `dim` must equal the engine `--dim`.
    pub fn new(url: &str, model: &str, dim: usize) -> Result<Self, String> {
        let rest = url
            .strip_prefix("http://")
            .ok_or("only http:// URLs are supported (use a local proxy for https)")?;
        let (hostport, path) = rest
            .split_once('/')
            .map(|(h, p)| (h, format!("/{p}")))
            .unwrap_or((rest, "/".into()));
        let addr = if hostport.contains(':') {
            hostport.to_string()
        } else {
            format!("{hostport}:80")
        };
        Ok(HttpEmbedder {
            addr,
            host: hostport.to_string(),
            path,
            model: model.into(),
            dim,
            strong: std::env::var("ATLAS_EMBED_STRONG")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.35),
        })
    }
}

impl Embedder for HttpEmbedder {
    fn name(&self) -> String {
        format!("http:{}:{}:{}", self.addr, self.model, self.dim)
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn strong_similarity(&self) -> f32 {
        self.strong
    }
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let body = json!({"model": self.model, "input": text}).to_string();
        let sock = self
            .addr
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
            .next()
            .ok_or("no address")?;
        let mut s = TcpStream::connect_timeout(&sock, Duration::from_secs(5))
            .map_err(|e| format!("connect: {e}"))?;
        s.set_read_timeout(Some(Duration::from_secs(30))).ok();
        s.set_write_timeout(Some(Duration::from_secs(10))).ok();
        let req = format!("POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", self.path, self.host, body.len(), body);
        s.write_all(req.as_bytes())
            .map_err(|e| format!("write: {e}"))?;
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).map_err(|e| format!("read: {e}"))?;
        let text = String::from_utf8_lossy(&raw);
        let (head, payload) = text.split_once("\r\n\r\n").ok_or("malformed response")?;
        if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
            return Err(format!(
                "provider status: {}",
                head.lines().next().unwrap_or("")
            ));
        }
        let v: Value = serde_json::from_str(payload).map_err(|e| format!("provider json: {e}"))?;
        let arr = v
            .pointer("/data/0/embedding")
            .and_then(Value::as_array)
            .ok_or("no data[0].embedding")?;
        let mut out: Vec<f32> = arr
            .iter()
            .filter_map(|x| x.as_f64().map(|f| f as f32))
            .collect();
        if out.len() != self.dim {
            return Err(format!(
                "provider returned dim {} but --dim is {}",
                out.len(),
                self.dim
            ));
        }
        normalize(&mut out);
        Ok(out)
    }
}
