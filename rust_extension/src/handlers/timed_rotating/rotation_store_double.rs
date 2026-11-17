//! A scripted [`RotationFileStore`] for rotation tests.
//!
//! The real store answers with whatever the filesystem holds, which makes two
//! kinds of test awkward: a retention scan needs a directory containing the
//! exact backup names under test, and a failed step needs the failure to
//! happen at a chosen point in the rotation. This double reports its listing
//! from memory, records the steps it was asked for, and can fail a named step
//! or a rename onto a chosen destination, so both cases are arranged in the
//! test rather than on disk.
//!
//! Every step it does not refuse is performed for real, so a test can inspect
//! what the filesystem holds after the strategy gave up or recovered.

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
/// Cloning shares the recorded calls and the remaining scripted failures, so a
/// test can hand one clone to the strategy under test and read the record from
/// the one it kept.
#[derive(Clone)]
pub(crate) struct ScriptedFileStore {
    /// Real store used for every step the test did not refuse.
    files: StdFileStore,
    /// Names [`RotationFileStore::read_file_names`] reports.
    listing: Vec<OsString>,
    /// Files the store was asked to remove, in the order it was asked.
    removed: Arc<Mutex<Vec<PathBuf>>>,
    /// Renames the store was asked to perform, as `(from, to)` pairs.
    renames: Arc<Mutex<Vec<(PathBuf, PathBuf)>>>,
    /// Steps with a scripted failure still to spend.
    failing: Arc<Mutex<Vec<StoreStep>>>,
    /// Rename destinations with a scripted failure still to spend.
    failing_onto: Arc<Mutex<Vec<PathBuf>>>,
}

impl ScriptedFileStore {
    /// A store that lists `listing` and performs every step for real.
    pub(crate) fn with_listing(listing: &[&str]) -> Self {
        Self {
            files: StdFileStore,
            listing: listing.iter().map(OsString::from).collect(),
            removed: Arc::new(Mutex::new(Vec::new())),
            renames: Arc::new(Mutex::new(Vec::new())),
            failing: Arc::new(Mutex::new(Vec::new())),
            failing_onto: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Make the next `step` report an error instead of performing it.
    ///
    /// One failure per call, so a second `failing_at` for the same step makes
    /// two calls fail. A retry within the same test therefore sees the step
    /// behaving normally, which is how a transient failure is arranged.
    pub(crate) fn failing_at(self, step: StoreStep) -> Self {
        self.failing.lock().push(step);
        self
    }

    /// Make the next rename onto `destination` report an error.
    ///
    /// A rollback and the recovery that retries it both rename back onto the
    /// active path, while the forward rename before them targets the
    /// timestamped name. Failing by destination is what lets a test break the
    /// rollback without breaking the forward rename or the recovery after it.
    pub(crate) fn failing_rename_onto(self, destination: &Path) -> Self {
        self.failing_onto.lock().push(destination.to_path_buf());
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

    /// Report the scripted failure for `step`, if one is still to spend.
    fn refuse(&self, step: StoreStep) -> io::Result<()> {
        let mut failing = self.failing.lock();
        let Some(position) = failing.iter().position(|planned| *planned == step) else {
            return Ok(());
        };
        failing.remove(position);
        Err(io::Error::other(format!("scripted {step:?} failure")))
    }

    /// Report the scripted failure for a rename onto `destination`, if one is
    /// still to spend.
    fn refuse_rename_onto(&self, destination: &Path) -> io::Result<()> {
        let mut failing = self.failing_onto.lock();
        let Some(position) = failing.iter().position(|planned| planned == destination) else {
            return Ok(());
        };
        failing.remove(position);
        Err(io::Error::other(format!(
            "scripted rename onto {} failure",
            destination.display()
        )))
    }
}

impl RotationFileStore for ScriptedFileStore {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.refuse(StoreStep::Rename)?;
        self.refuse_rename_onto(to)?;
        self.files.rename(from, to)?;
        self.renames
            .lock()
            .push((from.to_path_buf(), to.to_path_buf()));
        Ok(())
    }

    fn remove_file_if_exists(&self, path: &Path) -> io::Result<()> {
        self.refuse(StoreStep::Remove)?;
        self.files.remove_file_if_exists(path)?;
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
