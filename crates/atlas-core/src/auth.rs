//! Tenant identity, permissions and the scope every engine call is made under.
//!
//! `EffectiveScope` and `AuthContext` have private fields and no public constructor (the
//! `test-auth` feature adds a test-only one), so code outside this crate cannot mint a scope.
//!
//! ```
//! use atlas_core::{EffectiveScope, PermSet};
//! // Default is the empty scope: no tenant, no permissions.
//! assert!(EffectiveScope::default().tenant().is_none());
//! assert!(!EffectiveScope::default().allows(PermSet::SEARCH));
//! ```
//!
//! Struct-literal construction is rejected (private fields):
//! ```compile_fail,E0451
//! use atlas_core::{EffectiveScope, PermSet};
//! let _ = EffectiveScope { tenant: None, perms: PermSet::ALL };
//! ```
//!
//! The crate-private constructor is not callable from outside:
//! ```compile_fail,E0624
//! use atlas_core::{EffectiveScope, PermSet, TenantId};
//! let t = TenantId::parse("a").unwrap();
//! let _ = EffectiveScope::new(t, PermSet::ALL);
//! ```
//!
//! `AuthContext` cannot be built by a literal either:
//! ```compile_fail,E0451
//! use atlas_core::{AuthContext, KeyScope, Principal, PermSet, TenantId};
//! let t = TenantId::parse("a").unwrap();
//! let _ = AuthContext {
//!     principal: Principal { tenant: t.clone(), perms: PermSet::ALL },
//!     key: KeyScope { tenant: t, allowed: PermSet::ALL },
//! };
//! ```
//!
//! `AuthContext` is not `Clone`:
//! ```compile_fail,E0277
//! fn needs_clone<T: Clone>() {}
//! needs_clone::<atlas_core::AuthContext>();
//! ```
//!
//! `AuthContext` is not `Serialize`:
//! ```compile_fail,E0277
//! fn needs_ser<T: serde::Serialize>() {}
//! needs_ser::<atlas_core::AuthContext>();
//! ```
//!
//! `AuthContext` is not `Deserialize`:
//! ```compile_fail,E0277
//! fn needs_de<T: for<'de> serde::Deserialize<'de>>() {}
//! needs_de::<atlas_core::AuthContext>();
//! ```
use crate::AtlasError;
use std::ops::{BitAnd, BitOr};

/// Max characters of untrusted input echoed in an error message.
pub const MAX_ECHO_CHARS: usize = 64;

/// Debug-quote untrusted input for an error message, cut to `MAX_ECHO_CHARS` characters
/// (char-boundary safe) with an explicit marker when cut.
pub fn clip_echo(s: &str) -> String {
    let mut it = s.chars();
    let head: String = it.by_ref().take(MAX_ECHO_CHARS).collect();
    if it.next().is_some() {
        format!("{head:?} ...(truncated)")
    } else {
        format!("{head:?}")
    }
}

/// Max characters of an upstream error text kept in `Incomplete.reason` / the poison message.
pub const MAX_TEXT_CHARS: usize = 256;

/// Cut `s` to `MAX_TEXT_CHARS` characters (char-boundary safe), marking the cut.
pub fn clip_text(s: &str) -> String {
    let mut it = s.chars();
    let head: String = it.by_ref().take(MAX_TEXT_CHARS).collect();
    if it.next().is_some() {
        format!("{head}...(truncated)")
    } else {
        head
    }
}

/// Validated tenant id: 1..=64 bytes of `[A-Za-z0-9_-]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TenantId(String);

impl TenantId {
    pub const MAX_LEN: usize = 64;

