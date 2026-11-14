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
use generated_files::{prune, publish, temporary_name};
use rstest::{fixture, rstest};
use tempfile::TempDir;

/// A temporary crate root that already holds the generated directory.
struct Crate {
    /// Keeps the temporary directory alive for the duration of the test.
    _temp: TempDir,
    /// Capability handle for the crate root.
    dir: Dir,
}

/// The file name of a module inside the generated directory.
#[derive(Clone, Copy, Debug)]
struct GeneratedName(&'static str);

impl GeneratedName {
    /// Return the path of this module relative to the crate root.
    fn path(self) -> String {
        format!("{GENERATED_DIR}/{}", self.0)
    }
}

/// What a fixture file in the generated directory contains.
#[derive(Clone, Copy)]
enum Content {
    /// Source that a build would publish.
    Published,
    /// Source left behind by an earlier build.
    Stale,
    /// A file the loader never writes.
    Notes,
}

impl Content {
    /// Return the text this fixture writes.
    fn text(self) -> &'static str {
        match self {
            Self::Published => "fn demo() {}\n",
            Self::Stale => "old source\n",
            Self::Notes => "hand-written notes\n",
        }
    }
}

impl Crate {
    /// Write `content` to `name`, as a build or a stale tree would.
    fn write(&self, name: GeneratedName, content: Content) {
        self.dir
            .write(name.path(), content.text())
            .expect("the generated file should be writable");
    }

    /// Read the generated file at `name`.
    fn read(&self, name: GeneratedName) -> String {
        self.dir
            .read_to_string(name.path())
            .expect("the generated file should be readable")
    }

    /// Assert the generated directory holds exactly `expected`, sorted.
    fn assert_holds(&self, expected: &[GeneratedName]) {
        let mut names: Vec<String> = self
            .dir
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
        let expected: Vec<&str> = expected.iter().map(|name| name.0).collect();
        assert_eq!(names, expected);
    }
}

/// Create a temporary crate root with the generated directory in place.
#[fixture]
fn crate_root() -> Crate {
    let temp = tempfile::tempdir().expect("a temporary directory should exist");
    let path = Utf8Path::from_path(temp.path()).expect("temporary paths are UTF-8");
    let dir = Dir::open_ambient_dir(path, ambient_authority()).expect("the crate root should open");
    dir.create_dir_all(GENERATED_DIR)
        .expect("the generated directory should be created");
    Crate { _temp: temp, dir }
}

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

/// Publishing writes each example to the module its identifier names.
#[rstest]
fn publishing_writes_each_example_to_its_module(crate_root: Crate) {
    publish(&crate_root.dir, &examples(&["first", "second"])).expect("publication should succeed");

    crate_root.assert_holds(&[GeneratedName("first.rs"), GeneratedName("second.rs")]);
    assert_eq!(crate_root.read(GeneratedName("first.rs")), "fn demo() {}\n");
    assert_eq!(
        crate_root.read(GeneratedName("second.rs")),
        "fn demo() {}\n"
    );
}

/// Re-publishing replaces the module and leaves no in-progress file behind.
#[rstest]
fn publishing_replaces_a_module_without_leaving_a_temporary_file(crate_root: Crate) {
    crate_root.write(GeneratedName("example.rs"), Content::Stale);

    publish(&crate_root.dir, &examples(&["example"])).expect("publication should succeed");

    assert_eq!(
        crate_root.read(GeneratedName("example.rs")),
        "fn demo() {}\n"
    );
    crate_root.assert_holds(&[GeneratedName("example.rs")]);
}

/// Two publications of one example never share a temporary path.
#[rstest]
fn each_publication_takes_its_own_temporary_name() {
    let example = &examples(&["example"])[0];

    let first = temporary_name(example);
    let second = temporary_name(example);

    assert_ne!(first, second, "concurrent builds must not collide");
    for name in [&first, &second] {
        assert!(name.starts_with("tests/generated/example.rs."), "{name}");
        assert!(name.ends_with(".tmp"), "{name}");
    }
}

/// Pruning keeps exactly the modules the guide still publishes.
///
/// A module whose identifier disappeared is removed, and a guide that stops
/// publishing examples leaves the generated directory empty.
#[rstest]
#[case(
    &[
        (GeneratedName("kept.rs"), Content::Published),
        (GeneratedName("removed.rs"), Content::Stale),
    ],
    &["kept"],
    &[GeneratedName("kept.rs")]
)]
#[case(&[(GeneratedName("only.rs"), Content::Published)], &[], &[])]
fn pruning_keeps_only_published_modules(
    crate_root: Crate,
    #[case] initial: &[(GeneratedName, Content)],
    #[case] retained: &[&str],
    #[case] expected: &[GeneratedName],
) {
    for (name, content) in initial {
        crate_root.write(*name, *content);
    }

    prune(&crate_root.dir, &examples(retained)).expect("pruning should succeed");

    crate_root.assert_holds(expected);
}

/// Files the loader never writes are not generated modules and survive pruning.
#[rstest]
fn pruning_keeps_files_that_are_not_generated_modules(crate_root: Crate) {
    crate_root.write(GeneratedName("notes.txt"), Content::Notes);

    prune(&crate_root.dir, &[]).expect("pruning should succeed");

    crate_root.assert_holds(&[GeneratedName("notes.txt")]);
}
