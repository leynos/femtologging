//! Contracts for the traversal itself: which directories it descends into and
//! which files it collects.
//!
//! Separated from `tests/env_policy_source_scan.rs`, which holds the
//! crate-wide invariant and the record of what each rule was proved by, and
//! from [`super::policy_routes`], which catalogues the attribute routes. This
//! file asks only what the walk reads, and it answers with a real directory
//! tree rather than an inline fixture, because the traversal is what is under
//! test.
//!
//! The walk's rule for which directories are reachable is the same rule the
//! inclusion and `#[path]` findings are judged by, in
//! [`super::source_scan::is_walkable`]. That is deliberate: a target the walk
//! would not collect is a file the scan never reads, and two rules that could
//! drift would close one route and leave its twin open.

use camino::Utf8Path;
use cap_std::{ambient_authority, fs_utf8::Dir};
use rstest::rstest;
use tempfile::TempDir;

use super::source_scan::{crate_dir, is_walkable, rust_sources};

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

/// Build a crate-shaped tree and return the paths the walk collects from it.
///
/// A real directory rather than an inline fixture, because what is under test
/// is the traversal itself: which directories it enters and which files it
/// reads. The crate's own sources cannot discriminate the rule, since it has
/// no Cargo target outside the three named roots and builds into a target
/// directory outside itself, so the shapes have to be built.
///
/// Written through a `cap_std` handle for the same reason the walk reads
/// through one: the repository's Dylint suite disallows `std::fs`.
fn walked_paths() -> Result<Vec<String>, String> {
    let held = TempDir::new().map_err(|error| format!("temporary directory: {error}"))?;
    let root = Utf8Path::from_path(held.path()).ok_or("temporary path is not UTF-8")?;
    let directory = Dir::open_ambient_dir(root, ambient_authority())
        .map_err(|error| format!("open temporary directory: {error}"))?;
    for (parent, name) in [
        ("src", "lib.rs"),
        ("examples", "probe.rs"),
        ("target", "generated.rs"),
        (".generated", "bypass.rs"),
    ] {
        directory
            .create_dir(parent)
            .map_err(|error| format!("create {parent}: {error}"))?;
        directory
            .write(format!("{parent}/{name}"), "fn f() {}\n")
            .map_err(|error| format!("write {parent}/{name}: {error}"))?;
    }
    let mut paths: Vec<String> = rust_sources(root, "")?
        .into_iter()
        .map(|(path, _)| path.to_string())
        .collect();
    paths.sort();
    Ok(paths)
}

/// Scenario: the walk is pointed at a crate holding a Cargo target outside the
/// named roots, build output, and a dot-prefixed directory.
///
/// Invariant: it reads the sources and the added target, and neither the build
/// output nor the tool state. `lint-env-policy` passes `--all-targets`, which
/// compiles an `examples` target when one exists, so a walk restricted to the
/// named source roots would leave every example ungoverned; and a walk that
/// entered `target` would scan generated code the policy does not govern.
///
/// This is the only place either direction can fail, because the crate holds
/// no Rust file outside `src`, `tests` and `benches`: every other assertion in
/// this file reads the crate's own sources and cannot tell a walk of the whole
/// crate from a walk of the three roots.
///
/// Mutation proof (2026-09-16), each applied alone, run through the build and
/// reverted:
///
/// - letting `is_walkable` accept `target` fails this test on
///   `target/generated.rs`, the `build_output` case above, and
///   `include_under_build_output`, which judges an inclusion target by the
///   same function;
/// - removing the traversal's descent into a directory outside the named roots
///   is not expressible against the crate as it stands. Collecting the named
///   source roots one at a time instead of walking the crate directory, the
///   shape the scan shipped with, was run and passed everything: with no
///   Cargo target outside those three, the two walks read exactly the same
///   files. The difference is latent until an `examples` target is added,
///   which is what this test builds and reads.
#[test]
fn the_walk_reads_an_added_target_and_neither_build_output_nor_tool_state() -> Result<(), String> {
    let found = walked_paths()?;
    let expected = ["examples/probe.rs", "src/lib.rs"];
    if found == expected {
        return Ok(());
    }
    Err(format!(
        "expected the walk to read {expected:?}, it read {found:?}"
    ))
}

