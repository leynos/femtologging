//! Pruning of timestamped backups after a rotation.
//!
//! Retention answers one question — which rotated siblings of the active log
//! survive — and changes for one reason: the configured backup count. The
//! cadence decides what a backup *is named*, which is why it arrives as a
//! suffix predicate rather than as a second copy of the schedule rules.
//!
//! The directory is read and pruned through
//! [`RotationFileStore`](super::rotation_store::RotationFileStore), so a test
//! can present a listing without creating one.

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

use super::rotation_store::RotationFileStore;

/// Deletes the oldest timestamped backups beyond a configured count.
pub(super) struct BackupRetention {
    /// Active log path whose timestamped siblings are pruned.
    active_path: PathBuf,
    /// Maximum number of timestamped files retained after rotation.
    backup_count: usize,
}

impl BackupRetention {
    /// Retain at most `backup_count` rotated siblings of `active_path`.
    pub(super) fn new(active_path: PathBuf, backup_count: usize) -> Self {
        Self {
            active_path,
            backup_count,
        }
    }

    /// Remove timestamped files beyond the configured retention count.
    ///
    /// `is_valid_suffix` answers whether a trailing component names a rotation
    /// this handler's cadence produces, so an unrelated file that happens to
    /// share the prefix is left alone. Oldest-first ordering is by path, which
    /// is chronological because the suffix leads with its timestamp.
    pub(super) fn prune<S>(
        &self,
        store: &S,
        is_valid_suffix: impl Fn(&str) -> bool,
    ) -> io::Result<()>
    where
        S: RotationFileStore,
    {
        if self.backup_count == 0 {
            return Ok(());
        }
        let directory = backup_directory(&self.active_path);
        let prefix = self.backup_prefix();
        let mut backups: Vec<PathBuf> = store
            .read_file_names(directory)?
            .into_iter()
            .filter(|name| is_backup_name(name, &prefix, &is_valid_suffix))
            .map(|name| directory.join(name))
            .collect();
        backups.sort();
        while backups.len() > self.backup_count {
            let oldest = backups.remove(0);
            store.remove_file_if_exists(&oldest)?;
        }
        Ok(())
    }

    /// The `<active file name>.` prefix every backup of this log shares.
    fn backup_prefix(&self) -> OsString {
        let mut prefix = self
            .active_path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_else(|| self.active_path.as_os_str().to_os_string());
        prefix.push(".");
        prefix
    }
}

/// Check whether a name is a rotated sibling of the active log.
///
/// Matching is prefix-then-suffix: the name must start with the active file's
/// name and a dot, and what follows must be a suffix the cadence produces.
fn is_backup_name(
    name: &OsString,
    prefix: &OsString,
    is_valid_suffix: &impl Fn(&str) -> bool,
) -> bool {
    if !has_os_prefix(name, prefix) {
        return false;
    }
    let name = name.to_string_lossy();
    let prefix = prefix.to_string_lossy();
    let Some(suffix) = name.strip_prefix(prefix.as_ref()) else {
        return false;
    };
    is_valid_suffix(suffix)
}

/// Compares encoded path prefixes without requiring UTF-8 conversion.
pub(super) fn has_os_prefix(value: &OsString, prefix: &OsString) -> bool {
    value
        .as_os_str()
        .as_encoded_bytes()
        .starts_with(prefix.as_os_str().as_encoded_bytes())
}

/// The directory that holds a log path's rotated siblings.
///
/// A single-component path such as `app.log` has a parent of `""`, not `None`,
/// and `read_dir("")` fails on Unix. Retention must therefore treat an empty
/// parent as the current directory — which is what the empty string means —
/// or it silently skips every rollover.
pub(super) fn backup_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(test)]
mod tests {
    //! Unit tests for locating the directory a rotation's backups live in.

    use super::backup_directory;
    use std::path::Path;

    /// A bare filename must scan the current directory, not the empty path.
    #[test]
    fn a_bare_file_name_scans_the_current_directory() {
        assert_eq!(backup_directory(Path::new("app.log")), Path::new("."));
    }

    /// A directory component is kept as it is written.
    #[test]
    fn a_parent_directory_is_preserved() {
        assert_eq!(
            backup_directory(Path::new("logs/app.log")),
            Path::new("logs")
        );
        assert_eq!(
            backup_directory(Path::new("/var/log/app.log")),
            Path::new("/var/log")
        );
    }
}
