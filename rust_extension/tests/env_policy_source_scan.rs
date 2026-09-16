//! Assertions for the source scan that closes the attribute routes around the
//! environment-access policy.
//!
//! The machinery the assertions drive, and the measurements behind each route
//! it closes, live in [`source_scan`]; this file holds the invariants, the
//! fixtures and the mutation records. The split keeps every file inside the
//! 400-line module limit and separates what the scan refuses from the evidence
//! that it refuses it.

use camino::Utf8Path;
use proptest::prelude::*;
use rstest::rstest;

// The path is relative to this file's own directory, `tests/`.
#[path = "test_utils/source_scan.rs"]
mod source_scan;

use source_scan::{
    PROTECTED_LINTS, SOURCE_ROOTS, crate_dir, crate_sources, is_walkable, rust_sources,
    suppressed_lints,
};

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
/// contract switched off. Re-proved on 2026-09-15 after the machinery moved
/// into [`source_scan`], each mutation applied alone and reverted:
///
/// - the forwarded-path rule never firing fails the three cases that
///   exercise it;
/// - refusing every forwarded path, rather than only a reachable or inner
///   one, fails the benign fixture and this test, on the
///   `$(#[$meta:meta])*` doc-forwarding idiom in
///   `src/handlers/http_builder.rs` and `src/handlers/socket_builder.rs`;
/// - refusing any metavariable anywhere in the attribute, rather than in its
///   path, fails the benign fixture on `#[doc = $text]` and this test on
///   `#[doc = $doc]` in `src/handlers/builder_macros.rs` and
///   `src/logger/convenience_methods.rs`;
/// - accepting `.txt` alongside `.rs` as an `include!` target fails the case
///   that exercises it;
/// - judging `include_str!` as source inclusion fails this test on the real
///   `include!` of a Python fixture and of the two manifests, and the benign
///   fixture with it;
/// - walking every macro rather than only a `macro_rules!` transcriber fails
///   the benign fixture on `discards!(#[allow(clippy::all)])`, whose argument
///   the macro throws away. Nothing in the crate's own sources discriminates
///   that rule, so the fixture carries it: a narrowing nothing can fail is a
///   narrowing nobody has tested.
///
/// The `include!` target is judged by its decoded value and by the extension
/// the walk selects on, which was proved on 2026-09-15 in both directions:
///
/// - reading the literal as written, by trimming quotes off
///   `Literal::to_string`, fails the benign fixture on `r"fixtures/raw.rs"`
///   and on `"fixtures/escaped\x2Ers"`, which name `.rs` files the walk does
///   collect;
/// - accepting a `.rs` literal found anywhere in the argument, rather than the
///   whole argument parsed as one literal, fails
///   `include_of_a_computed_rust_path`, whose target is assembled at compile
///   time from a directory the scan cannot know;
/// - returning `None` for every target fails both `include!` cases here;
/// - comparing the rendered path with `ends_with(".rs")` fails
///   `include_of_a_bare_extension`, since `.rs` ends with `.rs` while naming a
///   file the walk never collects.
#[test]
fn no_source_file_suppresses_a_policy_lint() -> Result<(), String> {
    let sources = crate_sources()?;
    for required in SOURCE_ROOTS {
        if !sources.iter().any(|(path, _)| is_under(path, required)) {
            return Err(format!(
                "the walk should reach {required}, saw {} sources",
                sources.len()
            ));
        }
    }
    let mut offences = Vec::new();
    for (path, contents) in &sources {
        for finding in suppressed_lints(contents).map_err(|error| format!("{path}: {error}"))? {
            offences.push(format!("{path} {finding}"));
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

/// Return whether `path` sits under the directory named `root`.
///
/// The first component is compared rather than a string prefix, because the
/// walk joins with the platform separator: on Windows every path reads
/// `src\config\mod.rs`, and a `starts_with("src/")` test would match nothing
/// there while matching here.
fn is_under(path: &Utf8Path, root: &str) -> bool {
    path.components()
        .next()
        .is_some_and(|component| component.as_str() == root)
}

/// Scenario: the walk is asked which directory names it descends into.
///
/// Invariant: everything except build output and tool state. The walk reads
/// the whole crate, so what it refuses to enter is the whole of what it
/// cannot govern, and that is worth stating directly: nothing else in this
/// file can fail the rule, because this worktree builds into a target
/// directory outside the crate and has no dot-prefixed directory under it.
///
/// A guard no test can fail is a comment. Both directions are named here: a
/// rule that let `target` through would read every generated source under it,
/// and a rule that refused an ordinary directory would stop governing real
/// code.
#[rstest]
#[case::sources("src", true)]
#[case::an_added_target("examples", true)]
#[case::build_output("target", false)]
#[case::version_control(".git", false)]
#[case::tool_state(".cargo", false)]
fn the_walk_descends_into_everything_but_build_output_and_tool_state(
    #[case] name: &str,
    #[case] walkable: bool,
) -> Result<(), String> {
    if is_walkable(name) == walkable {
        return Ok(());
    }
    Err(format!(
        "expected is_walkable({name:?}) to be {walkable}, got {}",
        is_walkable(name)
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
/// Mutation proof, each applied alone, run through the build and reverted:
///
/// - (2026-09-16) making the walk refuse `benches` fails
///   [`no_source_file_suppresses_a_policy_lint`] with `the walk should reach
///   benches, saw 181 sources`. The roots are a tripwire on the whole-crate
///   walk now rather than the list of places it reads, so this is where a
///   root going unread is caught;
/// - (2026-09-14) stopping the walk from descending into subdirectories fails
///   this test's `src` case with `src did not yield src/config/mod.rs`, and
///   nothing else in the file notices. That is why the named source is nested
///   rather than `src/lib.rs`.
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
/// `#[allow(dead_code, ...)]` names no protected lint; `discards!` is handed
/// an attribute by an invocation and throws it away, which is why only a
/// `macro_rules!` transcriber is walked; `include_str!` and `include_bytes!`
/// embed bytes rather than compiling source; and the three `include!` calls
/// name `.rs` paths the scan reads in their own right, written plainly, as a
/// raw string and with the dot escaped, because the rule judges what the
/// literal means rather than how it was typed.
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
macro_rules! discards { ($ignored:tt) => {}; }
discards!(#[allow(clippy::all)]);
const EMBEDDED: &str = include_str!("fixtures/env_policy_probe.rs.txt");
const BYTES: &[u8] = include_bytes!("fixtures/table.dat");
include!("fixtures/generated.rs");
include!(r"fixtures/raw.rs");
include!("fixtures/escaped\x2Ers");
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
// A bare extension is a name the walk never collects, so the file it names is
// never scanned however the inclusion reads.
#[case::include_of_a_bare_extension("include!(\".rs\");")]
// A computed target that happens to hold a `.rs` literal is still a target the
// scan cannot resolve; the whole argument has to be one literal.
#[case::include_of_a_computed_rust_path("include!(concat!(env!(\"OUT_DIR\"), \"/probe.rs\"));")]
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