/// Scenario: the walk is pointed at a governed root that is not there.
///
/// Invariant: it returns an error naming the root it could not open, rather
/// than an empty source list. An empty list is indistinguishable from a root
/// that exists and holds no Rust, and the crate-wide invariant passes
/// vacuously over it: every file in a root nobody could open suppresses
/// nothing. That is the failure mode this contract exists to refuse, and it is
/// reachable by a rename, so `every_root_is_represented` is the tripwire and
/// this is the boundary underneath it.
///
/// The relative label is asserted rather than the operating system's text,
/// which differs by platform and by locale. The label is what tells a reader
/// which of the three roots went missing.
///
/// Mutation proof (2026-09-18), applied alone through the build and reverted:
/// returning `Ok(Vec::new())` where `rust_sources` maps the open error fails
/// this test with `the walk must refuse a root it cannot open, it returned
/// Ok([])`. That was an earlier draft's behaviour, and nothing else in the
/// suite noticed it.
#[test]
fn a_governed_root_that_cannot_be_opened_is_an_error_not_an_empty_walk() {
    let absent = crate_dir().join("no_such_governed_root");
    let outcome = rust_sources(&absent, "no_such_governed_root");
    let Err(message) = outcome else {
        panic!("the walk must refuse a root it cannot open, it returned {outcome:?}");
    };
    assert!(
        message.starts_with("open no_such_governed_root: "),
        "the error must name the root it could not open, saw {message:?}"
    );
}

/// Scenario: the walk reaches a `.rs` file whose bytes it cannot read as text.
///
/// Invariant: it returns an error naming the file, rather than skipping it and
/// returning the sources around it. A skipped file is the worst outcome
/// available here: the walk reports success, the crate-wide invariant finds no
/// offence in a file nobody read, and the one source that evaded the scan is
/// the one a suppression would be hidden in. This is the third of the
/// scanner's boundaries, after the missing root above and the unparseable
/// source in [`super::policy_routes`].
///
/// Invalid UTF-8 rather than a permission bit, because the outcome has to be
/// the same on every machine the suite runs on. A mode of `0o000` is readable
/// by a process running as root, which the container lanes do, so the test
/// would pass locally and report nothing in CI.
///
/// Mutation proof (2026-09-18), applied alone through the build and reverted:
/// returning `Ok(Found::Ignored)` where `classify` maps the read error fails
/// this test with `the walk must refuse a source it cannot read, it returned
/// Ok([])`. Skipping is the plausible alternative and the dangerous one, which
/// is what makes the mutation worth stating.
#[test]
fn a_source_that_cannot_be_read_as_text_is_an_error_not_a_skipped_file() -> Result<(), String> {
    let held = TempDir::new().map_err(|error| format!("temporary directory: {error}"))?;
    let root = Utf8Path::from_path(held.path()).ok_or("temporary path is not UTF-8")?;
    let directory = Dir::open_ambient_dir(root, ambient_authority())
        .map_err(|error| format!("open temporary directory: {error}"))?;
    directory
        .create_dir("src")
        .map_err(|error| format!("create src: {error}"))?;
    // A lone continuation byte: never a valid UTF-8 sequence, on any platform.
    directory
        .write("src/unreadable.rs", [0x80_u8])
        .map_err(|error| format!("write src/unreadable.rs: {error}"))?;

    let outcome = rust_sources(root, "");
    let Err(message) = outcome else {
        return Err(format!(
            "the walk must refuse a source it cannot read, it returned {outcome:?}"
        ));
    };
    if message.starts_with("read src/unreadable.rs: ") {
        return Ok(());
    }
    Err(format!(
        "the error must name the file it could not read, saw {message:?}"
    ))
}
