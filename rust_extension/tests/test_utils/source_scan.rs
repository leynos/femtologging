//! Source scan closing the attribute routes around the environment policy.
//!
//! `tests/env_policy_source_scan.rs` holds the assertions; this module holds
//! the machinery they drive and the measurements behind each route it closes,
//! split so that each file stays inside the 400-line module limit:
//!
//! - [`discovery`] decides which roots are governed and reads every Rust file
//!   under them;
//! - [`meta`] judges one parsed attribute;
//! - [`tokens`] reaches the attributes a parsed `Meta` never describes.
//!
//! The other contracts check the configuration, prove the lint fires, and
//! hold the build wiring. None of them sees a source file that switches the
//! lint off for itself. A crate-level inner attribute does exactly that, and
//! `clippy::allow_attributes` does not fire on inner attributes, so nothing
//! else in the repository notices.
//!
//! The file is parsed rather than searched. A text scan cannot tell an
//! attribute from attribute-shaped text in a string or a doc comment, cannot
//! follow `cfg_attr`, and breaks on a parenthesis inside a `reason`. This
//! replaces the line-start scan that shipped with the policy, which every one
//! of the measurements below would have walked past.
//!
//! Measured against Clippy 0.1.98 on this crate's `clippy.toml`, each probe a
//! single file with one unannotated `std::env::var` call. The baseline reports
//! one diagnostic:
//!
//! | Attribute | Diagnostics |
//! | --- | --- |
//! | none | 1 |
//! | `#![allow(clippy::style)]` | 0 |
//! | `#![allow(clippy::all)]` | 0 |
//! | `#![cfg_attr(all(), allow(clippy::disallowed_methods))]` | 0 |
//! | `#![allow(warnings)]`, lint denied on the command line | 1 |
//! | `#![allow(warnings)]`, lint at its configured level | 0 |
//!
//! Two things follow. Naming the lint is not required to silence it: Clippy
//! places `disallowed_methods` in the `style` group, so `clippy::style` and
//! the wider `clippy::all` each switch the policy off without ever writing its
//! name. And `warnings` does not evade the policy lane, which denies the lint
//! explicitly on the command line, but it does evade a lane that leaves the
//! lint at its configured level. It is protected here as insurance against the
//! manifest severity ever softening, not on the strength of a measured escape.
//!
//! `expect` is judged by scope rather than exempted. An item-scoped
//! `#[expect(..., reason = "...")]` is the sanctioned form for a composition
//! root precisely because it warns once the site grows a seam, and this scan
//! leaves it alone. A crate-scoped `#![expect(...)]` is not that form: one
//! call anywhere in the crate fulfils it, every other call goes unreported,
//! and no unfulfilled expectation is raised, so nothing is left to notice.
//! Measured: `#![expect(clippy::disallowed_methods)]` takes the probe from one
//! diagnostic to none and raises nothing in its place. The two spellings
//! differ by one character, and the quieter one is the evasion.
//!
//! Raw identifiers are the same identifiers. `#![r#allow(...)]` and
//! `clippy::r#style` each silence the lint, so paths are normalized before
//! they are compared.
//!
//! Two shapes are refused structurally rather than by their meta, because
//! neither is a complete attribute where it is written. A `macro_rules!` arm
//! may forward an attribute's path (`#[$attr]`) and let its caller supply
//! `allow`; and `rustc` parses an `include!` target as Rust whatever its
//! extension, so an `allow` inside a `.rs.txt` fixture reaches the compiler
//! from a file the scan cannot read. Both are documented in
//! `docs/adr-006-environment-seam-taxonomy.md`, with the narrowing that keeps
//! the doc-forwarding idiom and `include_str!` out of the findings.

use syn::visit::Visit;

// Paths are relative to this file's own directory, `tests/test_utils/`.
#[path = "source_scan/discovery.rs"]
pub(crate) mod discovery;
#[path = "source_scan/inclusion.rs"]
mod inclusion;
#[path = "source_scan/meta.rs"]
mod meta;
#[path = "source_scan/tokens.rs"]
mod tokens;

/// The extension a file must carry for the traversal to collect it.
///
/// Shared with the `include!` rule in [`tokens`], which has to judge an
/// inclusion target by the same standard the walk selects sources by. Written
/// once so the two cannot drift: a target the walk would not collect is a file
/// the scan never reads, whatever the inclusion looks like.
pub(crate) const SOURCE_EXTENSION: &str = "rs";

pub(crate) use discovery::{SOURCE_ROOTS, crate_dir, crate_sources, is_walkable, rust_sources};

use camino::Utf8Path;

use meta::suppressed_by;
use tokens::AttributeCollector;

/// Lints whose suppression disarms the environment-access policy.
///
/// Naming the lint alone is not enough, per the measurements above. Extend
/// this if the lint's group ever changes.
pub(crate) const PROTECTED_LINTS: [&str; 4] = [
    "clippy::disallowed_methods",
    "clippy::style",
    "clippy::all",
    "warnings",
];

/// Return every protected lint suppressed in one source file.
///
/// `path` is the file's path relative to the crate directory. It is needed
/// because `rustc` resolves an `include!` against the file that writes it, so
/// an inclusion cannot be judged without knowing where it was written. Inline
/// fixtures pass the path they are pretending to be.
///
/// Lint names are compared as whole paths, never as substrings. An earlier
/// draft compared by substring and would have reported
/// `#[allow(clippy::allow_attributes)]` as suppressing `clippy::all`, whose
/// name it contains.
pub(crate) fn suppressed_lints(path: &Utf8Path, contents: &str) -> Result<Vec<String>, String> {
    let parsed = syn::parse_file(contents).map_err(|error| format!("parse: {error}"))?;
    let mut collector = AttributeCollector {
        path: path.to_owned(),
        ..AttributeCollector::default()
    };
    collector.visit_file(&parsed);

    let mut found = collector.structural.clone();
    for (rendered, meta, inner) in &collector.attributes {
        for lint in suppressed_by(meta, *inner) {
            if PROTECTED_LINTS.contains(&lint.as_str()) {
                found.push(format!("suppresses {lint} via {rendered}"));
            }
        }
    }
    Ok(found)
}
