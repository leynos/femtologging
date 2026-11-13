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
use doc_examples::{Example, GENERATED_DIR, extract_examples};
use generated_files::{prune, publish};
use tempfile::TempDir;

/// Extract `fn demo() {}` examples under the given identifiers, as a build does.
///
/// Going through the loader rather than constructing [`Example`] values keeps
/// the cases tied to what `build.rs` actually publishes, and it means the whole
/// loader is reachable from this target rather than only its data types.
fn examples(identifiers: &[&str]) -> Vec<Example> {
    let guide: String = identifiers
        .iter()
        .map(|identifier| {
            format!("<!-- tested-example: {identifier} -->\n```rust\nfn demo() {{}}\n```\n")
        })
        .collect();
    extract_examples(&guide).expect("the generated guide should be valid")
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

/// Write `source` to `name` in the generated directory, as a build would.
fn write_generated(crate_dir: &Dir, name: &str, source: &str) {
    let path = format!("{GENERATED_DIR}/{name}");
    crate_dir
        .write(&path, source)
        .expect("the generated file should be writable");
}

/// Read a generated module.
fn read_generated(crate_dir: &Dir, name: &str) -> String {
    let path = format!("{GENERATED_DIR}/{name}");
    crate_dir
        .read_to_string(&path)
        .expect("the generated file should be readable")
}

/// Assert the generated directory holds exactly `expected`, in sorted order.
fn assert_generated(crate_dir: &Dir, expected: &[&str]) {
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
    assert_eq!(names, expected);
}

/// Publishing writes each example to the module its identifier names.
#[test]
fn publishing_writes_each_example_to_its_module() {
    let (_temp, crate_dir) = prepared_crate();

    publish(&crate_dir, &examples(&["first", "second"])).expect("publication should succeed");

    assert_generated(&crate_dir, &["first.rs", "second.rs"]);
    assert_eq!(read_generated(&crate_dir, "first.rs"), "fn demo() {}\n");
    assert_eq!(read_generated(&crate_dir, "second.rs"), "fn demo() {}\n");
}

/// Re-publishing replaces the module and leaves no in-progress file behind.
#[test]
fn publishing_replaces_a_module_without_leaving_a_temporary_file() {
    let (_temp, crate_dir) = prepared_crate();
    write_generated(&crate_dir, "example.rs", "old source\n");

    publish(&crate_dir, &examples(&["example"])).expect("publication should succeed");

    assert_eq!(read_generated(&crate_dir, "example.rs"), "fn demo() {}\n");
    assert_generated(&crate_dir, &["example.rs"]);
}

/// Pruning removes only modules the guide no longer publishes.
#[test]
fn pruning_removes_modules_no_longer_published() {
    let (_temp, crate_dir) = prepared_crate();
    write_generated(&crate_dir, "kept.rs", "fn demo() {}\n");
    write_generated(&crate_dir, "removed.rs", "old source\n");

    prune(&crate_dir, &examples(&["kept"])).expect("pruning should succeed");

    assert_generated(&crate_dir, &["kept.rs"]);
}

/// A guide that stops publishing examples removes the last module too.
#[test]
fn pruning_removes_every_module_when_the_guide_publishes_nothing() {
    let (_temp, crate_dir) = prepared_crate();
    write_generated(&crate_dir, "first.rs", "fn demo() {}\n");

    prune(&crate_dir, &[]).expect("pruning should succeed");

    assert_generated(&crate_dir, &[]);
}

/// Files the loader never writes are not generated modules and survive pruning.
#[test]
fn pruning_keeps_files_that_are_not_generated_modules() {
    let (_temp, crate_dir) = prepared_crate();
    write_generated(&crate_dir, "notes.txt", "hand-written notes\n");

    prune(&crate_dir, &[]).expect("pruning should succeed");

    assert_generated(&crate_dir, &["notes.txt"]);
}
