//! Publish extracted guide examples into the generated test tree.
//!
//! Cargo runs `build.rs` instead of a test harness, so the filesystem work it
//! performs cannot be unit-tested there. That work lives in this module, which
//! `tests/generated_files.rs` drives against a temporary directory and
//! `build.rs` calls for real. Both callers pass a [`Dir`] capability, so
//! neither reaches the ambient filesystem directly and the Dylint filesystem
//! policy applies unchanged.

use std::collections::BTreeSet;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

use cap_std::fs_utf8::Dir;

use crate::doc_examples::{Example, GENERATED_DIR};

/// Suffix of the in-progress file that is renamed into place.
const TEMP_SUFFIX: &str = ".tmp";

/// Extension of a generated module.
const GENERATED_EXTENSION: &str = ".rs";

/// Sequence that keeps one process's temporary names apart.
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Return a temporary name that no concurrent publisher can choose.
///
/// Two builds may publish the same example at once. A single shared temporary
/// path would let the first rename consume the file the second is about to
/// rename, so the second build would fail on a path that is not its own. The
/// process id separates builds and the sequence separates publications within
/// one build; the final rename into place stays atomic either way.
///
/// # Examples
///
/// Two calls never return the same path:
///
/// ```rust,no_run
/// assert_ne!(temporary_name(&example), temporary_name(&example));
/// ```
pub(crate) fn temporary_name(example: &Example) -> String {
    let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let process = std::process::id();
    format!(
        "{}.{process}.{sequence}{TEMP_SUFFIX}",
        example.output_name()
    )
}

/// Write each example into the generated directory.
///
/// Each module is renamed into place once it is complete, so a build that runs
/// alongside another one never exposes a half-written example.
///
/// # Examples
///
/// Publishing an example with the identifier `demo` leaves the module
/// `tests/generated/demo.rs` holding its source, and no temporary file behind:
///
/// ```rust,no_run
/// publish(&crate_dir, &[example])?;
/// ```
pub fn publish(crate_dir: &Dir, examples: &[Example]) -> io::Result<()> {
    for example in examples {
        let temp_name = temporary_name(example);
        crate_dir.write(&temp_name, &example.source)?;
        crate_dir.rename(&temp_name, crate_dir, example.output_name())?;
    }
    Ok(())
}

/// Remove generated modules left behind by an identifier that has gone.
///
/// Only modules this loader could have written are removed. Any other file in
/// the generated directory is left alone, so the directory can hold unrelated
/// output without being cleaned away.
///
/// # Examples
///
/// After a guide stops publishing `demo`, a stale
/// `tests/generated/demo.rs` is removed while `tests/generated/notes.txt`
/// survives:
///
/// ```rust,no_run
/// prune(&crate_dir, &remaining_examples)?;
/// ```
pub fn prune(crate_dir: &Dir, examples: &[Example]) -> io::Result<()> {
    let expected: BTreeSet<String> = examples.iter().map(Example::output_name).collect();
    for entry in crate_dir.read_dir(GENERATED_DIR)? {
        let name = format!("{GENERATED_DIR}/{}", entry?.file_name()?);
        if name.ends_with(GENERATED_EXTENSION) && !expected.contains(&name) {
            crate_dir.remove_file(&name)?;
        }
    }
    Ok(())
}
