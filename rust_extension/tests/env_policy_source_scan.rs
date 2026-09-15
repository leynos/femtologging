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
use proptest::prelude::*;
use rstest::rstest;
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
///
/// A root that cannot be opened is an error, not an empty result. An earlier
/// draft returned `Ok(Vec::new())` there, which made a renamed, deleted or
/// unreadable governed directory indistinguishable from one holding no
/// suppression: the scan reported success having read nothing.
fn rust_sources(root: &Utf8Path, relative: &str) -> Result<Vec<(Utf8PathBuf, String)>, String> {
    let directory = Dir::open_ambient_dir(root, ambient_authority())
        .map_err(|error| format!("open {relative}: {error}"))?;
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
    /// Each finding no meta describes, already worded as a report line.
    structural: Vec<String>,
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
        match render_path(&mac.path).rsplit("::").next() {
            Some("macro_rules") => {
                for (pattern, transcriber) in macro_arms(mac.tokens.clone()) {
                    let reachable = could_cover_a_policy_call(&pattern, &transcriber);
                    collect_from_tokens(transcriber, reachable, self);
                }
            }
            Some("include") => {
                if let Some(finding) = foreign_inclusion(&mac.tokens) {
                    self.structural.push(finding);
                }
            }
            _ => {}
        }
        syn::visit::visit_macro(self, mac);
    }
}

/// Return each arm of a `macro_rules!` body as its pattern and transcriber.
///
/// Only a transcriber is expanded, so only a transcriber can suppress
/// anything. The arms' patterns are not output, and the arguments of an
/// ordinary macro invocation may be discarded by the macro it is handed to:
/// walking either reports an attribute that never reaches the compiler, and a
/// contract that reports a false positive gets switched off.
///
/// The pattern comes back with it because the fragment specifiers declared
/// there decide whether a forwarded attribute in the transcriber could bear on
/// the policy.
///
/// An arm is `(pattern) => {transcriber};`, so each group following a `=>` is
/// a transcriber and the group before the `=>` is its pattern.
fn macro_arms(stream: TokenStream) -> Vec<(TokenStream, TokenStream)> {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut found = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token, TokenTree::Punct(punct) if punct.as_char() == '=') {
            continue;
        }
        if !matches!(tokens.get(index + 1), Some(TokenTree::Punct(punct)) if punct.as_char() == '>')
        {
            continue;
        }
        let Some(TokenTree::Group(transcriber)) = tokens.get(index + 2) else {
            continue;
        };
        let pattern = match index.checked_sub(1).and_then(|before| tokens.get(before)) {
            Some(TokenTree::Group(group)) => group.stream(),
            _ => TokenStream::new(),
        };
        found.push((pattern, transcriber.stream()));
    }
    found
}

/// Fragment specifiers whose value can carry an environment access.
///
/// A caller supplying one of these supplies code, so an attribute forwarded
/// over it can cover a call the arm never mentions. An `ident`, a `ty`, a
/// `lifetime` or a `literal` cannot carry a call, which is what keeps the
/// doc-forwarding idiom below out of the findings.
const CODE_FRAGMENTS: [&str; 5] = ["item", "block", "stmt", "expr", "tt"];

/// Return whether `stream` names the `env` module at any depth.
///
/// The call the arm writes sits inside the item's block, which is one group
/// down, so the search recurses. Reading only the top level found nothing and
/// let the route through.
fn mentions_env(stream: &TokenStream) -> bool {
    stream.clone().into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident == "env",
        TokenTree::Group(group) => mentions_env(&group.stream()),
        TokenTree::Punct(_) | TokenTree::Literal(_) => false,
    })
}

/// Return whether an arm could put a forwarded attribute over a policy call.
///
/// Either the arm writes the access itself, in which case `env` appears among
/// its tokens, or it forwards a fragment that the caller fills with code.
/// `$(#[$meta:meta])* $name:ident, $field:ident, $ty:ty` does neither: it is
/// the ordinary way to carry doc comments onto a generated setter, it appears
/// twice in this crate's own builders, and reporting it would be the false
/// positive that gets a contract switched off.
fn could_cover_a_policy_call(pattern: &TokenStream, transcriber: &TokenStream) -> bool {
    if mentions_env(transcriber) {
        return true;
    }
    let tokens: Vec<TokenTree> = pattern.clone().into_iter().collect();
    tokens.iter().enumerate().any(|(index, token)| {
        matches!(token, TokenTree::Punct(punct) if punct.as_char() == ':')
            && matches!(
                tokens.get(index + 1),
                Some(TokenTree::Ident(ident))
                    if CODE_FRAGMENTS.contains(&ident.to_string().as_str())
            )
    })
}

