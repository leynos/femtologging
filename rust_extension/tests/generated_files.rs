//! Build-boundary tests for publishing guide examples into the test tree.
//!
//! Cargo runs `build.rs` without a test harness, so the write, replace, and
//! prune behaviour it relies on is exercised here instead. Each case drives
//! `test_utils/generated_files.rs` against a temporary crate root through the
//! same capability handle the build script uses.

#[path = "test_utils/doc_examples.rs"]
mod doc_examples;
#[path = "test_utils/generated_files.rs"]
mod generated_files;

use camino::Utf8Path;
use cap_std::ambient_authority;
use cap_std::fs_utf8::Dir;
use doc_examples::{Example, GENERATED_DIR};
use generated_files::{prune, publish};
use tempfile::TempDir;

/// An example that writes `fn demo() {}` under the given identifier.
fn example(identifier: &str) -> Example {
    Example {
        identifier: identifier.to_owned(),
        source: "fn demo() {}\n".to_owned(),
    }
}

/// Return the names of the entries in the generated directory.
fn generated_entries(crate_dir: &Dir) -> Vec<String> {
    let mut names: Vec<String> = crate_dir
        .read_dir(GENERATED_DIR)
        .expect("the generated directory should be readable")
        .map(|entry| {
            entry
                .expect("the directory entry should be readable")
                .file_name()
                .expect("the entry should have a name")
        })
        .collect();
    names.sort();
    names
}

/// Read a generated module.
fn read_module(crate_dir: &Dir, identifier: &str) -> String {
    let name = format!("{GENERATED_DIR}/{identifier}.rs");
    crate_dir
        .read_to_string(&name)
        .expect("the generated module should be readable")
}

/// Open a temporary crate root that already holds the generated directory.
fn prepared_crate() -> (TempDir, Dir) {
    let temp = tempfile::tempdir().expect("a temporary directory should exist");
    let path = Utf8Path::from_path(temp.path()).expect("temporary paths are UTF-8");
    let crate_dir =
        Dir::open_ambient_dir(path, ambient_authority()).expect("the crate root should open");
    crate_dir
        .create_dir_all(GENERATED_DIR)
        .expect("the generated directory should be created");
    (temp, crate_dir)
}

/// Publishing writes each example to the module its identifier names.
#[test]
fn publishing_writes_each_example_to_its_module() {
    let (_temp, crate_dir) = prepared_crate();

    publish(&crate_dir, &[example("first"), example("second")])
        .expect("publication should succeed");

    assert_eq!(generated_entries(&crate_dir), ["first.rs", "second.rs"]);
    assert_eq!(read_module(&crate_dir, "first"), "fn demo() {}\n");
    assert_eq!(read_module(&crate_dir, "second"), "fn demo() {}\n");
}

/// Re-publishing replaces the module and leaves no in-progress file behind.
#[test]
fn publishing_replaces_a_module_without_leaving_a_temporary_file() {
    let (_temp, crate_dir) = prepared_crate();
    crate_dir
        .write("tests/generated/example.rs", "old source\n")
        .expect("the stale module should be writable");

    publish(&crate_dir, &[example("example")]).expect("publication should succeed");

    assert_eq!(read_module(&crate_dir, "example"), "fn demo() {}\n");
    assert_eq!(generated_entries(&crate_dir), ["example.rs"]);
}

/// A module whose identifier disappeared is removed.
#[test]
fn pruning_removes_a_module_that_is_no_longer_published() {
    let (_temp, crate_dir) = prepared_crate();
    crate_dir
        .write("tests/generated/removed.rs", "old source\n")
        .expect("the stale module should be writable");
    crate_dir
        .write("tests/generated/kept.rs", "fn demo() {}\n")
        .expect("the current module should be writable");

    prune(&crate_dir, &[example("kept")]).expect("pruning should succeed");

    assert_eq!(generated_entries(&crate_dir), ["kept.rs"]);
}

/// Files the loader never writes are not generated modules and survive pruning.
#[test]
fn pruning_keeps_files_that_are_not_generated_modules() {
    let (_temp, crate_dir) = prepared_crate();
    crate_dir
        .write("tests/generated/notes.txt", "hand-written notes\n")
        .expect("the unrelated file should be writable");

    prune(&crate_dir, &[]).expect("pruning should succeed");

    assert_eq!(generated_entries(&crate_dir), ["notes.txt"]);
}

/// A guide that stops publishing examples removes every generated module.
#[test]
fn pruning_removes_every_module_when_the_guide_publishes_nothing() {
    let (_temp, crate_dir) = prepared_crate();
    crate_dir
        .write("tests/generated/first.rs", "fn demo() {}\n")
        .expect("the first module should be writable");
    crate_dir
        .write("tests/generated/second.rs", "fn demo() {}\n")
        .expect("the second module should be writable");

    prune(&crate_dir, &[]).expect("pruning should succeed");

    assert!(generated_entries(&crate_dir).is_empty());
}
