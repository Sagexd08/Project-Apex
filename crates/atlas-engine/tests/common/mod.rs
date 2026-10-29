#![allow(dead_code)]
use atlas_core::{AuthContext, EffectiveScope, KeyScope, PermSet, Principal, TenantId};

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

pub fn single_tenant_full() -> EffectiveScope {
    full("legacy")
}
