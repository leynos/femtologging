//! The checked-in configuration this policy is written down in.
//!
//! `clippy.toml` and `Cargo.toml` are embedded at compile time, so a file that
//! moves or disappears is a build failure rather than a runtime one, and no
//! test reads the filesystem to find them.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use toml::Value;

use super::{DISALLOWED_METHODS_LINT, Fallible, TestResult};

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
    text: include_str!("../../../clippy.toml"),
};

/// The crate manifest, which carries the lint severity and the feature list.
pub(crate) const CRATE_MANIFEST: Embedded = Embedded {
    name: "rust_extension/Cargo.toml",
    text: include_str!("../../../Cargo.toml"),
};

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

/// One `clippy::disallowed_methods` diagnostic: the method Clippy named and
/// the note it printed underneath.
#[derive(Debug)]
pub(crate) struct Violation {
    pub(crate) method: String,
    pub(crate) notes: Vec<String>,
}

/// The fixture's source, embedded so a moved or deleted file is a compile
/// error rather than a silently skipped assertion.
pub(crate) const PROBE_FIXTURE: Embedded = Embedded {
    name: "rust_extension/tests/fixtures/env_policy_probe.rs",
    text: include_str!("../../fixtures/env_policy_probe.rs"),
};

/// The fixture function that stands in for a sanctioned composition root.
const COMPOSITION_ROOT: &str = "pub fn composition_root()";

/// Return the attribute attached to the fixture's composition root, if any.
///
/// The attribute must sit immediately above the function item. An expectation
/// written inside the body would suppress the same diagnostic and keep the
/// count at six, so counting diagnostics cannot tell the two apart: only the
/// shape can.
fn composition_root_attribute() -> Fallible<String> {
    let (before, _) = PROBE_FIXTURE
        .text
        .split_once(COMPOSITION_ROOT)
        .ok_or_else(|| format!("{} must define composition_root", PROBE_FIXTURE.name))?;
    let attribute: String = before
        .rsplit_once("#[expect(")
        .map(|(_, rest)| format!("#[expect({rest}"))
        .unwrap_or_default();
    if attribute.contains("fn ") || attribute.contains("}") {
        // Something else stands between the attribute and the function, so the
        // attribute is not this item's.
        return Ok(String::new());
    }
    Ok(attribute)
}

/// Fail unless the fixture's composition root carries the sanctioned shape.
///
/// Three things are asserted together because each alone can be satisfied
/// while the policy is evaded: the direct call must still be there, the
/// expectation must be item-scoped rather than statement-scoped, and it must
/// carry a non-empty reason.
pub(crate) fn ensure_composition_root_is_item_scoped() -> TestResult {
    let (_, after) = PROBE_FIXTURE
        .text
        .split_once(COMPOSITION_ROOT)
        .ok_or_else(|| format!("{} must define composition_root", PROBE_FIXTURE.name))?;
    let body = after.split_once("\n}").map_or(after, |(body, _)| body);
    if !body.contains("std::env::var") {
        return Err(format!(
            "{}'s composition_root must still make the direct call it exempts",
            PROBE_FIXTURE.name
        )
        .into());
    }
    let attribute = composition_root_attribute()?;
    if attribute.is_empty() {
        return Err(format!(
            "{}'s composition_root must carry an item-scoped #[expect(...)], not one \
             written inside its body",
            PROBE_FIXTURE.name
        )
        .into());
    }
    if !attribute.contains(DISALLOWED_METHODS_LINT) {
        return Err(format!(
            "{}'s composition_root must expect {DISALLOWED_METHODS_LINT}, found {attribute:?}",
            PROBE_FIXTURE.name
        )
        .into());
    }
    ensure_attribute_has_a_reason(&attribute)
}

/// Fail unless an expectation records why it is there.
fn ensure_attribute_has_a_reason(attribute: &str) -> TestResult {
    let reason = attribute
        .split_once("reason =")
        .map(|(_, rest)| rest.trim_start())
        .and_then(|rest| rest.strip_prefix('"'))
        .and_then(|rest| rest.split_once('"'))
        .map(|(reason, _)| reason.trim());
    match reason {
        Some(text) if !text.is_empty() => Ok(()),
        _ => Err(format!(
            "the composition-root expectation must carry a non-empty reason, found \
             {attribute:?}"
        )
        .into()),
    }
}

/// Return the host `PATH`, the one inherited value the probe cannot do without.
///
/// This is the composition root for the child process: the point where its
/// environment is built. Clearing the environment is what makes the probe
/// hermetic, and a cleared environment has no `PATH`, so `clippy-driver`
/// could not be found at all. The value has to come from the parent, and
/// reading it here is the ADR's sanctioned item-scoped exception rather than
/// an ambient read scattered through the code.
///
/// `option_env!` is not an alternative: it resolves at compile time and would
/// bake in the building machine's `PATH`.
#[expect(
    clippy::disallowed_methods,
    reason = "composition root: the child's PATH must come from the parent, and \
              a cleared environment has none"
)]
fn host_path() -> Fallible<OsString> {
    std::env::var_os("PATH").ok_or_else(|| "PATH must be set to find clippy-driver".into())
}

/// Return the policy violations Clippy reports for the fixture.
pub(crate) fn probe_violations() -> Fallible<Vec<Violation>> {
    let fixture = crate_dir().join("tests/fixtures/env_policy_probe.rs");
    let out_dir = tempfile::tempdir()?;
    // Build the child's environment rather than inheriting one. A probe for
    // the environment-access policy that reads host state would be testing
    // this machine as much as the policy: an inherited CLIPPY_CONF_DIR would
    // point the lint at another configuration, and RUSTFLAGS or CLIPPY_ARGS
    // would change the diagnostics it counts. PATH is forwarded because it is
    // how `clippy-driver` is found at all, which is the one ambient input this
    // cannot remove; the ADR's own rule is to clear and then add back what the
    // child legitimately needs.
    let output = Command::new("clippy-driver")
        .env_clear()
        .env("PATH", host_path()?)
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
