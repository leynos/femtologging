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
//! The Markdown rules live in `tests/test_utils/doc_examples.rs` and the file
//! operations in `tests/test_utils/generated_files.rs`, so the integration
//! tests can exercise both; this file only wires them together. The generated
//! files are written inside `tests` rather than to
//! `OUT_DIR` because the environment-policy scan in
//! `tests/env_policy_source_scan.rs` refuses any `include!` target the walk
//! never reads. A file under build output could carry a lint suppression the
//! scan would never see, and that refusal is deliberately not weakened for
//! documentation. Generated files are ignored by Git; rebuild to regenerate
//! them.

#[path = "tests/test_utils/doc_examples.rs"]
mod doc_examples;
#[path = "tests/test_utils/generated_files.rs"]
mod generated_files;

use std::error::Error;

use camino::Utf8Path;
use cap_std::ambient_authority;
use cap_std::fs_utf8::Dir;

use doc_examples::{GENERATED_DIR, extract_examples};
use generated_files::{prune, publish};

/// Guide path, relative to the repository root one level above the crate.
const GUIDE_PATH: &str = "docs/users-guide.md";

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