    pub fn parse(s: &str) -> Result<Self, AtlasError> {
        let ok = !s.is_empty()
            && s.len() <= Self::MAX_LEN
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if ok {
            Ok(Self(s.to_string()))
        } else {
            Err(AtlasError::InvalidTenant(clip_echo(s)))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Set of permissions (bit flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PermSet(u8);

impl PermSet {
    pub const EMPTY: PermSet = PermSet(0);
    pub const SEARCH: PermSet = PermSet(1);
    pub const FETCH: PermSet = PermSet(1 << 1);
    pub const COUNT: PermSet = PermSet(1 << 2);
    pub const WRITE: PermSet = PermSet(1 << 3);
    pub const DELETE: PermSet = PermSet(1 << 4);
    pub const SNAPSHOT: PermSet = PermSet(1 << 5);
    pub const EXPORT: PermSet = PermSet(1 << 6);
    pub const ALL: PermSet = PermSet(0x7f);

    /// True if every permission in `other` is in `self`.
    pub fn contains(self, other: PermSet) -> bool {
        self.0 & other.0 == other.0
    }

    /// `self` minus the permissions in `other`.
    pub fn without(self, other: PermSet) -> PermSet {
        PermSet(self.0 & !other.0)
    }
}

impl BitAnd for PermSet {
    type Output = PermSet;
    fn bitand(self, rhs: PermSet) -> PermSet {
        PermSet(self.0 & rhs.0)
    }
}

impl BitOr for PermSet {
    type Output = PermSet;
    fn bitor(self, rhs: PermSet) -> PermSet {
        PermSet(self.0 | rhs.0)
    }
}

/// What the caller is authorised to do, as proven by the authenticator.
#[derive(Debug, Clone)]
pub struct Principal {
    pub tenant: TenantId,
    pub perms: PermSet,
}

/// What the presented key may be used for.
#[derive(Debug, Clone)]
pub struct KeyScope {
    pub tenant: TenantId,
    pub allowed: PermSet,
}

/// The tenant and permissions one engine call runs under. Default = empty (denies everything).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveScope {
    tenant: Option<TenantId>,
    perms: PermSet,
}

impl EffectiveScope {
    pub(crate) fn new(tenant: TenantId, perms: PermSet) -> Self {
        Self {
            tenant: Some(tenant),
            perms,
        }
    }

    pub fn tenant(&self) -> Option<&TenantId> {
        self.tenant.as_ref()
    }

    pub fn perms(&self) -> PermSet {
        self.perms
    }

    /// True if the scope has a tenant and holds every permission in `p`.
    pub fn allows(&self, p: PermSet) -> bool {
        self.tenant.is_some() && self.perms.contains(p)
    }
}

/// Authenticated caller. Built only by the auth module (`test-auth` adds a test constructor).
/// Deliberately not `Clone`, `Serialize` or `Deserialize`.
#[derive(Debug)]
pub struct AuthContext {
    principal: Principal,
    key: KeyScope,
}

impl AuthContext {
    /// Test-only constructor.
    #[cfg(any(test, feature = "test-auth"))]
    pub fn for_test(principal: Principal, key: KeyScope) -> Self {
        Self { principal, key }
    }

    /// Empty scope if principal.tenant != key.tenant, else principal.perms & key.allowed.
    pub fn effective(&self) -> EffectiveScope {
        if self.principal.tenant != self.key.tenant {
            return EffectiveScope::default();
        }
        EffectiveScope::new(
            self.principal.tenant.clone(),
            self.principal.perms & self.key.allowed,
        )
    }
}

/// Embedder-supplied credential check. Key storage and hashing live behind this trait and are
/// out of scope for Atlas. Returns the principal and key claims for a valid secret, else `None`.
pub trait KeyVerifier {
    fn verify(&self, secret: &[u8]) -> Option<(Principal, KeyScope)>;
}

/// The only public way to obtain an `AuthContext`.
pub struct Authenticator<V: KeyVerifier> {
    verifier: V,
}

impl<V: KeyVerifier> Authenticator<V> {
    pub fn new(verifier: V) -> Self {
        Self { verifier }
    }

    /// `Forbidden` if the verifier rejects the secret or the principal and key tenants differ.
    pub fn authenticate(&self, secret: &[u8]) -> Result<AuthContext, AtlasError> {
        let (principal, key) = self.verifier.verify(secret).ok_or(AtlasError::Forbidden)?;
        if principal.tenant != key.tenant {
            return Err(AtlasError::Forbidden);
        }
        Ok(AuthContext { principal, key })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_id_validation() {
        for ok in ["a", "A-z_9", &"x".repeat(64)] {
            assert_eq!(TenantId::parse(ok).unwrap().as_str(), ok);
        }
        for bad in [
            "",
            "a b",
            "a.b",
            "a/b",
            "../x",
            "\u{e9}",
            "a\0",
            &"x".repeat(65),
        ] {
            let e = TenantId::parse(bad).unwrap_err();
            assert_eq!(e.code(), "INVALID_TENANT", "{bad:?}");
        }
    }

    struct Fixed(Option<(Principal, KeyScope)>);
    impl KeyVerifier for Fixed {
        fn verify(&self, secret: &[u8]) -> Option<(Principal, KeyScope)> {
            if secret == b"good" {
                self.0.clone()
            } else {
                None
            }
        }
    }

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

    #[test]
    fn authenticator_builds_intersection_or_rejects() {
        let ok = Authenticator::new(Fixed(Some(claims(
            "a",
            "a",
            PermSet::SEARCH | PermSet::WRITE,
            PermSet::SEARCH,
        ))));
        let s = ok.authenticate(b"good").unwrap().effective();
        assert_eq!(s.perms(), PermSet::SEARCH);
        assert_eq!(s.tenant().unwrap().as_str(), "a");
        assert_eq!(ok.authenticate(b"bad").unwrap_err().code(), "FORBIDDEN");
    }

    #[test]
    fn authenticate_rejects_principal_key_tenant_mismatch() {
        let mismatch =
            Authenticator::new(Fixed(Some(claims("a", "b", PermSet::ALL, PermSet::ALL))));
        assert_eq!(
            mismatch.authenticate(b"good").unwrap_err().code(),
            "FORBIDDEN"
        );
    }

    #[test]
    fn neither_claim_alone_widens() {
        for (p, k) in [
            (PermSet::ALL, PermSet::SEARCH),
            (PermSet::SEARCH, PermSet::ALL),
        ] {
            let a = Authenticator::new(Fixed(Some(claims("a", "a", p, k))));
            let s = a.authenticate(b"good").unwrap().effective();
            assert_eq!(s.perms(), PermSet::SEARCH);
            assert!(!s.allows(PermSet::WRITE) && !s.allows(PermSet::EXPORT));
        }
        let a = Authenticator::new(Fixed(Some(claims("a", "a", PermSet::ALL, PermSet::EMPTY))));
        assert_eq!(
            a.authenticate(b"good").unwrap().effective().perms(),
            PermSet::EMPTY
        );
    }

    #[test]
    fn effective_mismatch_is_still_empty_defense_in_depth() {
        let (principal, key) = claims("a", "b", PermSet::ALL, PermSet::ALL);
        let ctx = AuthContext::for_test(principal, key);
        assert_eq!(ctx.effective(), EffectiveScope::default());
    }

    #[test]
    fn echo_clip_is_bounded_and_char_safe() {
        assert_eq!(clip_echo("abc"), "\"abc\"");
        let long = "\u{20ac}".repeat(1000);
        let c = clip_echo(&long);
        assert!(c.contains("truncated") && c.len() < 1024, "{}", c.len());
    }

    #[test]
    fn permset_ops() {
        let rw = PermSet::SEARCH | PermSet::WRITE;
        assert!(rw.contains(PermSet::SEARCH) && !rw.contains(PermSet::FETCH));
        assert_eq!(rw & PermSet::WRITE, PermSet::WRITE);
        assert_eq!(rw.without(PermSet::WRITE), PermSet::SEARCH);
        assert!(PermSet::ALL.contains(PermSet::EXPORT));
        assert_eq!(PermSet::default(), PermSet::EMPTY);
    }
}
