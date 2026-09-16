//! Source discovery for the policy scan: which roots are governed, and every
//! Rust file under them.
//!
//! Separated from the judgement so that a change to what is read cannot be
//! confused with a change to what is refused, and so each file stays inside
//! the 400-line module limit.

use std::collections::VecDeque;

use camino::{Utf8Path, Utf8PathBuf};
use cap_std::{ambient_authority, fs_utf8::Dir};

use super::SOURCE_EXTENSION;

/// Directories the walk expects to find sources under.
///
/// These are not a filter. The walk starts at the crate directory and reads
/// every `.rs` file it can reach, so a Cargo target added outside them is
/// governed like anything else. They are a tripwire: a walk that silently
/// returned nothing, or stopped at the first directory, reports a failure
/// rather than a clean crate.
///
/// An earlier draft used them as the list of places to read. `lint-env-policy`
/// passes `--all-targets`, which compiles an `examples` target if one exists,
/// and no list written in advance governs a directory nobody has added yet.
pub(crate) const SOURCE_ROOTS: [&str; 3] = ["src", "tests", "benches"];

/// Directory names the walk does not descend into.
///
/// `target` holds build output, and a dot-prefixed name holds tool state such
/// as `.git`. Neither is a place a contributor writes a source Cargo compiles.
/// Everything else is walked, so an example, a build script or a second binary
/// is read wherever it is added.
pub(crate) fn is_walkable(name: &str) -> bool {
    !name.starts_with('.') && name != "target"
}

/// Read every `.rs` file in the crate, wherever it sits.
///
/// This is what the policy scan reads. `rust_sources` remains for the
/// per-root coverage assertions, which name a directory deliberately.
pub(crate) fn crate_sources() -> Result<Vec<(Utf8PathBuf, String)>, String> {
    rust_sources(&crate_dir(), "")
}

/// Return the crate directory, which holds every governed source.
pub(crate) fn crate_dir() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Collect every `.rs` file under one root, breadth first.
///
/// Read through a `cap_std` directory handle rather than `std::fs`, which the
/// repository's Dylint suite disallows: the scan needs to see files that do
/// not exist yet, so `include_str!` is not an option here as it is elsewhere
/// in this suite.
///
/// A root that cannot be opened is an error, not an empty result. An earlier
/// draft returned `Ok(Vec::new())` there, which made a renamed, deleted or
/// unreadable governed directory indistinguishable from one holding no
/// suppression: the scan reported success having read nothing.
pub(crate) fn rust_sources(
    root: &Utf8Path,
    relative: &str,
) -> Result<Vec<(Utf8PathBuf, String)>, String> {
    let directory = Dir::open_ambient_dir(root, ambient_authority())
        .map_err(|error| format!("open {relative}: {error}"))?;
    let mut pending = VecDeque::from([(directory, Utf8PathBuf::from(relative))]);
    let mut sources = Vec::new();

    while let Some((current, prefix)) = pending.pop_front() {
        let entries = current
            .entries()
            .map_err(|error| format!("read {prefix}: {error}"))?;
        for candidate in entries {
            let entry = candidate.map_err(|error| format!("entry under {prefix}: {error}"))?;
            let name = entry
                .file_name()
                .map_err(|error| format!("name under {prefix}: {error}"))?;
            let path = prefix.join(&name);
            let file_type = entry
                .file_type()
                .map_err(|error| format!("type of {path}: {error}"))?;
            if file_type.is_dir() {
                if !is_walkable(&name) {
                    continue;
                }
                let child = current
                    .open_dir(&name)
                    .map_err(|error| format!("open {path}: {error}"))?;
                pending.push_back((child, path));
            } else if path.extension() == Some(SOURCE_EXTENSION) {
                let contents = current
                    .read_to_string(&name)
                    .map_err(|error| format!("read {path}: {error}"))?;
                sources.push((path, contents));
            }
        }
    }
    Ok(sources)
}
