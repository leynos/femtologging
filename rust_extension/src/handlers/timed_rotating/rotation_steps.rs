//! The filesystem steps of one rotation, and their unwinding.
//!
//! A rotation replaces the handler's writer with a placeholder so the active
//! file can be renamed, then opens a fresh writer for the active path. Each
//! step can fail, and the handler must survive that: the original writer is
//! put back before anything else is attempted, and a rename that already moved
//! the active file is undone so the handler still points at the file it
//! started with.
//!
//! Undoing the rename can fail too, and then the active path is gone while the
//! timestamped file holds the only copy of its records. [`Rollback`] reports
//! that outcome rather than reducing it to an error message: the caller has to
//! remember the stranded name and restore it before a later rotation stages
//! over the same target.
//!
//! These steps are free functions rather than methods because they take the
//! rotation's state apart: [`RotationParts`] carries what the steps share, and
//! the strategy that owns the path, schedule and retention stays the only
//! caller.

use std::{
    fs::File,
    io::{self, BufWriter},
    path::{Path, PathBuf},
};

use super::rotation_store::RotationFileStore;

/// The state one rotation carries between its steps.
///
/// A rotation replaces the handler's writer with a temporary one so the active
/// file can be renamed. Every later step needs the same four pieces to finish
/// or to put the handler back as it was, so they travel together rather than as
/// a parameter list each step has to unpack.
pub(super) struct RotationParts<'a> {
    /// The handler's writer, rebuilt from the original file if a step fails.
    pub(super) writer: &'a mut BufWriter<File>,
    /// Buffer capacity used when the writer is rebuilt.
    pub(super) capacity: usize,
    /// The file the writer held before the temporary swap, until it is restored
    /// or deliberately dropped.
    pub(super) original_file: Option<File>,
    /// Timestamped destination the active file is renamed onto.
    pub(super) rotated_path: PathBuf,
}

impl RotationParts<'_> {
    /// Put the original file back, so the handler keeps working after a failed
    /// rotation step.
    pub(super) fn restore_writer(&mut self) {
        if let Some(file) = self.original_file.take() {
            *self.writer = BufWriter::with_capacity(self.capacity, file);
        }
    }
}

/// Temporarily swaps in a placeholder writer so the active file can be
/// renamed without losing the original on extraction failure.
pub(super) fn swap_writer_with_temp<S>(
    store: &S,
    directory: &Path,
    writer: &mut BufWriter<File>,
    capacity: usize,
) -> io::Result<File>
where
    S: RotationFileStore,
{
    let placeholder_file = store.placeholder_file(directory)?;
    let placeholder = BufWriter::with_capacity(capacity, placeholder_file);
    let original_writer = std::mem::replace(writer, placeholder);
    match original_writer.into_inner() {
        Ok(file) => Ok(file),
        Err(err) => {
            let io_error = io::Error::new(err.error().kind(), err.error().to_string());
            let original = err.into_inner();
            *writer = original;
            Err(io_error)
        }
    }
}

/// Clear the previous rotation target and rename the active file onto it.
///
/// Any failure restores the original file as the active writer, so the
/// caller observes a handler that is still usable and unchanged on disk.
pub(super) fn stage_rotated_file<S>(
    store: &S,
    active_path: &Path,
    rotation: &mut RotationParts<'_>,
) -> io::Result<()>
where
    S: RotationFileStore,
{
    if let Err(err) = store.remove_file_if_exists(&rotation.rotated_path) {
        rotation.restore_writer();
        return Err(err);
    }
    if let Err(err) = store.rename(active_path, &rotation.rotated_path) {
        rotation.restore_writer();
        return Err(err);
    }
    Ok(())
}

/// Open a truncated writer for the active path after a rollover.
pub(super) fn open_fresh_writer<S>(
    store: &S,
    active_path: &Path,
    capacity: usize,
) -> io::Result<BufWriter<File>>
where
    S: RotationFileStore,
{
    let file = store.open_truncated(active_path)?;
    Ok(BufWriter::with_capacity(capacity, file))
}

/// What undoing a rotation rename achieved.
///
/// The error is always present, because a rollback exists only to report why
/// the rotation failed. [`Rollback::is_restored`] says whether the records are
/// back under the active path or stranded under the timestamped one.
pub(super) struct Rollback {
    /// The failure to report, naming the rollback failure when there was one.
    pub(super) error: io::Error,
    /// Whether the rotated file was renamed back onto the active path.
    pub(super) is_restored: bool,
}

/// Undo a rotation rename, reporting both failures when the undo also fails.
///
/// The open error stays the cause so callers see why rotation started, while
/// the message names the rollback failure that left the handler degraded.
/// `is_restored` is `false` when that failure means the timestamped file is now
/// the only copy of the records and the active path is missing.
pub(super) fn rollback_rotation<S>(
    store: &S,
    active_path: &Path,
    rotated_path: &Path,
    cause: io::Error,
) -> Rollback
where
    S: RotationFileStore,
{
    if let Err(rollback_err) = store.rename(rotated_path, active_path) {
        return Rollback {
            error: io::Error::new(
                cause.kind(),
                format!(
                    concat!(
                        "failed to open fresh writer: {}; ",
                        "rollback rename also failed: {}"
                    ),
                    cause, rollback_err
                ),
            ),
            is_restored: false,
        };
    }
    Rollback {
        error: cause,
        is_restored: true,
    }
}
