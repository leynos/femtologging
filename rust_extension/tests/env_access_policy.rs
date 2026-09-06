//! Contract coverage for the environment-access lint policy.
//!
//! The policy has three parts and each is load-bearing on its own: the banned
//! `std::env` paths in `clippy.toml`, the `deny` severity in the manifest, and
//! the Clippy invocation that carries both across every target kind. Removing
//! any one of them silently restores ambient environment access, so this file
//! asserts the mechanism rather than prose describing it.
//!
//! See `docs/adr-005-environment-seam-taxonomy.md` for the policy itself.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use toml::Value;

type TestResult = Result<(), Box<dyn Error>>;

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

/// Return the crate directory, which holds `clippy.toml` and `Cargo.toml`.
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Return the repository root, which holds the `Makefile`.
fn repository_root() -> Result<PathBuf, Box<dyn Error>> {
    crate_dir()
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "crate directory has no parent".into())
}

/// Parse a TOML document, naming the file in any failure.
fn parse_toml(path: &Path) -> Result<Value, Box<dyn Error>> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    // `str::parse` reads a single TOML *value*, not a document; use the
    // deserializer so the whole file is parsed.
    toml::from_str::<Value>(&text)
        .map_err(|error| format!("parse {}: {error}", path.display()).into())
}

/// Return the `disallowed-methods` entries declared in the Clippy policy.
fn disallowed_methods() -> Result<Vec<(String, String)>, Box<dyn Error>> {
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
    let mut methods = Vec::with_capacity(entries.len());
    for entry in entries {
        let path = entry
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "every disallowed-methods entry needs a path".to_string())?;
        let reason = entry
            .get("reason")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("disallowed-methods entry {path} needs a reason"))?;
        methods.push((path.to_owned(), reason.to_owned()));
    }
    Ok(methods)
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
fn makefile_recipe(makefile: &str, target: &str) -> Result<String, Box<dyn Error>> {
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

/// Read the repository Makefile.
fn makefile() -> Result<String, Box<dyn Error>> {
    let path = repository_root()?.join("Makefile");
    std::fs::read_to_string(&path)
        .map_err(|error| format!("read {}: {error}", path.display()).into())
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
/// "disallowed_methods must be denied ... found \"warn\"".
#[test]
fn manifest_denies_disallowed_methods() -> TestResult {
    let manifest_path = crate_dir().join("Cargo.toml");
    let manifest = parse_toml(&manifest_path)?;
    let lints = manifest
        .get("lints")
        .and_then(|lints| lints.get("clippy"))
        .ok_or_else(|| format!("{} must declare [lints.clippy]", manifest_path.display()))?;
    match lint_level(lints, "disallowed_methods").as_deref() {
        Some("deny") | Some("forbid") => Ok(()),
        other => Err(format!(
            "disallowed_methods must be denied in {}, found {other:?}",
            manifest_path.display()
        )
        .into()),
    }
}

/// Scenario: a contributor edits the Rust lint targets.
///
/// Invariant: the policy lane keeps denying `clippy::disallowed_methods` over
/// every target kind and every feature, and `make lint` still reaches it, so
/// the ban governs tests and benches rather than the library alone.
///
/// Mutation proof (2026-09-06): dropping `--all-targets` from
/// `ENV_POLICY_CLIPPY_FLAGS` failed this test with "ENV_POLICY_CLIPPY_FLAGS
/// must lint every target kind and every feature"; separately, removing the
/// `lint-env-policy` prerequisite from `lint-rust` failed it with "lint-rust
/// must run the lint-env-policy lane".
#[test]
fn environment_policy_lane_covers_every_target_and_feature() -> TestResult {
    let makefile = makefile()?;
    let flags = makefile
        .lines()
        .find_map(|line| line.strip_prefix("ENV_POLICY_CLIPPY_FLAGS ?="))
        .map(str::trim)
        .ok_or_else(|| "Makefile must define ENV_POLICY_CLIPPY_FLAGS".to_string())?;
    let (selection, rustc_flags) = flags
        .split_once(" -- ")
        .ok_or_else(|| "ENV_POLICY_CLIPPY_FLAGS must pass flags through to rustc".to_string())?;
    let selected: Vec<&str> = selection.split_whitespace().collect();
    if !selected.contains(&"--all-targets") || !selected.contains(&"--all-features") {
        return Err("ENV_POLICY_CLIPPY_FLAGS must lint every target kind and every feature".into());
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
    let policy_recipe = makefile_recipe(&makefile, "lint-env-policy")?;
    let clippy_command = policy_recipe
        .lines()
        .find(|line| line.contains("cargo clippy"))
        .ok_or_else(|| "lint-env-policy must invoke cargo clippy".to_string())?;
    if !clippy_command.contains("$(ENV_POLICY_CLIPPY_FLAGS)") {
        return Err("lint-env-policy must pass $(ENV_POLICY_CLIPPY_FLAGS) to cargo clippy".into());
    }
    let rust_recipe = makefile_recipe(&makefile, "lint-rust")?;
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
    let lint = makefile_recipe(&makefile, "lint")?;
    if !lint.contains("$(MAKE) lint-rust") {
        return Err("lint must delegate the Rust lanes to lint-rust".into());
    }
    Ok(())
}
