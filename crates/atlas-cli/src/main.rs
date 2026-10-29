mod demo;

use atlas_cli::{serve, Server, StaticKeys};
use atlas_core::{Authenticator, Config};
use atlas_engine::Engine;
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const USAGE: &str = "usage:\n  atlas demo\n  atlas serve --dir <path> --dim <n> [--addr 127.0.0.1:8080]\n\
serve needs ATLAS_API_KEYS=tenant=secret[,tenant2=secret2] (secrets >= 8 bytes); no default key exists.";

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn run_serve(args: &[String]) -> Result<(), String> {
    let dir = flag(args, "--dir").ok_or("--dir is required")?;
    let dim: usize = flag(args, "--dim")
        .ok_or("--dim is required")?
        .parse()
        .map_err(|_| "--dim must be a positive integer")?;
    let addr = flag(args, "--addr").unwrap_or_else(|| "127.0.0.1:8080".into());
    let keys = std::env::var("ATLAS_API_KEYS").map_err(|_| "ATLAS_API_KEYS is not set")?;
    let keys = StaticKeys::parse(&keys)?;
    let engine = Engine::open(&dir, Config::new(dim)).map_err(|e| e.to_string())?;
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
    println!("atlas listening on http://{addr} (dim {dim}, dir {dir})");
    let server = Arc::new(Server {
        engine: Arc::new(engine),
        auth: Authenticator::new(keys),
    });
    serve(server, listener, Arc::new(AtomicBool::new(false)));
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("demo") => demo::run(),
        Some("serve") => run_serve(&args[1..]),
        _ => Err(USAGE.to_string()),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(2);
    }
}
