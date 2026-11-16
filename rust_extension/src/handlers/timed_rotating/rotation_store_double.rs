//! A scripted [`RotationFileStore`] for rotation tests.
//!
//! The real store answers with whatever the filesystem holds, which makes two
//! kinds of test awkward: a retention scan needs a directory containing the
//! exact backup names under test, and a failed step needs the failure to
//! happen at a chosen point in the rotation. This double reports its listing
//! from memory, records the steps it was asked for, and can fail one named
//! step, so both cases are arranged in the test rather than on disk.

use std::{
    ffi::OsString,
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use parking_lot::Mutex;

use super::{RotationFileStore, StdFileStore};

/// The filesystem steps a rotation can ask a store for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoreStep {
    /// [`RotationFileStore::rename`].
    Rename,
    /// [`RotationFileStore::remove_file_if_exists`].
    Remove,
    /// [`RotationFileStore::open_truncated`].
    Open,
    /// [`RotationFileStore::read_file_names`].
    List,
    /// [`RotationFileStore::placeholder_file`].
    Placeholder,
}

/// A [`RotationFileStore`] whose answers the test decides.
///
/// Cloning shares the recorded calls, so a test can hand one clone to the
/// strategy under test and read the record from the one it kept.
#[derive(Clone)]
pub(crate) struct ScriptedFileStore {
    /// Real store used only for steps that must yield a file handle.
    files: StdFileStore,
    /// Names [`RotationFileStore::read_file_names`] reports.
    listing: Vec<OsString>,
    /// Files the store was asked to remove, in the order it was asked.
    removed: Arc<Mutex<Vec<PathBuf>>>,
    /// Renames the store was asked to perform, as `(from, to)` pairs.
    renames: Arc<Mutex<Vec<(PathBuf, PathBuf)>>>,
    /// Step that fails instead of doing its work, if the test chose one.
    failing: Option<StoreStep>,
}

impl ScriptedFileStore {
    /// A store that lists `listing` and performs every step for real.
    pub(crate) fn with_listing(listing: &[&str]) -> Self {
        Self {
            files: StdFileStore,
            listing: listing.iter().map(OsString::from).collect(),
            removed: Arc::new(Mutex::new(Vec::new())),
            renames: Arc::new(Mutex::new(Vec::new())),
            failing: None,
        }
    }

    /// A store whose `step` reports an error instead of performing it.
    pub(crate) fn failing_at(mut self, step: StoreStep) -> Self {
        self.failing = Some(step);
        self
    }

    /// The paths the store was asked to remove, in order.
    pub(crate) fn removed(&self) -> Vec<PathBuf> {
        self.removed.lock().clone()
    }

    /// The renames the store was asked to perform, in order.
    pub(crate) fn renames(&self) -> Vec<(PathBuf, PathBuf)> {
        self.renames.lock().clone()
    }

    /// Report the scripted failure for `step`, if the test chose it.
    fn refuse(&self, step: StoreStep) -> io::Result<()> {
        if self.failing == Some(step) {
            return Err(io::Error::other(format!("scripted {step:?} failure")));
        }
        Ok(())
    }
}

impl RotationFileStore for ScriptedFileStore {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.refuse(StoreStep::Rename)?;
        self.renames
            .lock()
            .push((from.to_path_buf(), to.to_path_buf()));
        Ok(())
    }

    fn remove_file_if_exists(&self, path: &Path) -> io::Result<()> {
        self.refuse(StoreStep::Remove)?;
        self.removed.lock().push(path.to_path_buf());
        Ok(())
    }

    fn open_truncated(&self, path: &Path) -> io::Result<File> {
        self.refuse(StoreStep::Open)?;
        self.files.open_truncated(path)
    }

    fn read_file_names(&self, _directory: &Path) -> io::Result<Vec<OsString>> {
        self.refuse(StoreStep::List)?;
        Ok(self.listing.clone())
    }

    fn placeholder_file(&self, directory: &Path) -> io::Result<File> {
        self.refuse(StoreStep::Placeholder)?;
        self.files.placeholder_file(directory)
    }
}
