mod demo;
mod ingest_cmd;

use atlas_cli::{serve, Server, StaticKeys};
use atlas_core::{Authenticator, Config};
use atlas_engine::Engine;
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const USAGE: &str = "usage:\n  atlas demo\n  atlas serve --dir <path> [--dim <n>] [--addr 127.0.0.1:8080] [--seed]
  (--seed writes a small SAMPLE forest for trying the UI; real data comes from `atlas ingest`. Default --dim is 64, the built-in embedder size; ATLAS_EMBED_URL plugs in a real embedding model and then needs --dim)\n\
serve needs ATLAS_API_KEYS=tenant=secret[,tenant2=secret2] (secrets >= 8 bytes); no default key exists.";

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn run_serve(args: &[String]) -> Result<(), String> {
    let dir = flag(args, "--dir").ok_or("--dir is required")?;
    let dim: usize = match flag(args, "--dim") {
        Some(d) => d.parse().map_err(|_| "--dim must be a positive integer")?,
        None => atlas_lcf::model::DIM,
    };
    let addr = flag(args, "--addr").unwrap_or_else(|| "127.0.0.1:8080".into());
    let keys = std::env::var("ATLAS_API_KEYS").map_err(|_| "ATLAS_API_KEYS is not set")?;
    let first_secret = keys
        .split(',')
        .next()
        .and_then(|p| p.split_once('='))
        .map(|(_, s)| s.trim().to_string())
        .unwrap_or_default();
    let keys = StaticKeys::parse(&keys)?;
    let embedder: std::sync::Arc<dyn atlas_lcf::embed::Embedder> =
        match std::env::var("ATLAS_EMBED_URL") {
            Ok(url) => {
                if flag(args, "--dim").is_none() {
                    return Err("ATLAS_EMBED_URL needs --dim <embedding size>".into());
                }
                let model = std::env::var("ATLAS_EMBED_MODEL").unwrap_or_default();
                std::sync::Arc::new(atlas_lcf::embed::HttpEmbedder::new(&url, &model, dim)?)
            }
            Err(_) => std::sync::Arc::new(atlas_lcf::embed::HashEmbedder),
        };
    if embedder.dim() != dim {
        return Err(format!(
            "embedder dimension {} does not match --dim {dim}",
            embedder.dim()
        ));
    }
    println!("embedder: {}", embedder.name());
    // ATLAS_INDEX=hnsw opts in to the approximate graph index (exact scan stays the default).
    let engine = match std::env::var("ATLAS_INDEX").as_deref() {
        Ok("hnsw") => {
            println!("index: hnsw (approximate; recall measured in docs/MVP_STATUS.md)");
            Engine::open_with_index(&dir, Config::new(dim), atlas_index::HnswIndex::factory())
        }
        _ => {
            println!("index: exact scan");
            Engine::open(&dir, Config::new(dim))
        }
    }
    .map_err(|e| e.to_string())?;
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
    println!("atlas listening on http://{addr} (dim {dim}, dir {dir})");
    let server = Arc::new(Server {
        engine: Arc::new(engine),
        auth: Authenticator::new(keys),
        lcf: atlas_lcf::LcfState::new(embedder),
        throttle: Default::default(),
    });
    if args.iter().any(|a| a == "--seed") {
        let ctx = server
            .auth
            .authenticate(first_secret.as_bytes())
            .map_err(|e| e.code().to_string())?;
        let lcf = atlas_lcf::Lcf::new(&server.engine, &server.lcf);
        let (n, e) = atlas_lcf::seed::seed_demo(&lcf, &ctx.effective())
            .map_err(|e| format!("seed failed: {} {}", e.code(), e.detail()))?;
        println!("seeded demo forest: {n} nodes, {e} edges (idempotent)");
    }
    println!("explorer: http://{addr}/explorer");
    println!("playground: http://{addr}/playground");
    let stop = Arc::new(AtomicBool::new(false));
    atlas_cli::checkpointer::spawn(server.engine.clone(), stop.clone());
    serve(server, listener, stop);
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("demo") => demo::run(),
        Some("ingest") => ingest_cmd::run(&args[1..]),
        Some("serve") => run_serve(&args[1..]),
        _ => Err(USAGE.to_string()),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(2);
    }
}
