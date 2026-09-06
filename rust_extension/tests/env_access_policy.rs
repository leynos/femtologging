//! Contract coverage for the environment-access lint policy.
//!
//! The policy has three configuration parts and each is load-bearing on its
//! own: the banned `std::env` paths in `clippy.toml`, the `deny` severity in
//! the manifest, and the Clippy invocation that carries both across every
//! target kind and every feature arm. Removing any one of them silently
//! restores ambient environment access, so this file asserts each mechanism
//! rather than prose describing it.
//!
//! A fourth test closes the loop by compiling a fixture: it runs
//! `clippy-driver` against `tests/fixtures/env_policy_probe.rs` with this
//! crate's `clippy.toml`, and checks that all six methods are rejected and
//! that the sanctioned item-scoped `expect` is honoured.
//!
//! See `docs/adr-005-environment-seam-taxonomy.md` for the policy itself.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use toml::Value;

type TestResult = Result<(), Box<dyn Error>>;
type Fallible<T> = Result<T, Box<dyn Error>>;

/// The Clippy lint that carries the ban.
const DISALLOWED_METHODS_LINT: &str = "clippy::disallowed_methods";

/// Environment methods the project refuses to call ambiently, each paired with
/// the remedy Clippy must print when a contributor trips the lint.
const REQUIRED_DISALLOWED_METHODS: [(&str, &str); 6] = [
    ("std::env::var", "inject an environment reader"),
    ("std::env::var_os", "inject an environment reader"),
    ("std::env::vars", "inject an environment reader"),
    ("std::env::vars_os", "inject an environment reader"),
    ("std::env::set_var", "use a stub environment in tests"),
    ("std::env::remove_var", "use a stub environment in tests"),
];

/// Feature lanes the policy must be linted under regardless of what else the
/// lane list gains. `none` and `all` between them compile both arms of every
/// feature gate, which `--all-features` alone cannot do.
const REQUIRED_FEATURE_LANES: [&str; 2] = ["none", "all"];

/// Manifest feature keys that name no lane of their own.
const LANE_EXEMPT_FEATURES: [&str; 1] = ["default"];

/// Return the crate directory, which holds `clippy.toml` and `Cargo.toml`.
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Return the repository root, which holds the `Makefile`.
fn repository_root() -> Fallible<PathBuf> {
    crate_dir()
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "crate directory has no parent".into())
}

/// Read a file, naming it in any failure.
fn read(path: &Path) -> Fallible<String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()).into())
}

/// Parse a TOML document, naming the file in any failure.
fn parse_toml(path: &Path) -> Fallible<Value> {
    // `str::parse` reads a single TOML *value*, not a document; use the
    // deserializer so the whole file is parsed.
    toml::from_str::<Value>(&read(path)?)
        .map_err(|error| format!("parse {}: {error}", path.display()).into())
}

/// Read the repository Makefile.
fn makefile() -> Fallible<String> {
    read(&repository_root()?.join("Makefile"))
}

/// Read the crate manifest.
fn manifest() -> Fallible<Value> {
    parse_toml(&crate_dir().join("Cargo.toml"))
}

/// Return the `disallowed-methods` entries declared in the Clippy policy.
fn disallowed_methods() -> Fallible<Vec<(String, String)>> {
    let policy_path = crate_dir().join("clippy.toml");
    let policy = parse_toml(&policy_path)?;
    let entries = policy
        .get("disallowed-methods")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "{} must declare a disallowed-methods array",
                policy_path.display()
            )
        })?;
    entries.iter().map(disallowed_method_entry).collect()
}

/// Return one `disallowed-methods` entry as a path and its remedy.
fn disallowed_method_entry(entry: &Value) -> Fallible<(String, String)> {
    let path = entry
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "every disallowed-methods entry needs a path".to_string())?;
    let reason = entry
        .get("reason")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("disallowed-methods entry {path} needs a reason"))?;
    Ok((path.to_owned(), reason.to_owned()))
}

/// Return the configured severity of a manifest lint, whether it is written as
/// a bare string or as a table with a `level` key.
fn lint_level(lints: &Value, lint: &str) -> Option<String> {
    let entry = lints.get(lint)?;
    entry
        .as_str()
        .or_else(|| entry.get("level").and_then(Value::as_str))
        .map(str::to_owned)
}

