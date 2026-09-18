//! Assertions for the attribute routes the scan closes, and for the shapes it
//! must leave alone.
//!
//! Separated from `tests/env_policy_source_scan.rs`, which keeps the
//! crate-wide invariant and the traversal's own contracts, because this file
//! is a catalogue and that one is a set of invariants. Each route here was
//! measured against Clippy before it was closed; the measurements are recorded
//! in [`super::source_scan`], and the cases below pin the spellings.
//!
//! The narrowness cases matter as much as the positive ones. A scan that
//! reported a lint name inside a doc comment, or this crate's own
//! `#[path = "..."]` module declarations, would be switched off within a week,
//! so both halves are asserted and both are mutation-proved.

use camino::Utf8Path;
use proptest::prelude::*;
use rstest::rstest;

use super::source_scan::{PROTECTED_LINTS, suppressed_lints};

/// The path an inline fixture pretends to be.
///
/// Every fixture below is judged as an ordinary source under `src`, so a rule
/// that depends on where the file sits is exercised somewhere real. An
/// `include!` in a fixture therefore resolves against `src`.
const FIXTURE_PATH: &str = "src/fixture.rs";

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
/// `macro_rules!` transcriber is walked; `quoted!` writes an attribute inside
/// `stringify!`, whose argument becomes a string rather than an attribute, so
/// the walk does not descend into an ordinary invocation's arguments;
/// `takes_an_env` forwards the doc idiom over a parameter *named* `env`, which
/// is not an environment access and must not make the arm reachable;
/// `include_str!` and `include_bytes!`
/// embed bytes rather than compiling source; and the three `include!` calls
/// name `.rs` paths the scan reads in their own right, written plainly, as a
/// raw string and with the dot escaped, because the rule judges what the
/// literal means rather than how it was typed. The last names a dot-prefixed
/// *file*, which the walk collects like any other because it selects a file by
/// its extension alone: only the directories a target passes through are
/// judged.
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
macro_rules! quoted { () => { stringify!(#[allow(clippy::all)]) }; }
macro_rules! takes_an_env { ($(#[$meta:meta])* $name:ident) => {
    $(#[$meta])* pub fn $name(env: &str) -> usize { env.len() }
}; }
const EMBEDDED: &str = include_str!("fixtures/env_policy_probe.rs.txt");
const BYTES: &[u8] = include_bytes!("fixtures/table.dat");
include!("fixtures/generated.rs");
include!(r"fixtures/raw.rs");
include!("fixtures/escaped\x2Ers");
include!(".hidden.rs");
#[path = "fixtures/support.rs"]
mod support;
#[cfg_attr(all(), path = "fixtures/conditional.rs")]
mod conditional;
"##;
    let found = suppressed_lints(Utf8Path::new(FIXTURE_PATH), benign)?;
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
// A forwarded lint *name*: `syn` accepts the outer list and cannot read its
// contents, so an earlier draft read it as suppressing nothing at all. Invoked
// as `suppress!(clippy::disallowed_methods)` the arm expands to a real
// suppression over a real call.
#[case::forwarded_lint_name(concat!(
    "macro_rules! suppress { ($lint:meta) => {\n",
    "    #[allow($lint)]\n",
    "    pub fn ambient() { let _ = std::env::var(\"X\"); }\n",
    "}; }\nsuppress!(clippy::disallowed_methods);"
))]
// An item-scoped `expect` with no reason is a quieter `allow`. The reason is
// the whole of what makes the sanctioned form sanctioned.
#[case::item_scoped_expect_without_a_reason(
    "#[expect(clippy::disallowed_methods)]\nfn f() { let _ = std::env::var(\"X\"); }"
)]
// And one whose reason is blank, which says exactly as much.
#[case::item_scoped_expect_with_a_blank_reason(concat!(
    "#[expect(clippy::disallowed_methods, reason = \"   \")]\n",
    "fn f() { let _ = std::env::var(\"X\"); }"
))]
// A code fragment declared inside a repetition. The arm names no `env`
// itself, and a matcher read only at its top level never sees the `item`, so
// the forwarded attribute went unreported.
#[case::forwarded_over_a_repeated_item(concat!(
    "macro_rules! forward { ($(#[$attr:meta])? $($body:item)*) => {\n",
    "    $(#[$attr])? $($body)*\n",
    "}; }"
))]
// A `.rs` target the walk never reaches. `rustc` resolves it against the file
// that writes it, and the walk skips `target` and every dot-prefixed
// directory, so each of these names a real Rust file the compiler reads and
// the scan does not open.
#[case::include_under_a_dot_directory("include!(\".generated/bypass.rs\");")]
#[case::include_under_build_output("include!(\"../target/debug/bypass.rs\");")]
// And two that leave the crate the walk reads altogether.
#[case::include_above_the_crate("include!(\"../../bypass.rs\");")]
#[case::include_of_an_absolute_path("include!(\"/tmp/bypass.rs\");")]
// A module loaded through `#[path]`, which names a second source exactly as
// `include!` does. `crate_sources` walks only below `CARGO_MANIFEST_DIR`, so a
// crate-level `allow` written in the target switches the policy off for
// everything the module covers and the scan never opens it.
#[case::module_path_above_the_crate("#[path = \"../../bypass.rs\"]\nmod bypass;")]
#[case::module_path_of_an_absolute_path("#[path = \"/tmp/bypass.rs\"]\nmod bypass;")]
#[case::module_path_under_build_output("#[path = \"../target/debug/bypass.rs\"]\nmod bypass;")]
#[case::module_path_under_a_dot_directory("#[path = \".generated/bypass.rs\"]\nmod bypass;")]
// `rustc` compiles a `#[path]` target as Rust whatever its extension, so a
// fixture suffix hides the source from the walk without hiding it from Clippy.
#[case::module_path_of_a_foreign_extension("#[path = \"bypass.rs.txt\"]\nmod bypass;")]
// The same reference behind a `cfg_attr`, the wrapper the `allow` route also
// travels through.
#[case::module_path_through_cfg_attr("#[cfg_attr(all(), path = \"../../bypass.rs\")]\nmod bypass;")]
// A `macro_rules!` arm forwarding a `cfg_attr` condition. The attribute's own
// path is written out as `cfg_attr`, so the forwarded-path rule does not
// refuse it, and the argument list will not parse as a list of metas. Invoked
// as `gate!(all())` the expansion compiles and the suppression is real.
#[case::forwarded_cfg_attr_condition(concat!(
    "macro_rules! gate { ($cond:meta) => {\n",
    "    #[cfg_attr($cond, allow(clippy::disallowed_methods))]\n",
    "    pub fn ambient() { let _ = std::env::var(\"X\"); }\n",
    "}; }\ngate!(all());"
))]
fn the_scan_follows_groups_and_cfg_attr(#[case] source: &str) -> Result<(), String> {
    if suppressed_lints(Utf8Path::new(FIXTURE_PATH), source)?.is_empty() {
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
        let found = suppressed_lints(Utf8Path::new(FIXTURE_PATH), &source).map_err(TestCaseError::fail)?;
        prop_assert!(!found.is_empty(), "the scan must report {source:?}");
    }
}

