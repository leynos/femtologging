//! Contract coverage for the environment-access lint policy.
//!
//! The policy has four load-bearing parts and losing any one of them silently
//! restores ambient environment access: the banned `std::env` paths in
//! `clippy.toml`, the `deny` severity in the manifest, the lane list and flags
//! that carry the lint across every target kind and feature arm, and the shell
//! guard that lets a failing lane fail the build. This file asserts each
//! mechanism rather than prose describing it.
//!
//! Two of the tests execute rather than inspect. One runs `clippy-driver` over
//! `tests/fixtures/env_policy_probe.rs` under this crate's `clippy.toml`; the
//! other runs `make lint-env-policy` with a failing lane ahead of a passing
//! one.
//!
//! The machinery lives in `tests/test_utils/env_policy.rs`, keeping both files
//! inside the 400-line limit in `AGENTS.md`.
//!
//! See `docs/adr-006-environment-seam-taxonomy.md` for the policy itself.

#[path = "test_utils/env_policy.rs"]
mod env_policy;

use env_policy::{
    CRATE_MANIFEST, Makefile, REQUIRED_DISALLOWED_METHODS, TestResult, configured_severity,
    disallowed_methods, ensure_ci_runs_the_policy_gates, ensure_flags_deny_the_policy,
    ensure_lanes_cover_every_feature, ensure_lint_reaches_the_policy_lane,
    policy_lane_run_succeeds, probe_violations,
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

/// Scenario: a contributor edits the Rust lint targets or adds a feature.
///
/// Invariant: the policy lane keeps denying `clippy::disallowed_methods` over
/// every target kind and both arms of every feature gate, and `make lint`
/// still reaches it. `--all-features` alone would never compile a
/// `#[cfg(not(feature = ...))]` block, so the `none` lane is not optional.
///
/// Mutation proof (2026-09-06): narrowing `ENV_POLICY_CLIPPY_FLAGS` from
/// `--all-targets` to `--lib` failed this test with "ENV_POLICY_CLIPPY_FLAGS
/// must lint every target kind"; dropping `-D clippy::disallowed_methods` from
/// those flags failed it with "ENV_POLICY_CLIPPY_FLAGS must deny
/// clippy::disallowed_methods"; dropping the `none` lane failed it with
/// "ENV_POLICY_FEATURE_LANES must include the \"none\" lane"; dropping the
/// `test-util` lane failed it with "ENV_POLICY_FEATURE_LANES must lint the
/// Mutation proof (2026-09-07): narrowing `ENV_POLICY_CARGO_ARGS` from
/// `--all-targets` to `--lib` failed with "must lint every target kind";
/// dropping `-D clippy::disallowed_methods` from `ENV_POLICY_LINT_ARGS`
/// failed with "must deny clippy::disallowed_methods"; dropping the `none`
/// lane and the `test-util` lane each failed by name; and removing the
/// `lint-lanes-test` prerequisite failed with "lint-rust must run
/// lint-lanes-test".
///
/// Recipes are judged one whole command at a time, which is what kills the
/// two ways a command can survive review while doing nothing. Wrapping
/// `$(MAKE) lint-rust` in `if false; then ...; fi` and appending `|| true` to
/// it both failed with "lint must run \"$(MAKE) lint-rust\" as a command of
/// its own, not inside a wrapper". The same two mutations on the policy
/// recipe's driver call failed with "lint-env-policy must end in the lane
/// driver call", and appending a second command failed with "must be one
/// command, found 2".
///
/// `lint` reaches `lint-rust` through a prerequisite, which cannot be wrapped,
/// prefixed or made conditional at all; a whole recipe command whose failure
/// reaches Make is accepted as the equivalent. Removing the prerequisite,
/// moving it into a wrapped command, and prefixing the command with `-` or
/// `@-` all failed with "lint must run lint-rust as a prerequisite, or as a
/// command of its own whose failure reaches Make". A `@` prefix alone is
/// accepted, because it only suppresses echoing.
///
/// Mutation proof (2026-09-07), on the policy recipe's driver call: a `-`
/// prefix, a `-@` prefix, `|| true`, `|| :`, `; true`, and a trailing pipe
/// each failed with "lint-env-policy must let Make see the driver fail". Two
/// of Make's three recipe prefixes are cosmetic and one is not: stripping `-`
/// alongside `@` and `+` would have made `-$(MAKE) lint-rust` read as the
/// real gate.
#[test]
fn environment_policy_lane_covers_every_target_and_feature() -> TestResult {
    let makefile = Makefile::embedded();
    ensure_lanes_cover_every_feature(&makefile)?;
    ensure_flags_deny_the_policy(&makefile)?;
    ensure_lint_reaches_the_policy_lane(&makefile)
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

/// Scenario: one feature lane rejects the code and a later lane accepts it.
///
/// Invariant: `make lint-env-policy` fails. A shell `for` loop reports the
/// status of its last command, so without a per-lane guard a rejection in any
/// lane but the last is discarded and the gate silently stops gating. That is
/// exactly the case the lane list exists to catch, since a violation inside a
/// `#[cfg(not(feature = ...))]` block is reachable only from the `none` lane.
///
/// The run is driven through the real recipe with the lane list overridden, so
/// it tests the Makefile rather than a description of it. `badfeature` is not
/// a declared feature, so Cargo rejects that lane before building anything.
///
/// Mutation proof (2026-09-07): removing `|| exit 1` from the `lint-env-policy`
/// Clippy call made this run exit 0 and failed the test with "a failing lane
/// must fail lint-env-policy". That mutation is the state the target shipped
/// in before this test existed.
#[test]
fn a_failing_lane_fails_the_policy_target() -> TestResult {
    if policy_lane_run_succeeds("badfeature none")? {
        return Err(
            "a failing lane must fail lint-env-policy, even when a later lane passes".into(),
        );
    }
    if !policy_lane_run_succeeds("none")? {
        return Err("lint-env-policy must succeed when every lane passes".into());
    }
    Ok(())
}

/// Scenario: a contributor edits the CI workflow.
///
/// Invariant: CI still runs both policy gates on every pull request, each as
/// a step's whole command, with no condition on the step or its job and no
/// blanket tolerance of that job's failure. Everything else in this file
/// proves the policy holds when the gates run; this proves they run.
///
/// The workflow is parsed rather than searched, because a text search is
/// satisfied by `if false; then make lint; fi` and by `make lint || true`,
/// and no list of falsy spellings is reliable: YAML resolves `false` to a
/// boolean whose string form is `False`.
///
/// Failure tolerance is judged by an allow-list of shapes rather than a
/// deny-list of spellings, because a deny-list would have to enumerate every
/// constant-true expression and `${{ true }}` is only the first. A job may
/// consult the matrix, which is how an experimental leg is singled out; a
/// required step may not, since on a step that would say this gate may fail.
///
/// Mutation proof (2026-09-07): `if: false` on the Lint step failed with "the
/// \"make lint\" step in job build-test must carry no condition"; the same on
/// the job failed with "job build-test runs \"make lint\" but carries a
/// condition"; a push-only condition failed identically; wrapping the command
/// and appending `|| true` each failed with "must run \"make lint\" as a
/// step's whole command", as did renaming the command; and deleting the
/// `pull_request` trigger failed with "must trigger on pull_request".
/// Renaming the job changes nothing, correctly, since the command is sought
/// in any job rather than in one by name.
///
/// Mutation proof (2026-09-08): on the job, `continue-on-error: true`,
/// `${{ true }}` and `'yes'` each failed with "tolerates failure outside a
/// matrix leg", while `false` and the matrix expression were accepted. On the
/// Lint step, `true`, `${{ true }}` and even `${{ matrix.experimental }}`
/// each failed with "must not tolerate its own failure", while `false` was
/// accepted.
#[test]
fn ci_runs_the_policy_gates_unconditionally() -> TestResult {
    ensure_ci_runs_the_policy_gates()
}