/// Return the body of the named Makefile recipe, including its own line.
fn makefile_recipe(makefile: &str, target: &str) -> Fallible<String> {
    let prefix = format!("{target}:");
    let mut lines = makefile
        .lines()
        .skip_while(|line| !line.starts_with(&prefix));
    let header = lines
        .next()
        .ok_or_else(|| format!("Makefile has no `{target}` target"))?;
    let mut recipe = String::from(header);
    for line in lines {
        if !line.starts_with('\t') && !line.trim().is_empty() {
            break;
        }
        writeln!(recipe)?;
        recipe.push_str(line);
    }
    Ok(recipe)
}

/// Return the value of a `?=` Makefile variable.
fn makefile_variable<'a>(makefile: &'a str, name: &str) -> Fallible<&'a str> {
    let prefix = format!("{name} ?=");
    makefile
        .lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .map(str::trim)
        .ok_or_else(|| format!("Makefile must define {name}").into())
}

/// Return the optional-feature names declared by the crate manifest.
fn declared_features() -> Fallible<Vec<String>> {
    let manifest = manifest()?;
    let features = manifest
        .get("features")
        .and_then(Value::as_table)
        .ok_or_else(|| "Cargo.toml must declare [features]".to_string())?;
    Ok(features
        .keys()
        .filter(|name| !LANE_EXEMPT_FEATURES.contains(&name.as_str()))
        .cloned()
        .collect())
}

/// Fail unless the policy lane list covers both arms of every feature gate.
fn ensure_lanes_cover_every_feature(lanes: &[&str]) -> TestResult {
    for required in REQUIRED_FEATURE_LANES {
        if !lanes.contains(&required) {
            return Err(format!(
                "ENV_POLICY_FEATURE_LANES must include the {required:?} lane, found {lanes:?}"
            )
            .into());
        }
    }
    for feature in declared_features()? {
        if !lanes.contains(&feature.as_str()) {
            return Err(format!(
                "ENV_POLICY_FEATURE_LANES must lint the {feature:?} feature, found {lanes:?}"
            )
            .into());
        }
    }
    Ok(())
}

/// Fail unless the policy Clippy flags reach every target kind and deny the
/// lint outright.
fn ensure_flags_deny_the_policy(flags: &str) -> TestResult {
    let (selection, rustc_flags) = flags
        .split_once(" -- ")
        .ok_or_else(|| "ENV_POLICY_CLIPPY_FLAGS must pass flags through to rustc".to_string())?;
    if !selection
        .split_whitespace()
        .any(|flag| flag == "--all-targets")
    {
        return Err("ENV_POLICY_CLIPPY_FLAGS must lint every target kind".into());
    }
    let denials: Vec<&str> = rustc_flags.split_whitespace().collect();
    if denials
        .windows(2)
        .all(|pair| pair != ["-D", DISALLOWED_METHODS_LINT])
    {
        return Err(format!(
            "ENV_POLICY_CLIPPY_FLAGS must deny {DISALLOWED_METHODS_LINT}, found {rustc_flags:?}"
        )
        .into());
    }
    Ok(())
}