/// Scenario: the scan is handed a file that is not Rust at all.
///
/// Invariant: it returns an error naming the parse, rather than reporting no
/// suppressions. The two outcomes are opposite in meaning and identical to a
/// caller that only asks whether the finding list is empty: a file the parser
/// choked on has not been cleared, it has not been read. The crate-wide
/// invariant propagates this error rather than skipping the file, so a source
/// that stops parsing stops the gate.
///
/// The message is asserted, not merely the failure, because the context is the
/// whole value of the error here. `no_source_file_suppresses_a_policy_lint`
/// reports it against the path it was reading, and an error that does not say
/// what went wrong sends a reader to the wrong question.
///
/// Mutation proof (2026-09-18), applied alone through the build and reverted:
/// returning `Ok(Vec::new())` where `suppressed_lints` maps the parse error
/// fails all three cases with `the scan must refuse "fn missing_a_body(", it
/// returned Ok([])`. That is the shape the scan shipped with before it parsed,
/// and it is the reason the boundary is asserted here rather than left to the
/// crate-wide invariant, which cannot tell an empty finding list from a file
/// nobody read.
#[rstest]
#[case::unclosed_item("fn missing_a_body(")]
#[case::stray_delimiter("}")]
#[case::not_rust_at_all("<html><body>not rust</body></html>")]
fn a_source_that_will_not_parse_is_an_error_not_an_empty_finding_list(#[case] source: &str) {
    let outcome = suppressed_lints(Utf8Path::new(FIXTURE_PATH), source);
    let Err(message) = outcome else {
        panic!("the scan must refuse {source:?}, it returned {outcome:?}");
    };
    assert!(
        message.starts_with("parse: "),
        "the error must name the parse as its cause, saw {message:?}"
    );
}