/// Return a finding if an `include!` names a target that is not Rust source.
///
/// `rustc` parses an included file as Rust whatever its extension, so
/// `include!("fixture.rs.txt")` compiles the fixture's contents into this
/// crate. An `allow` written there suppresses the policy for the calls around
/// it, and an enclosing `expect` stays fulfilled, so nothing warns. The scan
/// cannot read the target, because the target need not exist when the scan
/// runs, so the inclusion itself is the finding.
///
/// A literal `.rs` path is not a finding: such a file is scanned in its own
/// right, being a `.rs` file under a governed root. `include_str!` and
/// `include_bytes!` are not source inclusion at all and never reach here.
fn foreign_inclusion(tokens: &TokenStream) -> Option<String> {
    let rendered = tokens.to_string();
    let target = tokens.clone().into_iter().next().and_then(|token| {
        let TokenTree::Literal(literal) = token else {
            return None;
        };
        let text = literal.to_string();
        let trimmed = text.strip_prefix('"')?.strip_suffix('"')?;
        Some(trimmed.to_owned())
    });
    match target {
        Some(path) if path.ends_with(".rs") => None,
        Some(path) => Some(format!(
            "include!(\"{path}\") compiles a file the scan cannot see as Rust; \
             name a `.rs` path, which is scanned in its own right"
        )),
        None => Some(format!(
            "include!({rendered}) names a target the scan cannot resolve; \
             name a literal `.rs` path, which is scanned in its own right"
        )),
    }
}

/// Collect attribute-shaped token sequences from a `macro_rules!` transcriber.
///
/// An attribute is `#`, optionally `!`, then a bracketed group. Recursing
/// through every group reaches an attribute at any depth, including one inside
/// a nested macro. A token walk cannot mistake prose for policy the way a text
/// scan can: a string literal is one token, never a `#` followed by brackets.
fn collect_from_tokens(stream: TokenStream, reachable: bool, collector: &mut AttributeCollector) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        if let TokenTree::Group(group) = token {
            collect_from_tokens(group.stream(), reachable, collector);
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
        if let Some(finding) = forwarded_path(&group.stream(), bang, reachable) {
            collector.structural.push(finding);
        } else if let Ok(meta) = syn::parse2::<Meta>(group.stream()) {
            collector.attributes.push((
                format!("#{bang}[{}]", group.stream()),
                meta,
                !bang.is_empty(),
            ));
        }
    }
}