/// Fail unless `make lint` still reaches the policy lane.
fn ensure_lint_reaches_the_policy_lane(makefile: &str) -> TestResult {
    let policy_recipe = makefile_recipe(makefile, "lint-env-policy")?;
    if !policy_recipe.contains("$(ENV_POLICY_CLIPPY_FLAGS)")
        || !policy_recipe.contains("$(ENV_POLICY_FEATURE_LANES)")
    {
        return Err("lint-env-policy must drive Clippy from both policy variables".into());
    }
    let rust_recipe = makefile_recipe(makefile, "lint-rust")?;
    let prerequisites = rust_recipe
        .lines()
        .next()
        .and_then(|header| header.split_once(':'))
        .map(|(_, rest)| rest)
        .ok_or_else(|| "lint-rust must declare its prerequisites".to_string())?;
    if !prerequisites
        .split_whitespace()
        .any(|word| word == "lint-env-policy")
    {
        return Err("lint-rust must run the lint-env-policy lane".into());
    }
    if !makefile_recipe(makefile, "lint")?.contains("$(MAKE) lint-rust") {
        return Err("lint must delegate the Rust lanes to lint-rust".into());
    }
    Ok(())
}

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
    let manifest_path = crate_dir().join("Cargo.toml");
    let manifest = manifest()?;
    let lints = manifest
        .get("lints")
        .and_then(|lints| lints.get("clippy"))
        .ok_or_else(|| format!("{} must declare [lints.clippy]", manifest_path.display()))?;
    match lint_level(lints, "disallowed_methods").as_deref() {
        Some("deny" | "forbid") => Ok(()),
        other => Err(format!(
            "disallowed_methods must be denied in {}, found {other:?}",
            manifest_path.display()
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
/// \"test-util\" feature"; and removing the `lint-env-policy` prerequisite
/// from `lint-rust` failed it with "lint-rust must run the lint-env-policy
/// lane".
#[test]
fn environment_policy_lane_covers_every_target_and_feature() -> TestResult {
    let makefile = makefile()?;
    let lanes: Vec<&str> = makefile_variable(&makefile, "ENV_POLICY_FEATURE_LANES")?
        .split_whitespace()
        .collect();
    ensure_lanes_cover_every_feature(&lanes)?;
    ensure_flags_deny_the_policy(makefile_variable(&makefile, "ENV_POLICY_CLIPPY_FLAGS")?)?;
    ensure_lint_reaches_the_policy_lane(&makefile)
}

/// Return the `clippy::disallowed_methods` diagnostics Clippy emits for the
/// fixture, as `(method, line)` pairs.
fn probe_diagnostics() -> Fallible<Vec<(String, u64)>> {
    let fixture = crate_dir().join("tests/fixtures/env_policy_probe.rs");
    let out_dir = tempfile::tempdir()?;
    let output = Command::new("clippy-driver")
        .env("CLIPPY_CONF_DIR", crate_dir())
        .arg("--edition=2024")
        .arg("--crate-type=lib")
        .arg("--emit=metadata")
        .arg("--error-format=json")
        .arg("--out-dir")
        .arg(out_dir.path())
        .arg("-D")
        .arg(DISALLOWED_METHODS_LINT)
        .arg(&fixture)
        .output()
        .map_err(|error| format!("run clippy-driver on {}: {error}", fixture.display()))?;
    let rendered = String::from_utf8(output.stderr)?;
    let mut found = Vec::new();
    for line in rendered.lines() {
        let Ok(diagnostic) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        found.extend(policy_violation(&diagnostic));
    }
    Ok(found)
}

/// Return the method and line of one diagnostic, if it is a policy violation.
fn policy_violation(diagnostic: &serde_json::Value) -> Option<(String, u64)> {
    let code = diagnostic.get("code")?.get("code")?.as_str()?;
    if code != DISALLOWED_METHODS_LINT {
        return None;
    }
    let line = diagnostic
        .get("spans")?
        .as_array()?
        .first()?
        .get("line_start")?
        .as_u64()?;
    let message = diagnostic.get("message")?.as_str()?;
    let method = message.split('`').nth(1)?.to_owned();
    Some((method, line))
}

/// Scenario: Clippy compiles code that calls each banned method, and code that
/// calls one from a sanctioned composition root.
///
/// Invariant: every banned method is rejected under this crate's `clippy.toml`,
/// and the item-scoped `#[expect(clippy::disallowed_methods, reason = "...")]`
/// escape hatch suppresses exactly one call and nothing more. The count is what
/// proves the escape hatch is honoured rather than merely tolerated.
///
/// Mutation proof (2026-09-06): removing the `#[expect]` attribute from the
/// fixture's `composition_root` raised the violation count to seven and failed
/// this test with "the composition-root expect must suppress exactly one call".
#[test]
fn clippy_rejects_every_banned_method_in_a_compiled_fixture() -> TestResult {
    let violations = probe_diagnostics()?;
    for (banned, _) in REQUIRED_DISALLOWED_METHODS {
        if !violations.iter().any(|(method, _)| method == banned) {
            return Err(format!(
                "Clippy must reject {banned} in the fixture, found {violations:?}"
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
