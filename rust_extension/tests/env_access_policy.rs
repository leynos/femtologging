//! Contract coverage for the environment-access lint policy, Rust side.
//!
//! Three things here are facts about Rust rather than about the build: the
//! banned `std::env` paths in `clippy.toml`, the `deny` severity in the
//! manifest, and the shape of the one sanctioned exception. Losing any of them
//! silently restores ambient environment access.
//!
//! Two of the tests execute rather than inspect. One runs `clippy-driver` over
//! `tests/fixtures/env_policy_probe.rs` under this crate's `clippy.toml` and
//! counts the diagnostics; the other reads the fixture's source, because
//! counting cannot tell an item-scoped expectation from one written inside the
//! body.
//!
//! The Makefile and workflow half of the policy lives in
//! `tests/test_env_access_policy_contract.py`, which uses the repository's
//! pinned `makeutil` parser and PyYAML rather than a second parser written
//! here. That file also holds the source scan for `allow` attributes that
//! would switch the policy off wholesale.
//!
//! See `docs/adr-006-environment-seam-taxonomy.md` for the policy itself.

#[path = "test_utils/env_policy.rs"]
mod env_policy;

use env_policy::{
    CRATE_MANIFEST, REQUIRED_DISALLOWED_METHODS, TestResult, configured_severity,
    disallowed_methods, ensure_composition_root_is_item_scoped, probe_violations,
};

/// Scenario: a contributor edits `clippy.toml`.
///
/// Invariant: all six `std::env` paths stay banned, each carrying the remedy
/// the diagnostic is expected to print.
///
/// Mutation proof (2026-09-06): deleting the `std::env::set_var` entry from
/// `rust_extension/clippy.toml` failed this test with
/// "clippy.toml must disallow std::env::set_var".
#[test]
fn clippy_policy_disallows_every_ambient_environment_method() -> TestResult {
    let methods = disallowed_methods()?;
    for (required_path, required_reason) in REQUIRED_DISALLOWED_METHODS {
        let entry = methods.iter().find(|(path, _)| path == required_path);
        let Some((_, reason)) = entry else {
            return Err(
                format!("clippy.toml must disallow {required_path}, found {methods:?}").into(),
            );
        };
        if reason != required_reason {
            return Err(format!(
                "clippy.toml must tell a contributor to {required_reason:?} for \
                 {required_path}, found {reason:?}"
            )
            .into());
        }
    }
    Ok(())
}

/// Scenario: a contributor edits the crate manifest's lint tables.
///
/// Invariant: `clippy::disallowed_methods` stays denied, so the banned paths
/// stop a build rather than adding a warning nobody reads.
///
/// Mutation proof (2026-09-06): changing the manifest entry to
/// `disallowed_methods = "warn"` failed this test with
/// "disallowed_methods must be denied ... found Some(\"warn\")".
#[test]
fn manifest_denies_disallowed_methods() -> TestResult {
    let manifest = CRATE_MANIFEST.parse()?;
    let lints = manifest
        .get("lints")
        .and_then(|lints| lints.get("clippy"))
        .ok_or_else(|| format!("{} must declare [lints.clippy]", CRATE_MANIFEST.name))?;
    match configured_severity(lints) {
        Some("deny" | "forbid") => Ok(()),
        other => Err(format!(
            "disallowed_methods must be denied in {}, found {other:?}",
            CRATE_MANIFEST.name
        )
        .into()),
    }
}

/// Scenario: Clippy compiles code that calls each banned method, and code that
/// calls one from a sanctioned composition root.
///
/// Invariant: every banned method is rejected under this crate's `clippy.toml`
/// and the diagnostic carries the remedy the policy promises, so a contributor
/// who trips the lint is told what to do instead. The item-scoped
/// `#[expect(clippy::disallowed_methods, reason = "...")]` escape hatch
/// suppresses exactly one call and nothing more; the count is what proves the
/// hatch is honoured rather than merely tolerated.
///
/// Mutation proof (2026-09-07): removing the `#[expect]` attribute from the
/// fixture's `composition_root` raised the count to seven and failed with "the
/// composition-root expect must suppress exactly one call"; deleting the
/// `std::env::vars_os` entry from `clippy.toml` dropped the count to five and
/// failed with "Clippy must reject std::env::vars_os"; and changing that
/// entry's reason string failed with "Clippy must print the remedy".
#[test]
fn clippy_rejects_every_banned_method_in_a_compiled_fixture() -> TestResult {
    let violations = probe_violations()?;
    for (banned, remedy) in REQUIRED_DISALLOWED_METHODS {
        let Some(violation) = violations.iter().find(|found| found.method == banned) else {
            return Err(format!(
                "Clippy must reject {banned} in the fixture, found {violations:?}"
            )
            .into());
        };
        if !violation.notes.iter().any(|note| note == remedy) {
            return Err(format!(
                "Clippy must print the remedy {remedy:?} for {banned}, found {:?}",
                violation.notes
            )
            .into());
        }
    }
    if violations.len() != REQUIRED_DISALLOWED_METHODS.len() {
        return Err(format!(
            "the composition-root expect must suppress exactly one call, leaving {} \
             violations; found {violations:?}",
            REQUIRED_DISALLOWED_METHODS.len()
        )
        .into());
    }
    Ok(())
}

/// Scenario: a contributor edits the fixture's sanctioned composition root.
///
/// Invariant: the exception keeps its shape. The direct call stays, the
/// expectation is attached to the function item, and it carries a non-empty
/// reason.
///
/// Counting diagnostics cannot see this. An expectation written inside the
/// body suppresses the same warning and leaves the count at six, so the
/// fixture would still look correct while the policy's one permitted shape
/// had quietly become two.
///
/// Mutation proof (2026-09-08): moving the attribute inside the function body
/// failed with "must carry an item-scoped #[expect(...)], not one written
/// inside its body"; emptying the reason failed with "must carry a non-empty
/// reason"; and removing the `std::env::var` call failed with "must still
/// make the direct call it exempts". The diagnostic-count test passed
/// throughout the first two, which is why this test exists.
#[test]
fn the_composition_root_exception_keeps_its_shape() -> TestResult {
    ensure_composition_root_is_item_scoped()
}