/// Return a finding if an attribute in a transcriber forwards its own path.
///
/// `#[$attr]` is written by the arm and completed by the caller, so the arm
/// cannot be read for what it applies and the invocation carries no `#` for a
/// walk to notice. Neither half is a suppression on its own, and together they
/// are: invoked as `forward!(allow(clippy::disallowed_methods), ...)`, the
/// expansion silences every call the item contains.
///
/// Two things keep the rule narrow. Only a forwarded *path* is refused: an
/// attribute whose path is written out cannot become `allow`, however much of
/// its argument is forwarded, so `#[doc = $text]` and `#[derive($traits)]` are
/// judged as any other attribute and report nothing. And an outer forwarded
/// path is refused only where the arm could put it over a policy call, which
/// leaves the `$(#[$meta:meta])*` doc-forwarding idiom alone.
///
/// An inner forwarded path is refused wherever it appears. `#![$attr]` applies
/// to everything enclosing it rather than to one item, so there is no call it
/// could fail to cover, and nothing to weigh.
fn forwarded_path(stream: &TokenStream, bang: &str, reachable: bool) -> Option<String> {
    let mut tokens = stream.clone().into_iter();
    let first = tokens.next()?;
    if !matches!(&first, TokenTree::Punct(punct) if punct.as_char() == '$') {
        return None;
    }
    let inner = !bang.is_empty();
    if !inner && !reachable {
        return None;
    }
    Some(format!(
        "#{bang}[{stream}] forwards its own path, which the caller can complete \
         with `allow`; write the attribute out, or take the item rather than \
         the attribute"
    ))
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
        .flat_map(|meta| suppressed_by(meta, inner))
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
fn suppressed_lints(contents: &str) -> Result<Vec<String>, String> {
    let parsed = syn::parse_file(contents).map_err(|error| format!("parse: {error}"))?;
    let mut collector = AttributeCollector::default();
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
///
/// Re-proved on 2026-09-14, after `suppressed_by_cfg_attr` was folded back
/// into `suppressed_by` rather than repeating its dispatch: the `cfg_attr`
/// mutation above still fails this test through the build.
///
/// Two further routes were closed on 2026-09-14, each measured elsewhere
/// against Clippy before being closed here, and each proved in both
/// directions through the build. A rule that reaches nothing and a rule that
/// reaches everything both pass a one-sided proof, and the second gets the
/// contract switched off:
///
/// - the forwarded-path rule never firing fails the three cases that
///   exercise it;
/// - refusing every forwarded path, rather than only a reachable or inner
///   one, fails the benign fixture and this test, on the
///   `$(#[$meta:meta])*` doc-forwarding idiom in `src/handlers/`;
/// - refusing any metavariable anywhere in the attribute, rather than in its
///   path, fails on `#[doc = $doc]` and on a forwarded `#[pyo3(name = ...)]`;
/// - accepting `.txt` alongside `.rs` as an `include!` target fails the case
///   that exercises it;
/// - judging `include_str!` as source inclusion fails this test on the three
///   real `include_str!` calls in the crate, and the benign fixture with it.
#[test]
fn no_source_file_suppresses_a_policy_lint() -> Result<(), String> {
    let crate_dir = crate_dir();
    let mut offences = Vec::new();
    for root in SOURCE_ROOTS {
        for (path, contents) in rust_sources(&crate_dir.join(root), root)? {
            for finding in
                suppressed_lints(&contents).map_err(|error| format!("{path}: {error}"))?
            {
                offences.push(format!("{path} {finding}"));
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

/// A source each governed root must yield, to prove the walk reached it.
///
/// One per root, and the `src` entry is nested so a walk that read only a
/// root's immediate children would fail rather than pass on `lib.rs`.
const EXPECTED_SOURCES: [(&str, &str); 3] = [
    ("src", "src/config/mod.rs"),
    ("tests", "tests/env_policy_source_scan.rs"),
    ("benches", "benches/config.rs"),
];

/// Scenario: the scan's source discovery is asked what it actually read.
///
/// Invariant: every governed root opens, yields at least one `.rs` file, and
/// contains the nested source named for it, whose contents are non-empty.
/// Without this, [`no_source_file_suppresses_a_policy_lint`] has a vacuous
/// path: it fails only on a non-empty offence list, so a root that yielded
/// nothing would report success having read nothing at all.
///
/// Mutation proof (2026-09-14), each applied alone, run through the build and
/// reverted:
///
/// - renaming `SOURCE_ROOTS`' `benches` entry to `benchmarks` fails
///   [`no_source_file_suppresses_a_policy_lint`] with
///   `open benchmarks: No such file or directory`. Before [`rust_sources`]
///   stopped swallowing the open failure, the same mutation passed;
/// - stopping the walk from descending into subdirectories fails this test's
///   `src` case with `src did not yield src/config/mod.rs`, and nothing else
///   in the file notices. That is why the named source is nested rather than
///   `src/lib.rs`.
#[rstest]
#[case::src(0)]
#[case::tests(1)]
#[case::benches(2)]
fn every_governed_root_yields_its_sources(#[case] index: usize) -> Result<(), String> {
    let (root, expected) = EXPECTED_SOURCES[index];
    let sources = rust_sources(&crate_dir().join(root), root)?;
    if sources.is_empty() {
        return Err(format!("{root} yielded no Rust source"));
    }
    let found = sources
        .iter()
        .find(|(path, _)| path.as_str() == expected)
        .ok_or_else(|| format!("{root} did not yield {expected}"))?;
    if found.1.is_empty() {
        return Err(format!("{expected} was read as empty"));
    }
    Ok(())
}

/// Scenario: the scan meets shapes that are not suppressions.
///
/// Invariant: it reports none of them. A lint name inside a string or a doc
/// comment is discussion, an `expect` is the sanctioned form, and
/// `clippy::allow_attributes` merely contains the text of `clippy::all`.
///
/// The last six are the narrowness half of the two routes closed alongside
/// them, and they matter as much as the reach half: a contract that reports a
/// false positive gets switched off, and then it reports nothing at all.
/// `#[doc = $text]` and `#[derive($traits)]` forward an argument but write
/// their own path, which cannot become `allow`; `option_setter` forwards the
/// path itself, in the ordinary idiom for carrying doc comments onto a
/// generated setter, but over an `ident`, an `ident` and a `ty`, none of which
/// can carry a call, and this crate's own builders use it twice; a transcriber
/// emitting
/// `#[allow(dead_code, ...)]` names no protected lint; `include_str!` and
/// `include_bytes!` embed bytes rather than compiling source; and `include!`
/// of a literal `.rs` path names a file the scan reads in its own right.
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
macro_rules! documented { ($text:expr) => { #[doc = $text] pub fn described() {} }; }
macro_rules! option_setter {
    ($(#[$meta:meta])* $name:ident, $field:ident, $ty:ty) => {
        $(#[$meta])*
        pub fn $name(mut self, value: $ty) -> Self { self.$field = Some(value); self }
    };
}
macro_rules! derived { ($traits:path) => { #[derive($traits)] pub struct Held; }; }
macro_rules! generated { () => { #[allow(dead_code, reason = "generated")] fn unused() {} }; }
const EMBEDDED: &str = include_str!("fixtures/env_policy_probe.rs.txt");
const BYTES: &[u8] = include_bytes!("fixtures/table.dat");
include!("fixtures/generated.rs");
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
/// Invariant: each is reported. These are the measured evasions, kept as
/// cases so the parser's behaviour is pinned without editing a real source
/// file, alongside the mutation proofs that do edit one. One case per
/// spelling, so a regression names the spelling that regressed rather than
/// stopping at the first.
#[rstest]
#[case::lint_group("#![allow(clippy::style)]")]
#[case::wider_group("#![allow(clippy::all)]")]
#[case::warnings("#![allow(warnings)]")]
#[case::cfg_attr("#![cfg_attr(all(), allow(clippy::disallowed_methods))]")]
#[case::nested_cfg_attr("#![cfg_attr(unix, cfg_attr(all(), allow(clippy::style)))]")]
#[case::reason_with_parentheses(
    r##"#![allow(clippy::disallowed_methods, reason = "a reason with (parentheses)")]"##
)]
#[case::item_scoped("#[allow(clippy::all)]\nfn wrapped() {}")]
// A macro arm's body is an opaque token stream to `syn`, but Clippy expands
// it and honours the attribute.
#[case::macro_arm(concat!(
    "macro_rules! bypass { () => {\n",
    "    #[allow(clippy::disallowed_methods)]\n",
    "    pub fn ambient() { let _ = std::env::var(\"X\"); }\n",
    "}; }\nbypass!();"
))]
// Crate-scoped `expect`: one call fulfils it crate-wide, so nothing is
// reported and no unfulfilled expectation is raised either.
#[case::crate_scoped_expect("#![expect(clippy::disallowed_methods)]")]
#[case::cfg_attr_crate_scoped_expect("#![cfg_attr(all(), expect(clippy::disallowed_methods))]")]
// Raw identifiers, in the attribute name and in the lint path.
#[case::raw_attribute_name("#![r#allow(clippy::disallowed_methods)]")]
#[case::raw_lint_path("#![allow(clippy::r#style)]")]
// Nested one macro deeper, to show the walk recurses rather than peeking one
// level.
#[case::macro_within_macro(concat!(
    "macro_rules! outer { () => {\n",
    "    macro_rules! inner { () => { #[allow(clippy::style)] fn f() {} }; }\n",
    "}; }"
))]
// A transcriber that writes the attribute but forwards its path. The arm
// cannot be read for what it applies, and the invocation carries no `#`, so
// neither half is a suppression alone.
#[case::forwarded_attribute_path(concat!(
    "macro_rules! forward { ($attr:meta) => {\n",
    "    #[$attr]\n",
    "    pub fn ambient() { let _ = std::env::var(\"X\"); }\n",
    "}; }\nforward!(allow(clippy::disallowed_methods));"
))]
// The same forwarding at inner scope, which is the quieter half of the route.
#[case::forwarded_inner_attribute_path("macro_rules! forward { ($attr:meta) => { #![$attr] }; }")]
// Forwarding over an item the caller supplies: the arm names no `env` itself,
// but whatever it is handed comes under the forwarded attribute.
#[case::forwarded_over_a_supplied_item(
    "macro_rules! forward { ($attr:meta, $body:item) => { #[$attr] $body }; }"
)]
// `include!` of a target the scan cannot see: rustc parses it as Rust whatever
// the extension, so an `allow` written there reaches the compiler.
#[case::include_of_a_foreign_extension("include!(\"fixtures/probe.rs.txt\");")]
// An `include!` whose target is not a literal cannot be judged at all.
#[case::include_of_a_computed_path("include!(concat!(env!(\"OUT_DIR\"), \"/probe\"));")]
fn the_scan_follows_groups_and_cfg_attr(#[case] source: &str) -> Result<(), String> {
    if suppressed_lints(source)?.is_empty() {
        return Err(format!("the scan must report {source:?}"));
    }
    Ok(())
}

/// How a suppression is wrapped before the scan sees it.
///
/// Each variant is a route measured against Clippy and closed by the scan.
/// Generating them rather than listing them is the point: the invariant is
/// over the shapes, not over the thirteen spellings the case list pins.
#[derive(Clone, Debug)]
enum Wrapping {
    /// The attribute as written.
    Bare,
    /// Nested in `cfg_attr` to the given depth, at least one level.
    CfgAttr(u8),
    /// Emitted from a `macro_rules!` arm, nested to the given depth.
    Macro(u8),
}

/// Render an `allow` of `lint`, inner or outer, raw-identified or not.
fn allow_attribute(lint: &str, inner: bool, raw: bool) -> String {
    let keyword = if raw { "r#allow" } else { "allow" };
    let bang = if inner { "!" } else { "" };
    format!("#{bang}[{keyword}({lint})]")
}

/// Nest an inner attribute's contents in `depth` levels of `cfg_attr`.
fn nested_in_cfg_attr(attribute: &str, depth: u8) -> String {
    let mut rendered = attribute
        .trim_start_matches("#![")
        .trim_end_matches(']')
        .to_owned();
    for _ in 0..depth {
        rendered = format!("cfg_attr(all(), {rendered})");
    }
    format!("#![{rendered}]\n")
}

/// Put an outer attribute in `depth` levels of `macro_rules!` arm.
fn nested_in_macro(attribute: &str, depth: u8) -> String {
    let mut rendered = format!("{attribute} fn probe() {{}}");
    for level in 0..depth {
        rendered = format!("macro_rules! m{level} {{ () => {{ {rendered} }}; }}");
    }
    format!("{rendered}\n")
}

/// Render `attribute` wrapped as `wrapping` says, as a whole source file.
fn wrapped_source(attribute: &str, wrapping: &Wrapping) -> String {
    match wrapping {
        Wrapping::Bare => format!("{attribute}\n"),
        Wrapping::CfgAttr(depth) => nested_in_cfg_attr(attribute, *depth),
        Wrapping::Macro(depth) => nested_in_macro(attribute, *depth),
    }
}

/// A strategy over the protected lint names.
fn protected_lint() -> impl Strategy<Value = String> {
    prop::sample::select(PROTECTED_LINTS.to_vec()).prop_map(str::to_owned)
}

/// A strategy over the wrappings, bounded so each case stays small.
fn wrapping() -> impl Strategy<Value = Wrapping> {
    prop_oneof![
        Just(Wrapping::Bare),
        (1u8..=6).prop_map(Wrapping::CfgAttr),
        (1u8..=5).prop_map(Wrapping::Macro),
    ]
}

proptest! {
    /// Scenario: a protected lint is allowed through a generated wrapping.
    ///
    /// Invariant: the scan reports it, whatever the nesting depth, whichever
    /// protected lint it names, and whether or not the attribute keyword is
    /// written as a raw identifier. The cases above pin the spellings that
    /// were measured; this pins the shape they are instances of, and reaches
    /// depths no case spells out.
    ///
    /// Mutation proof (2026-09-14), run through the build and reverted:
    /// capping [`suppressed_by_cfg_attr`] at two levels of recursion fails
    /// this property on
    /// `#![cfg_attr(all(), cfg_attr(all(), cfg_attr(all(), allow(clippy::disallowed_methods))))]`,
    /// a depth the case list does not reach.
    #[test]
    fn every_wrapped_allow_of_a_protected_lint_is_reported(
        lint in protected_lint(),
        shape in wrapping(),
        raw in prop::bool::ANY,
    ) {
        let inner = !matches!(shape, Wrapping::Macro(_));
        let source = wrapped_source(&allow_attribute(&lint, inner, raw), &shape);
        let found = suppressed_lints(&source).map_err(TestCaseError::fail)?;
        prop_assert!(!found.is_empty(), "the scan must report {source:?}");
    }
}
