//! Source scan closing the attribute routes around the environment policy.
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

use std::collections::VecDeque;

use camino::{Utf8Path, Utf8PathBuf};
use cap_std::{ambient_authority, fs_utf8::Dir};
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::{
    AttrStyle, Attribute, Macro, Meta, MetaList, Path, Token, ext::IdentExt,
    punctuated::Punctuated, visit::Visit,
};

/// Lints whose suppression disarms the environment-access policy.
///
/// Naming the lint alone is not enough, per the measurements above. Extend
/// this if the lint's group ever changes.
const PROTECTED_LINTS: [&str; 4] = [
    "clippy::disallowed_methods",
    "clippy::style",
    "clippy::all",
    "warnings",
];

/// Directories under the crate holding Rust sources the policy governs.
const SOURCE_ROOTS: [&str; 3] = ["src", "tests", "benches"];

/// Return the crate directory, which holds every governed source.
fn crate_dir() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Collect every `.rs` file under one root, breadth first.
///
/// Read through a `cap_std` directory handle rather than `std::fs`, which the
/// repository's Dylint suite disallows: the scan needs to see files that do
/// not exist yet, so `include_str!` is not an option here as it is elsewhere
/// in this suite.
fn rust_sources(root: &Utf8Path, relative: &str) -> Result<Vec<(Utf8PathBuf, String)>, String> {
    let Ok(directory) = Dir::open_ambient_dir(root, ambient_authority()) else {
        return Ok(Vec::new());
    };
    let mut pending = VecDeque::from([(directory, Utf8PathBuf::from(relative))]);
    let mut sources = Vec::new();

    while let Some((current, prefix)) = pending.pop_front() {
        let entries = current
            .entries()
            .map_err(|error| format!("read {prefix}: {error}"))?;
        for candidate in entries {
            let entry = candidate.map_err(|error| format!("entry under {prefix}: {error}"))?;
            let name = entry
                .file_name()
                .map_err(|error| format!("name under {prefix}: {error}"))?;
            let path = prefix.join(&name);
            let file_type = entry
                .file_type()
                .map_err(|error| format!("type of {path}: {error}"))?;
            if file_type.is_dir() {
                let child = current
                    .open_dir(&name)
                    .map_err(|error| format!("open {path}: {error}"))?;
                pending.push_back((child, path));
            } else if path.extension() == Some("rs") {
                let contents = current
                    .read_to_string(&name)
                    .map_err(|error| format!("read {path}: {error}"))?;
                sources.push((path, contents));
            }
        }
    }
    Ok(sources)
}

/// Collect every attribute in a parsed file, wherever it sits.
///
/// A visitor is used rather than a hand-rolled walk so attributes on nested
/// items, on function-local items and on expressions are all reached.
///
/// Macro bodies are walked too, as token streams. `syn` keeps the body of a
/// `macro_rules!` arm opaque, so an attribute written there never reaches
/// `visit_attribute`, and Clippy expands and honours it. Measured on this
/// crate's `clippy.toml`: a macro arm emitting
/// `#[allow(clippy::disallowed_methods)]` around a function that calls
/// `std::env::var` reports zero diagnostics, where the same file without the
/// attribute reports one.
#[derive(Default)]
struct AttributeCollector {
    /// Each suppression found: the text to report, the meta to judge, and
    /// whether it was written at inner scope.
    attributes: Vec<(String, Meta, bool)>,
}

impl<'ast> Visit<'ast> for AttributeCollector {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        self.attributes.push((
            render_attribute(attribute),
            attribute.meta.clone(),
            matches!(attribute.style, AttrStyle::Inner(_)),
        ));
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        collect_from_tokens(mac.tokens.clone(), &mut self.attributes);
        syn::visit::visit_macro(self, mac);
    }
}

/// Collect attribute-shaped token sequences from a macro body.
///
/// An attribute is `#`, optionally `!`, then a bracketed group. Recursing
/// through every group reaches an attribute at any depth, including one inside
/// a nested macro. A token walk cannot mistake prose for policy the way a text
/// scan can: a string literal is one token, never a `#` followed by brackets.
fn collect_from_tokens(stream: TokenStream, found: &mut Vec<(String, Meta, bool)>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        if let TokenTree::Group(group) = token {
            collect_from_tokens(group.stream(), found);
        }
        if !matches!(token, TokenTree::Punct(punct) if punct.as_char() == '#') {
            continue;
        }
        let mut next = index + 1;
        let mut bang = "";
        if matches!(tokens.get(next), Some(TokenTree::Punct(punct)) if punct.as_char() == '!') {
            bang = "!";
            next += 1;
        }
        let Some(TokenTree::Group(group)) = tokens.get(next) else {
            continue;
        };
        if group.delimiter() != Delimiter::Bracket {
            continue;
        }
        if let Ok(meta) = syn::parse2::<Meta>(group.stream()) {
            found.push((
                format!("#{bang}[{}]", group.stream()),
                meta,
                !bang.is_empty(),
            ));
        }
    }
}

