//! Shared machinery for the environment-access policy contract.
//!
//! The three configuration files are embedded with `include_str!`, which
//! resolves relative to *this* source file at compile time. That reaches the
//! `Makefile` above `CARGO_MANIFEST_DIR` without any runtime filesystem
//! access, so Whitaker's `no_std_fs_operations` never fires and the crate
//! needs no `dylint.toml` exclusion. Moving or deleting one of the three files
//! becomes a compile error rather than a runtime failure.
//!
//! `tests/env_access_policy.rs` holds the assertions; this module holds the
//! parsing, the Makefile queries, and the Clippy probe they share.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use toml::Value;

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

/// Feature lanes the policy must be linted under regardless of what else the
/// lane list gains. `none` and `all` between them compile both arms of every
/// feature gate, which `--all-features` alone cannot do.
const REQUIRED_FEATURE_LANES: [&str; 2] = ["none", "all"];

/// Manifest feature keys that name no lane of their own.
const LANE_EXEMPT_FEATURES: [&str; 1] = ["default"];

/// A checked-in configuration file, embedded at compile time.
#[derive(Clone, Copy)]
pub(crate) struct Embedded {
    /// Repository-relative path, used only in failure messages.
    pub(crate) name: &'static str,
    /// The file's contents as of the last build.
    pub(crate) text: &'static str,
}

impl Embedded {
    /// Parse the file as a TOML document, naming it in any failure.
    pub(crate) fn parse(self) -> Fallible<Value> {
        // `str::parse` reads a single TOML *value*, not a document; use the
        // deserializer so the whole file is parsed.
        toml::from_str::<Value>(self.text)
            .map_err(|error| format!("parse {}: {error}", self.name).into())
    }
}

/// The Clippy policy this crate is linted under.
pub(crate) const CLIPPY_POLICY: Embedded = Embedded {
    name: "rust_extension/clippy.toml",
    text: include_str!("../../clippy.toml"),
};

/// The crate manifest, which carries the lint severity and the feature list.
pub(crate) const CRATE_MANIFEST: Embedded = Embedded {
    name: "rust_extension/Cargo.toml",
    text: include_str!("../../Cargo.toml"),
};

/// Return the repository root, which is where `make` must run.
pub(crate) fn repository_root() -> Fallible<PathBuf> {
    crate_dir()
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "crate directory has no parent".into())
}

