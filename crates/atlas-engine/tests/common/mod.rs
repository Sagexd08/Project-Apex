#![allow(dead_code)]
use atlas_core::{AuthContext, EffectiveScope, KeyScope, PermSet, Principal, TenantId};

/// Scope for `tenant` holding exactly `perms` (principal and key both grant them).
pub fn scope(tenant: &str, perms: PermSet) -> EffectiveScope {
    let t = TenantId::parse(tenant).unwrap();
    AuthContext::for_test(
        Principal {
            tenant: t.clone(),
            perms,
        },
        KeyScope {
            tenant: t,
            allowed: PermSet::ALL,
        },
    )
    .effective()
}

pub fn full(tenant: &str) -> EffectiveScope {
    scope(tenant, PermSet::ALL)
}

/// TEST-ONLY full-permission scope for one tenant ("legacy"); used by the single-tenant pre-security tests. Exists only in test code, not in any library.
pub fn single_tenant_full() -> EffectiveScope {
    full("legacy")
}