/// Render a lint path with raw identifiers normalized.
///
/// `r#allow` is `allow` and `clippy::r#style` is `clippy::style`; Clippy
/// honours both spellings, so comparing the written form would let either
/// through. Measured: `#![r#allow(clippy::disallowed_methods)]` and
/// `#![allow(clippy::r#style)]` each reduce the probe from one diagnostic to
/// none.
fn render_path(path: &Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.unraw().to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// Return the lint names an `allow` meta-list suppresses.
///
/// Key-value arguments such as `reason = "..."` are not lint names and are
/// skipped.
fn allowed_lints(list: &MetaList) -> Vec<String> {
    let Ok(nested) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        return Vec::new();
    };
    nested
        .iter()
        .filter_map(|meta| match meta {
            Meta::Path(path) => Some(render_path(path)),
            Meta::List(_) | Meta::NameValue(_) => None,
        })
        .collect()
}

/// Return the lint names one attribute suppresses, following `cfg_attr`.
///
/// `inner` is the scope of the attribute this began at, and a `cfg_attr`
/// carries it down: `#![cfg_attr(all(), expect(...))]` is crate-scoped however
/// deeply the nesting runs.
///
/// `expect` is judged by that scope rather than exempted outright. An
/// item-scoped `#[expect(..., reason = "...")]` is the sanctioned form and is
/// left alone; a crate-scoped `#![expect(...)]` is not, because one call
/// anywhere in the crate fulfils it and the rest go unreported. Measured:
/// `#![expect(clippy::disallowed_methods)]` reports neither the disallowed
/// method nor an unfulfilled expectation, so nothing at all is left to notice.
fn suppressed_by(meta: &Meta, inner: bool) -> Vec<String> {
    let Ok(list) = meta.require_list() else {
        return Vec::new();
    };
    match render_path(meta.path()).as_str() {
        "allow" => allowed_lints(list),
        "expect" if inner => allowed_lints(list),
        "cfg_attr" => suppressed_by_cfg_attr(list, inner),
        _ => Vec::new(),
    }
}

/// Return the lint names nested inside a `cfg_attr`.
///
/// The condition is followed whatever it says. A suppression that applies
/// under some configuration is still a suppression, and deciding which
/// configurations are reachable is not this contract's job.
fn suppressed_by_cfg_attr(list: &MetaList, inner: bool) -> Vec<String> {
    let Ok(nested) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        return Vec::new();
    };
    nested
        .iter()
        .skip(1)
        .filter_map(|meta| match meta {
            Meta::List(nested_list) => Some(match render_path(&nested_list.path).as_str() {
                "allow" => allowed_lints(nested_list),
                "expect" if inner => allowed_lints(nested_list),
                "cfg_attr" => suppressed_by_cfg_attr(nested_list, inner),
                _ => Vec::new(),
            }),
            Meta::Path(_) | Meta::NameValue(_) => None,
        })
        .flatten()
        .collect()
}

/// Render an attribute roughly as written, for a failure message.
fn render_attribute(attribute: &Attribute) -> String {
    let bang = match attribute.style {
        AttrStyle::Inner(_) => "!",
        AttrStyle::Outer => "",
    };
    let path = render_path(attribute.path());
    attribute.meta.require_list().map_or_else(
        |_| format!("#{bang}[{path}]"),
        |list| format!("#{bang}[{path}({})]", list.tokens),
    )
}

/// Return every protected lint suppressed in one source file.
///
/// Lint names are compared as whole paths, never as substrings. An earlier
/// draft compared by substring and would have reported
/// `#[allow(clippy::allow_attributes)]` as suppressing `clippy::all`, whose
/// name it contains.
fn suppressed_lints(contents: &str) -> Result<Vec<(String, String)>, String> {
    let parsed = syn::parse_file(contents).map_err(|error| format!("parse: {error}"))?;
    let mut collector = AttributeCollector::default();
    collector.visit_file(&parsed);

    let mut found = Vec::new();
    for (rendered, meta, inner) in &collector.attributes {
        for lint in suppressed_by(meta, *inner) {
            if PROTECTED_LINTS.contains(&lint.as_str()) {
                found.push((lint, rendered.clone()));
            }
        }
    }
    Ok(found)
}

