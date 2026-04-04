//! I/O helpers shared by the file handler and worker.
//!
//! These helpers isolate file-opening and record-writing details so the main
//! handler module can stay focused on API wiring.

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
};

use super::worker::FlushTracker;

/// Open a log file in append mode and attach the file path to I/O errors.
pub(super) fn open_log_file<P: AsRef<Path>>(path: P) -> io::Result<File> {
    let path_ref = path.as_ref();
    let path_display = path_ref.display();
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path_ref)
        .map_err(|e| io::Error::new(e.kind(), format!("{path_display}: {e}")))
}

/// Write one formatted record and delegate periodic flushing to the tracker.
pub(super) fn write_record<W>(
    writer: &mut W,
    message: &str,
    flush_tracker: &mut FlushTracker,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "{message}")?;
    flush_tracker.record_write(writer)
}
