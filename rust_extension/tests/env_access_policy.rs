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
//! crate's `clippy.toml`, and checks that all six methods are rejected with
//! their reason strings and that the sanctioned item-scoped `expect` is
//! honoured.
//!
//! The three configuration files are embedded with `include_str!`, which
//! resolves relative to this source file at compile time. That reaches the
//! `Makefile` above `CARGO_MANIFEST_DIR` without any runtime filesystem
//! access, so Whitaker's `no_std_fs_operations` never fires and this crate
//! needs no `dylint.toml` exclusion. Moving or deleting one of the three
//! files becomes a compile error rather than a runtime failure.
//!
//! See `docs/adr-005-environment-seam-taxonomy.md` for the policy itself.

use std::error::Error;
use std::fmt::Write as _;
use std::path::PathBuf;
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

/// A checked-in configuration file, embedded at compile time.
#[derive(Clone, Copy)]
struct Embedded {
    /// Repository-relative path, used only in failure messages.
    name: &'static str,
    /// The file's contents as of the last build.
    text: &'static str,
}

impl Embedded {
    /// Parse the file as a TOML document, naming it in any failure.
    fn parse(self) -> Fallible<Value> {
        // `str::parse` reads a single TOML *value*, not a document; use the
        // deserializer so the whole file is parsed.
        toml::from_str::<Value>(self.text)
            .map_err(|error| format!("parse {}: {error}", self.name).into())
    }
}

/// The Clippy policy this crate is linted under.
const CLIPPY_POLICY: Embedded = Embedded {
    name: "rust_extension/clippy.toml",
    text: include_str!("../clippy.toml"),
};

/// The crate manifest, which carries the lint severity and the feature list.
const CRATE_MANIFEST: Embedded = Embedded {
    name: "rust_extension/Cargo.toml",
    text: include_str!("../Cargo.toml"),
};

/// Return the crate directory, which holds `clippy.toml` and the fixture.
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Return the `disallowed-methods` entries declared in the Clippy policy.
fn disallowed_methods() -> Fallible<Vec<(String, String)>> {
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

/// Return the configured severity of a manifest lint, whether it is written as
/// a bare string or as a table with a `level` key.
fn lint_level(lints: &Value, lint: &str) -> Option<String> {
    let entry = lints.get(lint)?;
    entry
        .as_str()
        .or_else(|| entry.get("level").and_then(Value::as_str))
        .map(str::to_owned)
}

/// The repository `Makefile`, read once and queried by name.
struct Makefile(&'static str);

impl Makefile {
    /// Return the repository Makefile, embedded at compile time.
    fn embedded() -> Self {
        Self(include_str!("../../Makefile"))
    }

    /// Return the body of the named recipe, including its own line.
    fn recipe(&self, target: &str) -> Fallible<String> {
        let prefix = format!("{target}:");
        let mut lines = self.0.lines().skip_while(|line| !line.starts_with(&prefix));
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

    /// Return the value of a `?=` variable.
    fn variable(&self, name: &str) -> Fallible<&'static str> {
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
fn ensure_lanes_cover_every_feature(lanes: &[&str]) -> TestResult {
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
fn ensure_lint_reaches_the_policy_lane(makefile: &Makefile) -> TestResult {
    let policy_recipe = makefile.recipe("lint-env-policy")?;
    if !policy_recipe.contains("$(ENV_POLICY_CLIPPY_FLAGS)")
        || !policy_recipe.contains("$(ENV_POLICY_FEATURE_LANES)")
    {
        return Err("lint-env-policy must drive Clippy from both policy variables".into());
    }
    let rust_recipe = makefile.recipe("lint-rust")?;
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
    if !makefile.recipe("lint")?.contains("$(MAKE) lint-rust") {
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
    let manifest = CRATE_MANIFEST.parse()?;
    let lints = manifest
        .get("lints")
        .and_then(|lints| lints.get("clippy"))
        .ok_or_else(|| format!("{} must declare [lints.clippy]", CRATE_MANIFEST.name))?;
    match lint_level(lints, "disallowed_methods").as_deref() {
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
/// \"test-util\" feature"; and removing the `lint-env-policy` prerequisite
/// from `lint-rust` failed it with "lint-rust must run the lint-env-policy
/// lane".
#[test]
fn environment_policy_lane_covers_every_target_and_feature() -> TestResult {
    let makefile = Makefile::embedded();
    let lanes: Vec<&str> = makefile
        .variable("ENV_POLICY_FEATURE_LANES")?
        .split_whitespace()
        .collect();
    ensure_lanes_cover_every_feature(&lanes)?;
    ensure_flags_deny_the_policy(makefile.variable("ENV_POLICY_CLIPPY_FLAGS")?)?;
    ensure_lint_reaches_the_policy_lane(&makefile)
}

/// One `clippy::disallowed_methods` diagnostic: the method Clippy named and
/// the note it printed underneath.
#[derive(Debug)]
struct Violation {
    method: String,
    notes: Vec<String>,
}

/// Return the policy violations Clippy reports for the fixture.
fn probe_violations() -> Fallible<Vec<Violation>> {
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