/// Scenario: a source file switches the policy lint off for itself.
///
/// Invariant: no Rust source suppresses a protected lint, by any spelling and
/// through any `cfg_attr`. Suppression means an `allow` at any scope or an
/// `expect` at crate scope; an item-scoped reasoned `expect` is the sanctioned
/// form and passes. An inner attribute is the case that matters, because
/// `clippy::allow_attributes` cannot see one, so nothing else in the
/// repository would notice.
///
/// Mutation proof (2026-09-08); each applied alone to a real source file, run
/// through the build, and reverted:
///
/// - `#![allow(clippy::disallowed_methods)]` in `src/lib.rs` fails this test;
/// - `#![allow(clippy::style)]`, naming the group rather than the lint, fails;
/// - `#![cfg_attr(all(), allow(clippy::disallowed_methods))]` fails;
/// - `#![allow(clippy::all)]` spread over several lines fails;
/// - `#[allow(warnings)]` on an item fails;
/// - a `macro_rules!` arm emitting `#[allow(clippy::disallowed_methods)]`
///   around a call to `std::env::var` fails, which is the route both reviewers
///   found: Clippy expands and honours it, reporting zero diagnostics where the
///   same file without the attribute reports one;
/// - `#![expect(clippy::disallowed_methods)]` at crate scope fails, since one
///   call fulfils it crate-wide and no unfulfilled expectation is raised;
/// - `#![r#allow(...)]` and `#![allow(clippy::r#style)]` fail, raw identifiers
///   being the same identifiers;
/// - `#[expect(clippy::disallowed_methods, reason = "...")]` on an item must,
///   and does, keep passing: it is the sanctioned form;
/// - `#[allow(clippy::allow_attributes)]` must, and does, keep passing.
#[test]
fn no_source_file_suppresses_a_policy_lint() -> Result<(), String> {
    let crate_dir = crate_dir();
    let mut offences = Vec::new();
    for root in SOURCE_ROOTS {
        for (path, contents) in rust_sources(&crate_dir.join(root), root)? {
            for (lint, attribute) in
                suppressed_lints(&contents).map_err(|error| format!("{path}: {error}"))?
            {
                offences.push(format!("{path} suppresses {lint} via {attribute}"));
            }
        }
    }
    if offences.is_empty() {
        return Ok(());
    }
    Err(format!(
        "no Rust source may switch the environment-access policy off; use an \
         item-scoped expect with a reason instead, which is the one sanctioned \
         form: {}",
        offences.join("; ")
    ))
}

/// Scenario: the scan meets shapes that are not suppressions.
///
/// Invariant: it reports none of them. A lint name inside a string or a doc
/// comment is discussion, an `expect` is the sanctioned form, and
/// `clippy::allow_attributes` merely contains the text of `clippy::all`.
#[test]
fn the_scan_reports_neither_prose_nor_the_sanctioned_form() -> Result<(), String> {
    let benign = r##"
//! A module doc comment mentioning #![allow(clippy::all)] for illustration.
const NOTE: &str = "#![allow(clippy::disallowed_methods)]";
#[expect(clippy::disallowed_methods, reason = "composition root (see ADR)")]
fn root() { let _ = std::env::var("X"); }
#[allow(clippy::allow_attributes)]
fn tolerated() {}
#[expect(clippy::disallowed_methods, reason = "item-scoped is the sanctioned form")]
fn also_root() { let _ = std::env::var("Y"); }
#[allow(clippy::alloc_instead_of_core)]
fn different_lint_whose_name_starts_with_clippy_all() {}
"##;
    let found = suppressed_lints(benign)?;
    if found.is_empty() {
        return Ok(());
    }
    Err(format!(
        "the scan must report none of these, found {found:?}"
    ))
}

/// Scenario: a suppression is written in a shape a text scan cannot see.
///
/// Invariant: each is reported. These are the measured evasions, kept as unit
/// cases so the parser's behaviour is pinned without editing a real source
/// file, alongside the mutation proofs that do edit one.
#[test]
fn the_scan_follows_groups_and_cfg_attr() -> Result<(), String> {
    for source in [
        "#![allow(clippy::style)]",
        "#![allow(clippy::all)]",
        "#![allow(warnings)]",
        "#![cfg_attr(all(), allow(clippy::disallowed_methods))]",
        "#![cfg_attr(unix, cfg_attr(all(), allow(clippy::style)))]",
        r##"#![allow(clippy::disallowed_methods, reason = "a reason with (parentheses)")]"##,
        "#[allow(clippy::all)]\nfn wrapped() {}",
        // A macro arm's body is an opaque token stream to `syn`, but Clippy
        // expands it and honours the attribute.
        concat!(
            "macro_rules! bypass { () => {\n",
            "    #[allow(clippy::disallowed_methods)]\n",
            "    pub fn ambient() { let _ = std::env::var(\"X\"); }\n",
            "}; }\nbypass!();"
        ),
        // Crate-scoped `expect`: one call fulfils it crate-wide, so nothing is
        // reported and no unfulfilled expectation is raised either.
        "#![expect(clippy::disallowed_methods)]",
        "#![cfg_attr(all(), expect(clippy::disallowed_methods))]",
        // Raw identifiers, in the attribute name and in the lint path.
        "#![r#allow(clippy::disallowed_methods)]",
        "#![allow(clippy::r#style)]",
        // Nested one macro deeper, to show the walk recurses rather than
        // peeking one level.
        concat!(
            "macro_rules! outer { () => {\n",
            "    macro_rules! inner { () => { #[allow(clippy::style)] fn f() {} }; }\n",
            "}; }"
        ),
    ] {
        if suppressed_lints(source)?.is_empty() {
            return Err(format!("the scan must report {source:?}"));
        }
    }
    Ok(())
}
