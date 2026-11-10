//! Load the Rust examples published in the user guide into the test tree.
//!
//! `docs/documentation-style-guide.md` requires user-facing examples to be
//! exercised as they are published, rather than as a copy kept beside them.
//! A build script is the earliest point at which the published text can
//! become Rust source: it extracts every marked `tested-example` fence from
//! `docs/users-guide.md` and writes it under `tests/generated`. An integration
//! test then `include!`s the generated file, so the test compiles exactly the
//! text the guide ships and the two cannot drift apart.
//!
//! The Markdown rules live in `tests/test_utils/doc_examples.rs`, which
//! `tests/doc_example_loader.rs` exercises directly; this file only does the
//! I/O. The generated files are written inside `tests` rather than to
//! `OUT_DIR` because the environment-policy scan in
//! `tests/env_policy_source_scan.rs` refuses any `include!` target the walk
//! never reads. A file under build output could carry a lint suppression the
//! scan would never see, and that refusal is deliberately not weakened for
//! documentation. Generated files are ignored by Git; rebuild to regenerate
//! them.

#[path = "tests/test_utils/doc_examples.rs"]
mod doc_examples;

use std::collections::BTreeSet;
use std::error::Error;
use std::io;

use camino::Utf8Path;
use cap_std::ambient_authority;
use cap_std::fs_utf8::Dir;

use doc_examples::{Example, GENERATED_DIR, extract_examples};

/// Guide path, relative to the repository root one level above the crate.
const GUIDE_PATH: &str = "docs/users-guide.md";

/// Suffix of the in-progress file that is renamed into place.
const TEMP_SUFFIX: &str = ".tmp";

/// Extension of a generated module.
const GENERATED_EXTENSION: &str = ".rs";

/// Extract the published examples and write them where tests can include them.
fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rerun-if-changed=../{GUIDE_PATH}");

    let repository = Dir::open_ambient_dir(manifest_dir.join(".."), ambient_authority())?;
    let text = repository.read_to_string(GUIDE_PATH)?;
    let examples = extract_examples(&text)?;

    let crate_dir = Dir::open_ambient_dir(manifest_dir, ambient_authority())?;
    crate_dir.create_dir_all(GENERATED_DIR)?;
    publish(&crate_dir, &examples)?;
    prune(&crate_dir, &examples)?;

    Ok(())
}

/// Write each example, then remove generated modules it no longer produces.
///
/// Each module is renamed into place once it is complete, so a build that runs
/// alongside another one never exposes a half-written example.
fn publish(crate_dir: &Dir, examples: &[Example]) -> io::Result<()> {
    for example in examples {
        let temp_name = format!("{}{TEMP_SUFFIX}", example.output_name());
        crate_dir.write(&temp_name, &example.source)?;
        crate_dir.rename(&temp_name, crate_dir, example.output_name())?;
    }
    Ok(())
}

/// Remove generated modules left behind by an identifier that has gone.
fn prune(crate_dir: &Dir, examples: &[Example]) -> io::Result<()> {
    let expected: BTreeSet<String> = examples.iter().map(Example::output_name).collect();
    for entry in crate_dir.read_dir(GENERATED_DIR)? {
        let name = format!("{GENERATED_DIR}/{}", entry?.file_name()?);
        if name.ends_with(GENERATED_EXTENSION) && !expected.contains(&name) {
            crate_dir.remove_file(&name)?;
        }
    }
    Ok(())
}
