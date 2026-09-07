//! Shared machinery for the environment-access policy contract.
//!
//! `tests/env_access_policy.rs` holds the assertions; this module holds what
//! they share, split so that each file stays inside the 400-line module limit
//! Whitaker enforces.
//!
//! - [`config`] reads the three checked-in configuration files.
//! - [`makefile`] queries and judges the lint recipes.
//! - [`workflow`] proves CI runs those recipes at all.
//!
//! The configuration files are embedded with `include_str!`, which resolves
//! relative to the source file at compile time. That reaches the `Makefile`
//! above `CARGO_MANIFEST_DIR` without any runtime filesystem access, so
//! Whitaker's `no_std_fs_operations` never fires and the crate needs no
//! `dylint.toml` exclusion. Moving or deleting one of the three files becomes
//! a compile error rather than a runtime failure.

use std::error::Error;

// Paths are relative to this file's own directory, `tests/test_utils/`.
#[path = "env_policy/config.rs"]
pub(crate) mod config;
#[path = "env_policy/makefile.rs"]
pub(crate) mod makefile;
#[path = "env_policy/workflow.rs"]
pub(crate) mod workflow;

pub(crate) type TestResult = Result<(), Box<dyn Error>>;
pub(crate) type Fallible<T> = Result<T, Box<dyn Error>>;

/// The Clippy lint that carries the ban.
pub(crate) const DISALLOWED_METHODS_LINT: &str = "clippy::disallowed_methods";

/// Environment methods the project refuses to call ambiently, each paired with
/// the remedy Clippy must print when a contributor trips the lint.
pub(crate) const REQUIRED_DISALLOWED_METHODS: [(&str, &str); 6] = [
    ("std::env::var", "inject an environment reader"),
    ("std::env::var_os", "inject an environment reader"),
    ("std::env::vars", "inject an environment reader"),
    ("std::env::vars_os", "inject an environment reader"),
    ("std::env::set_var", "use a stub environment in tests"),
    ("std::env::remove_var", "use a stub environment in tests"),
];

pub(crate) use config::{
    CRATE_MANIFEST, configured_severity, disallowed_methods, probe_violations,
};
pub(crate) use makefile::{
    Makefile, ensure_flags_deny_the_policy, ensure_lanes_cover_every_feature,
    ensure_lint_reaches_the_policy_lane, policy_lane_run_succeeds,
};
pub(crate) use workflow::ensure_ci_runs_the_policy_gates;
