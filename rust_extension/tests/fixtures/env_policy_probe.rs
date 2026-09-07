//! Fixture compiled by `tests/env_access_policy.rs` to prove the Clippy
//! environment policy rejects each banned method and honours the sanctioned
//! composition-root escape hatch.
//!
//! This file is not a crate target. It lives under `tests/fixtures/`, which
//! Cargo does not auto-discover, and is compiled only by `clippy-driver` from
//! the contract test. Every call below is deliberate.
//!
//! The functions are public so that compiling this as a library emits no
//! `dead_code` diagnostics, leaving the six `clippy::disallowed_methods`
//! errors as the only output the contract test has to reason about.

pub fn read_var() {
    let _ = std::env::var("FEMTOLOGGING_PROBE");
}

pub fn read_var_os() {
    let _ = std::env::var_os("FEMTOLOGGING_PROBE");
}

pub fn read_vars() {
    let _ = std::env::vars().count();
}

pub fn read_vars_os() {
    let _ = std::env::vars_os().count();
}

pub fn write_var() {
    unsafe { std::env::set_var("FEMTOLOGGING_PROBE", "1") };
}

pub fn clear_var() {
    unsafe { std::env::remove_var("FEMTOLOGGING_PROBE") };
}

/// The one sanctioned shape: an item-scoped expectation at a composition root.
#[expect(
    clippy::disallowed_methods,
    reason = "composition root: fixture for the sanctioned escape hatch"
)]
pub fn composition_root() {
    let _ = std::env::var("FEMTOLOGGING_PROBE");
}
