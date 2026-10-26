//! One probe per test-only API, each behind its own `use_<name>` feature (see Cargo.toml).
#![allow(unused_imports)]
use atlas_core::{AuthContext, KeyScope, PermSet, Principal, TenantId};
use atlas_engine::Engine;

#[cfg(feature = "use_fail_next_flush_handle")]
pub fn probe_fail_next_flush_handle(e: &Engine) {
    let _ = e.fail_next_flush_handle();
}

#[cfg(feature = "use_fail_next_truncate_handle")]
pub fn probe_fail_next_truncate_handle(e: &Engine) {
    let _ = e.fail_next_truncate_handle();
}

#[cfg(feature = "use_fail_next_zero_handle")]
pub fn probe_fail_next_zero_handle(e: &Engine) {
    let _ = e.fail_next_zero_handle();
}

#[cfg(feature = "use_partial_write_handle")]
pub fn probe_partial_write_handle(e: &Engine) {
    let _ = e.partial_write_handle();
}

#[cfg(feature = "use_fail_next_marker_handle")]
pub fn probe_fail_next_marker_handle(e: &Engine) {
    let _ = e.fail_next_marker_handle();
}

#[cfg(feature = "use_die_after_marker_handle")]
pub fn probe_die_after_marker_handle(e: &Engine) {
    let _ = e.die_after_marker_handle();
}

#[cfg(feature = "use_flush_delay_handle")]
pub fn probe_flush_delay_handle(e: &Engine) {
    let _ = e.flush_delay_handle();
}

#[cfg(feature = "use_set_before_gate_hook")]
pub fn probe_set_before_gate_hook(e: &Engine) {
    e.set_before_gate_hook(|| {});
}

#[cfg(feature = "use_open_with_fingerprint")]
pub fn probe_open_with_fingerprint() {
    let _ = Engine::open_with_fingerprint("x", atlas_core::Config::new(2), |_| 0);
}

#[cfg(feature = "use_len")]
pub fn probe_len(e: &Engine) {
    let _ = e.len();
}

#[cfg(feature = "use_is_empty")]
pub fn probe_is_empty(e: &Engine) {
    let _ = e.is_empty();
}

#[cfg(feature = "use_for_test")]
pub fn probe_for_test() {
    let t = TenantId::parse("a").unwrap();
    let _ = AuthContext::for_test(
        Principal { tenant: t.clone(), perms: PermSet::ALL },
        KeyScope { tenant: t, allowed: PermSet::ALL },
    );
}

#[cfg(feature = "use_wal_fail_next_flush_handle")]
pub fn probe_wal_fail_next_flush_handle(w: &atlas_storage::Wal) {
    let _ = w.fail_next_flush_handle();
}

#[cfg(feature = "use_wal_fail_next_truncate_handle")]
pub fn probe_wal_fail_next_truncate_handle(w: &atlas_storage::Wal) {
    let _ = w.fail_next_truncate_handle();
}

#[cfg(feature = "use_wal_fail_next_zero_handle")]
pub fn probe_wal_fail_next_zero_handle(w: &atlas_storage::Wal) {
    let _ = w.fail_next_zero_handle();
}

#[cfg(feature = "use_wal_partial_write_handle")]
pub fn probe_wal_partial_write_handle(w: &atlas_storage::Wal) {
    let _ = w.partial_write_handle();
}

#[cfg(feature = "use_wal_fail_next_marker_handle")]
pub fn probe_wal_fail_next_marker_handle(w: &atlas_storage::Wal) {
    let _ = w.fail_next_marker_handle();
}

#[cfg(feature = "use_wal_die_after_marker_handle")]
pub fn probe_wal_die_after_marker_handle(w: &atlas_storage::Wal) {
    let _ = w.die_after_marker_handle();
}

#[cfg(feature = "use_wal_flush_delay_handle")]
pub fn probe_wal_flush_delay_handle(w: &atlas_storage::Wal) {
    let _ = w.flush_delay_handle();
}

#[cfg(feature = "use_commit_hold_handle")]
pub fn probe_commit_hold_handle(e: &Engine) {
    let _ = e.commit_hold_handle();
}

#[cfg(feature = "use_poison_detail")]
pub fn probe_poison_detail(e: &Engine) {
    let _ = e.poison_detail();
}

#[cfg(feature = "use_poison_layer_counts")]
pub fn probe_poison_layer_counts(e: &Engine) {
    let _ = e.poison_layer_counts();
}

#[cfg(feature = "use_bypass_engine_poison_check")]
pub fn probe_bypass_engine_poison_check(e: &Engine) {
    e.bypass_engine_poison_check(true);
}

#[cfg(feature = "use_wal_commit_hold_handle")]
pub fn probe_wal_commit_hold_handle(w: &atlas_storage::Wal) {
    let _ = w.commit_hold_handle();
}
