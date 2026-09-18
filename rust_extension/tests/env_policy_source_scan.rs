//! Assertions for the source scan that closes the attribute routes around the
//! environment-access policy.
//!
//! Three files, each answering one kind of question and each inside the
//! 400-line module limit:
//!
//! - this one holds the crate-wide invariant, the traversal's own contracts
//!   and the mutation records behind them;
//! - [`policy_routes`] catalogues the attribute routes the scan closes and the
//!   shapes it must leave alone;
//! - [`policy_walk`] holds the traversal's own contracts, which ask what the
//!   walk reads rather than what the scan refuses;
//! - [`source_scan`] holds the machinery both drive, and the Clippy
//!   measurements behind each route.
//!
//! The catalogue and the traversal contracts moved out when this file reached
//! 641 lines. Both grow with every route closed, where the invariant here does
//! not, so keeping them together meant the limit would be breached again on
//! the next route. What stays is the one crate-wide assertion and the record
//! of what each rule was proved by, which is the part that has to be read
//! beside the rule it justifies.

use camino::{Utf8Path, Utf8PathBuf};

// The paths are relative to this file's own directory, `tests/`.
#[path = "test_utils/policy_routes.rs"]
mod policy_routes;
#[path = "test_utils/policy_walk.rs"]
mod policy_walk;
#[path = "test_utils/source_scan.rs"]
mod source_scan;

use source_scan::{SOURCE_ROOTS, crate_sources, suppressed_lints};

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
///
/// Six further routes were closed on 2026-09-16, each raised in review against
/// a measured evasion. Each is proved in both directions, every mutation
/// applied alone, run through the build and reverted, because a rule that
/// reaches nothing and a rule that reaches everything both pass a one-sided
/// proof:
///
/// - an unreadable `allow` argument list. Returning an empty lint list on a
///   parse failure, which is what the scan did, fails `forwarded_lint_name`;
///   returning every protected lint for a list that *did* parse fails this
///   test on the crate's own sources and the benign fixture with it;
/// - an item-scoped `expect` without a reason. Exempting every item-scoped
///   `expect`, which is what the scan did, fails
///   `item_scoped_expect_without_a_reason` and
///   `item_scoped_expect_with_a_blank_reason`; judging every `expect`
///   whatever its scope and reason fails this test and the benign fixture, on
///   the two sanctioned reasoned forms; accepting a blank reason fails
///   `item_scoped_expect_with_a_blank_reason` alone, which is why the reason
///   is trimmed before it is weighed;
/// - an identifier merely named `env`. Treating any such identifier as an
///   environment access, which is what the scan did, fails the benign fixture
///   on `takes_an_env`, a doc-forwarding arm over a parameter named `env`;
///   treating an arm as never writing an access fails
///   `forwarded_attribute_path`, whose transcriber writes `std::env::var`
///   itself and forwards a `meta` that carries no code;
/// - a code fragment declared inside a repetition. Reading the matcher only at
///   its top level, which is what the scan did, fails
///   `forwarded_over_a_repeated_item`; counting an `ident` among the
///   code-carrying fragments fails this test and the benign fixture, on the
///   `option_setter` idiom this crate's own builders use twice;
/// - an `include!` target that resolves somewhere the walk never reaches.
///   Accepting any `.rs` extension, which is what the scan did, fails
///   `include_under_a_dot_directory`, `include_under_build_output`,
///   `include_above_the_crate` and `include_of_an_absolute_path`; judging the
///   target's own file name as well as the directories it passes through
///   fails the benign fixture on `include!(".hidden.rs")`, a dot-prefixed file
///   the walk does collect;
/// - an ordinary macro invocation's arguments. Descending into every group,
///   which is what the scan did, fails the benign fixture on `quoted!`, whose
///   `stringify!` argument becomes a string, and `discards!`, whose argument
///   is thrown away; descending into none fails `macro_within_macro`,
///   `forwarded_over_a_repeated_item` and the generated-wrapping property,
///   whose attributes sit inside a nested transcriber.
///
/// Two further routes were closed on 2026-09-17, both raised in review, both
/// proved in both directions with each mutation applied alone, run through the
/// build and reverted:
///
/// - a `cfg_attr` whose argument list will not parse. Returning an empty lint
///   list, which is what the scan did, fails `forwarded_cfg_attr_condition`:
///   a `macro_rules!` arm writing
///   `#[cfg_attr($cond, allow(clippy::disallowed_methods))]` writes its own
///   path out as `cfg_attr`, so the forwarded-path rule does not refuse it,
///   and the expansion compiles with the suppression live. It now fails closed
///   the way an unreadable `allow` list already did;
/// - a module loaded through `#[path]`. Not reading the attribute at all,
///   which is what the scan did, fails the six `module_path_*` cases:
///   `#[path = "../../bypass.rs"] mod bypass;` compiles a file outside
///   `CARGO_MANIFEST_DIR`, which `crate_sources` never walks, so a crate-level
///   `allow` written there is unread. Calling every target unreadable fails
///   this test and the benign fixture, on the `#[path]` declarations this file
///   and `test_utils/source_scan.rs` use themselves: the narrowness half, and
///   the reason the rule is the walk's own rather than a second one written
///   for attributes.
#[test]
fn no_source_file_suppresses_a_policy_lint() -> Result<(), String> {
    let sources = crate_sources()?;
    every_root_is_represented(&sources)?;
    let offences = policy_offences(&sources)?;
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

/// Check that each named root is represented among the sources walked.
///
/// The tripwire half of [`no_source_file_suppresses_a_policy_lint`], stated
/// apart from the offence search so each reads as the one question it asks.
fn every_root_is_represented(sources: &[(Utf8PathBuf, String)]) -> Result<(), String> {
    for required in SOURCE_ROOTS {
        if !sources.iter().any(|(path, _)| is_under(path, required)) {
            return Err(format!(
                "the walk should reach {required}, saw {} sources",
                sources.len()
            ));
        }
    }
    Ok(())
}

/// Return every protected-lint suppression across the walked sources.
fn policy_offences(sources: &[(Utf8PathBuf, String)]) -> Result<Vec<String>, String> {
    let mut offences = Vec::new();
    for (path, contents) in sources {
        let findings =
            suppressed_lints(path, contents).map_err(|error| format!("{path}: {error}"))?;
        offences.extend(
            findings
                .into_iter()
                .map(|finding| format!("{path} {finding}")),
        );
    }
    Ok(offences)
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