/// Return the crate directory, which holds `clippy.toml` and the fixture.
pub(crate) fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Return the `disallowed-methods` entries declared in the Clippy policy.
pub(crate) fn disallowed_methods() -> Fallible<Vec<(String, String)>> {
    let policy = CLIPPY_POLICY.parse()?;
    let entries = policy
        .get("disallowed-methods")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "{} must declare a disallowed-methods array",
                CLIPPY_POLICY.name
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

/// Return the severity configured for `clippy::disallowed_methods`, whether it
/// is written as a bare string or as a table with a `level` key.
pub(crate) fn configured_severity(lints: &Value) -> Option<&str> {
    let entry = lints.get("disallowed_methods")?;
    entry
        .as_str()
        .or_else(|| entry.get("level").and_then(Value::as_str))
}

/// The name of a Makefile recipe this contract depends on.
#[derive(Clone, Copy)]
pub(crate) struct Recipe(&'static str);

/// The name of a Makefile variable this contract depends on.
#[derive(Clone, Copy)]
pub(crate) struct Variable(&'static str);

/// The aggregate lint target CI invokes.
const LINT: Recipe = Recipe("lint");
/// The Rust lint target, which must run the policy lane first.
const LINT_RUST: Recipe = Recipe("lint-rust");
/// The policy lane itself.
const LINT_ENV_POLICY: Recipe = Recipe("lint-env-policy");
/// The feature lanes the policy is linted under.
const FEATURE_LANES: Variable = Variable("ENV_POLICY_FEATURE_LANES");
/// The Cargo arguments every policy lane carries.
const CARGO_ARGS: Variable = Variable("ENV_POLICY_CARGO_ARGS");
/// The lint-driver arguments every policy lane carries.
const LINT_ARGS: Variable = Variable("ENV_POLICY_LINT_ARGS");
/// The script that walks the lanes.
const LANES_SCRIPT: Variable = Variable("LINT_LANES_SCRIPT");

/// The repository `Makefile`, embedded once and queried by name.
pub(crate) struct Makefile(&'static str);

impl Makefile {
    /// Return the repository Makefile, embedded at compile time.
    pub(crate) fn embedded() -> Self {
        Self(include_str!("../../../Makefile"))
    }

    /// Return the body of the named recipe, including its own line.
    pub(crate) fn recipe(&self, target: Recipe) -> Fallible<String> {
        let Recipe(name) = target;
        let prefix = format!("{name}:");
        let mut lines = self.0.lines().skip_while(|line| !line.starts_with(&prefix));
        let header = lines
            .next()
            .ok_or_else(|| format!("Makefile has no `{name}` target"))?;
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

    /// Return the value of a `?=` variable.
    pub(crate) fn variable(&self, variable: Variable) -> Fallible<&'static str> {
        let Variable(name) = variable;
        let prefix = format!("{name} ?=");
        self.0
            .lines()
            .find_map(|line| line.strip_prefix(prefix.as_str()))
            .map(str::trim)
            .ok_or_else(|| format!("Makefile must define {name}").into())
    }
}

/// Return the optional-feature names declared by the crate manifest.
fn declared_features() -> Fallible<Vec<String>> {
    let manifest = CRATE_MANIFEST.parse()?;
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
pub(crate) fn ensure_lanes_cover_every_feature(makefile: &Makefile) -> TestResult {
    let lanes: Vec<&str> = makefile
        .variable(FEATURE_LANES)?
        .split_whitespace()
        .collect();
    let missing_lane = REQUIRED_FEATURE_LANES
        .into_iter()
        .find(|required| !lanes.contains(required));
    if let Some(required) = missing_lane {
        return Err(format!(
            "ENV_POLICY_FEATURE_LANES must include the {required:?} lane, found {lanes:?}"
        )
        .into());
    }
    let missing_feature = declared_features()?
        .into_iter()
        .find(|feature| !lanes.contains(&feature.as_str()));
    match missing_feature {
        None => Ok(()),
        Some(feature) => Err(format!(
            "ENV_POLICY_FEATURE_LANES must lint the {feature:?} feature, found {lanes:?}"
        )
        .into()),
    }
}

/// Fail unless the policy lint reaches every target kind and denies the lint
/// outright.
pub(crate) fn ensure_flags_deny_the_policy(makefile: &Makefile) -> TestResult {
    let cargo_args = makefile.variable(CARGO_ARGS)?;
    if !cargo_args
        .split_whitespace()
        .any(|flag| flag == "--all-targets")
    {
        return Err(format!(
            "ENV_POLICY_CARGO_ARGS must lint every target kind, found {cargo_args:?}"
        )
        .into());
    }
    let lint_args = makefile.variable(LINT_ARGS)?;
    let denials: Vec<&str> = lint_args.split_whitespace().collect();
    if denials
        .windows(2)
        .all(|pair| pair != ["-D", DISALLOWED_METHODS_LINT])
    {
        return Err(format!(
            "ENV_POLICY_LINT_ARGS must deny {DISALLOWED_METHODS_LINT}, found {lint_args:?}"
        )
        .into());
    }
    Ok(())
}

/// The Makefile variables the policy recipe must hand to the driver.
const POLICY_EXPORTS: [&str; 3] = [
    "INPUT_LANES=\"$(ENV_POLICY_FEATURE_LANES)\"",
    "INPUT_CARGO_ARGS=\"$(ENV_POLICY_CARGO_ARGS)\"",
    "INPUT_LINT_ARGS=\"$(ENV_POLICY_LINT_ARGS)\"",
];

/// The targets `lint-rust` must run before the Whitaker suite.
const RUST_LINT_PREREQUISITES: [&str; 2] = ["lint-env-policy", "lint-lanes-test"];

/// Fail unless the policy recipe hands the driver every input it needs.
fn ensure_policy_recipe_drives_the_script(makefile: &Makefile) -> TestResult {
    let recipe = makefile.recipe(LINT_ENV_POLICY)?;
    let missing = POLICY_EXPORTS
        .into_iter()
        .find(|exported| !recipe.contains(exported));
    if let Some(exported) = missing {
        return Err(format!("lint-env-policy must export {exported}").into());
    }
    if !recipe.contains("uv run --script $(LINT_LANES_SCRIPT)") {
        return Err("lint-env-policy must run the lane driver script".into());
    }
    let script = makefile.variable(LANES_SCRIPT)?;
    if script != "scripts/lint_rust_lanes.py" {
        return Err(
            format!("LINT_LANES_SCRIPT must name the lane driver, found {script:?}").into(),
        );
    }
    Ok(())
}

/// Fail unless `make lint` reaches the policy recipe.
fn ensure_lint_reaches_lint_rust(makefile: &Makefile) -> TestResult {
    let rust_recipe = makefile.recipe(LINT_RUST)?;
    let prerequisites = rust_recipe
        .lines()
        .next()
        .and_then(|header| header.split_once(':'))
        .map(|(_, rest)| rest)
        .ok_or_else(|| "lint-rust must declare its prerequisites".to_string())?;
    let declared: Vec<&str> = prerequisites.split_whitespace().collect();
    let missing = RUST_LINT_PREREQUISITES
        .into_iter()
        .find(|required| !declared.contains(required));
    if let Some(required) = missing {
        return Err(format!("lint-rust must run {required}, found {declared:?}").into());
    }
    if !makefile.recipe(LINT)?.contains("$(MAKE) lint-rust") {
        return Err("lint must delegate the Rust lanes to lint-rust".into());
    }
    Ok(())
}

/// Fail unless `make lint` still reaches the policy lane through the driver.
///
/// The lane walk lives in `scripts/lint_rust_lanes.py`, so the recipe has to
/// hand that script the lane list and both argument sets, and `make lint` has
/// to reach it.
pub(crate) fn ensure_lint_reaches_the_policy_lane(makefile: &Makefile) -> TestResult {
    ensure_policy_recipe_drives_the_script(makefile)?;
    ensure_lint_reaches_lint_rust(makefile)
}

/// One `clippy::disallowed_methods` diagnostic: the method Clippy named and
/// the note it printed underneath.
#[derive(Debug)]
pub(crate) struct Violation {
    pub(crate) method: String,
    pub(crate) notes: Vec<String>,
}

/// Return the policy violations Clippy reports for the fixture.
pub(crate) fn probe_violations() -> Fallible<Vec<Violation>> {
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

/// Return one diagnostic as a violation, if it is one.
fn policy_violation(diagnostic: &serde_json::Value) -> Option<Violation> {
    let code = diagnostic.get("code")?.get("code")?.as_str()?;
    if code != DISALLOWED_METHODS_LINT {
        return None;
    }
    let message = diagnostic.get("message")?.as_str()?;
    let method = message.split('`').nth(1)?.to_owned();
    let notes = diagnostic
        .get("children")
        .and_then(serde_json::Value::as_array)
        .map(|children| {
            children
                .iter()
                .filter_map(|child| child.get("message")?.as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some(Violation { method, notes })
}

/// Run `make lint-env-policy` over the given lanes and report whether it
/// succeeded.
///
/// The lint flags are narrowed to `-A clippy::all` so the run exercises the
/// recipe's exit-status handling rather than the policy itself. A lane naming
/// a feature the crate does not declare fails immediately, before any build.
pub(crate) fn policy_lane_run_succeeds(lanes: &str) -> Fallible<bool> {
    let status = Command::new("make")
        .current_dir(repository_root()?)
        .arg("lint-env-policy")
        .arg(format!("ENV_POLICY_FEATURE_LANES={lanes}"))
        .arg("ENV_POLICY_LINT_ARGS=-A clippy::all")
        .output()
        .map_err(|error| format!("run make lint-env-policy over lanes {lanes:?}: {error}"))?
        .status;
    Ok(status.success())
}
