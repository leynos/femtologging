//! The filesystem seam behind the timed-rotation strategy.
//!
//! One rotation is a chain of filesystem steps — remove the previous target,
//! rename the active file, open a replacement writer, scan the directory for
//! backups, delete the oldest — and each step can fail after an earlier one
//! already succeeded. Driving a specific failure used to mean arranging a real
//! directory that refuses a rename at the right moment, which a test can only
//! approximate.
//!
//! [`RotationFileStore`] names those steps so the strategy asks for them
//! instead of calling `std::fs` itself. [`StdFileStore`] is the production
//! implementation; tests substitute a scripted store that reports a directory
//! listing from memory and fails a named step on demand.

use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io,
    path::Path,
};

use tempfile::NamedTempFile;

/// The filesystem operations a timed rotation performs.
///
/// The trait is deliberately narrow: it names the five steps rotation needs so
/// a test double can answer each one, and so no other capability leaks out to
/// a caller that only wanted a rotation.
pub(crate) trait RotationFileStore: Send + Sync {
    /// Rename the active log onto its timestamped rotation target.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;

    /// Remove a file, treating an absent file as already removed.
    ///
    /// Rotation retries a target that an earlier rotation may already have
    /// pruned, so "not found" is success rather than a failure the caller has
    /// to translate.
    fn remove_file_if_exists(&self, path: &Path) -> io::Result<()>;

    /// Open a truncated writer for the active path after a rollover.
    fn open_truncated(&self, path: &Path) -> io::Result<File>;

    /// List the file names in a directory so retention can find backups.
    fn read_file_names(&self, directory: &Path) -> io::Result<Vec<OsString>>;

    /// Open a placeholder file that lets the active file be renamed.
    ///
    /// The returned handle is unlinked as soon as it is created and stays
    /// usable, so extracting the original writer cannot lose the active file
    /// if the rename that follows fails.
    fn placeholder_file(&self, directory: &Path) -> io::Result<File>;
}

/// [`RotationFileStore`] backed by `std::fs`.
///
/// A zero-sized type: the handler contract mirrors CPython's
/// `TimedRotatingFileHandler`, which opens, renames and deletes whatever
/// ambient path the Python caller supplied, so there is no root to scope these
/// operations to.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct StdFileStore;

impl RotationFileStore for StdFileStore {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn remove_file_if_exists(&self, path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    fn open_truncated(&self, path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
    }

    fn read_file_names(&self, directory: &Path) -> io::Result<Vec<OsString>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(directory)? {
            names.push(entry?.file_name());
        }
        Ok(names)
    }

    fn placeholder_file(&self, directory: &Path) -> io::Result<File> {
        let temp = NamedTempFile::new_in(directory)?;
        let file = temp.reopen()?;
        // The reopened handle stays valid after the temporary path is removed,
        // so the placeholder never outlives the rotation that created it.
        drop(temp);
        Ok(file)
    }
}

#[cfg(test)]
#[path = "rotation_store_double.rs"]
pub(super) mod double;
