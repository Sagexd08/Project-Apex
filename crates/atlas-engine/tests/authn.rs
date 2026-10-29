use atlas_core::{
    AtlasError, Authenticator, Config, KeyScope, KeyVerifier, PermSet, Principal, TenantId,
    VectorId,
};
use atlas_engine::Engine;
use serde_json::json;

struct Fake;

fn claims(pt: &str, kt: &str, p: PermSet, k: PermSet) -> (Principal, KeyScope) {
    (
        Principal {
            tenant: TenantId::parse(pt).unwrap(),
            perms: p,
        },
        KeyScope {
            tenant: TenantId::parse(kt).unwrap(),
            allowed: k,
        },
    )
}

impl KeyVerifier for Fake {
    fn verify(&self, secret: &[u8]) -> Option<(Principal, KeyScope)> {
        match secret {
            b"ok" => Some(claims("a", "a", PermSet::ALL, PermSet::ALL)),
            b"ro" => Some(claims("a", "a", PermSet::ALL, PermSet::SEARCH)),
            b"split" => Some(claims("a", "b", PermSet::ALL, PermSet::ALL)),
            _ => None,
        }
    }
}

#[test]
fn engine_enforces_authenticated_context() {
    let dir = std::env::temp_dir().join(format!("atlas-authn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let e = Engine::open(&dir, Config::new(2)).unwrap();
    let auth = Authenticator::new(Fake);
    let scope = |s: &[u8]| auth.authenticate(s).unwrap().effective();

    e.insert(&scope(b"ok"), VectorId(1), &[1.0, 0.0], json!(null))
        .unwrap();
    let ro = scope(b"ro");
    assert_eq!(
        e.search(&ro, &[1.0, 0.0], 5, atlas_core::Metric::Dot)
            .unwrap()
            .len(),
        1
    );
    let w = e.insert(&ro, VectorId(2), &[1.0, 0.0], json!(null));
    assert_eq!(w.unwrap_err().code(), "FORBIDDEN");
    assert_eq!(e.fetch(&ro, VectorId(1)).unwrap_err().code(), "FORBIDDEN");
    assert!(matches!(
        auth.authenticate(b"split"),
        Err(AtlasError::Forbidden)
    ));
    assert!(matches!(
        auth.authenticate(b"nope"),
        Err(AtlasError::Forbidden)
    ));
}
