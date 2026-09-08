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

/// Read a variable as UTF-8. The policy answers: inject a reader.
pub fn read_var() {
    let _ = std::env::var("FEMTOLOGGING_PROBE");
}

/// Read a variable as an `OsString`. Banned for the same reason as
/// `read_var`; the `OsString` form is not an exemption.
pub fn read_var_os() {
    let _ = std::env::var_os("FEMTOLOGGING_PROBE");
}

/// Enumerate the whole environment as UTF-8 pairs.
pub fn read_vars() {
    let _ = std::env::vars().count();
}

/// Enumerate the whole environment as `OsString` pairs.
pub fn read_vars_os() {
    let _ = std::env::vars_os().count();
}

/// Set a variable in the parent process. This is the call that forces a
/// test suite to serialize, so the policy answers: use a stub environment.
pub fn write_var() {
    unsafe { std::env::set_var("FEMTOLOGGING_PROBE", "1") };
}

/// Remove a variable from the parent process, with the same consequence
/// for parallelism as `write_var`.
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
