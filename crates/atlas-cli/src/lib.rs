//! Atlas MVP server: a small, std-only HTTP/1.1 JSON API over `atlas-engine`.
//!
//! Auth is bearer-key via the engine's `Authenticator`. Keys come from the embedder (the
//! binary reads `ATLAS_API_KEYS`); there is no default credential. One request per connection,
//! hard caps on header size, body size, connections and `k`.
// ponytail: thread-per-connection with a cap and Connection: close; an async/keep-alive server
// only if connection churn is measured to matter.
pub mod api;
pub mod http;

use atlas_core::{Authenticator, KeyScope, KeyVerifier, PermSet, Principal, TenantId};
use atlas_engine::Engine;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const MAX_CONNECTIONS: usize = 128;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Static `tenant -> secret` table. Demo-grade: secrets are held in memory and compared in
/// constant time per entry; a real deployment supplies hashed storage behind `KeyVerifier`.
pub struct StaticKeys(Vec<(TenantId, Vec<u8>)>);

impl StaticKeys {
    /// Parses `tenant=secret,tenant2=secret2`. Short secrets and bad tenant ids are errors.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut out = Vec::new();
        for part in spec.split(',').filter(|p| !p.trim().is_empty()) {
            let (t, s) = part
                .split_once('=')
                .ok_or_else(|| "expected tenant=secret".to_string())?;
            let s = s.trim();
            if s.len() < 8 {
                return Err(format!(
                    "secret for '{}' must be at least 8 bytes",
                    t.trim()
                ));
            }
            let tenant = TenantId::parse(t.trim()).map_err(|e| e.code().to_string())?;
            out.push((tenant, s.as_bytes().to_vec()));
        }
        if out.is_empty() {
            return Err("no keys configured".into());
        }
        Ok(Self(out))
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as usize;
    }
    diff == 0
}

impl KeyVerifier for StaticKeys {
    fn verify(&self, secret: &[u8]) -> Option<(Principal, KeyScope)> {
        let mut found = None;
        for (tenant, key) in &self.0 {
            if ct_eq(key, secret) {
                found = Some(tenant.clone());
            }
        }
        found.map(|tenant| {
            (
                Principal {
                    tenant: tenant.clone(),
                    perms: PermSet::ALL,
                },
                KeyScope {
                    tenant,
                    allowed: PermSet::ALL,
                },
            )
        })
    }
}

pub struct Server {
    pub engine: Arc<Engine>,
    pub auth: Authenticator<StaticKeys>,
}

/// Accept loop. Returns when `stop` is set. The listener is switched to non-blocking.
pub fn serve(server: Arc<Server>, listener: TcpListener, stop: Arc<AtomicBool>) {
    listener.set_nonblocking(true).ok();
    let active = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(false).ok();
                stream.set_read_timeout(Some(IO_TIMEOUT)).ok();
                stream.set_write_timeout(Some(IO_TIMEOUT)).ok();
                if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    active.fetch_sub(1, Ordering::SeqCst);
                    http::respond(&mut stream, 503, "application/json", br#"{"error":"BUSY"}"#);
                    continue;
                }
                let (server, active) = (Arc::clone(&server), Arc::clone(&active));
                std::thread::spawn(move || {
                    http::handle(&server, stream);
                    active.fetch_sub(1, Ordering::SeqCst);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}
