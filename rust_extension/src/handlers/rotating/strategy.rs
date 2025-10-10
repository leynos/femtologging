//! Size-based rotation strategy for rotating file handlers.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use tempfile::NamedTempFile;

use super::fresh_failure::take_forced_fresh_failure_reason;
use crate::handlers::file::RotationStrategy;

pub(crate) struct FileRotationStrategy {
    path: PathBuf,
    max_bytes: u64,
    backup_count: usize,
}

impl FileRotationStrategy {
    pub(crate) fn new(path: PathBuf, max_bytes: u64, backup_count: usize) -> Self {
        Self {
            path,
            max_bytes,
            backup_count,
        }
    }

    pub(crate) fn next_record_bytes(message: &str) -> u64 {
        message.len() as u64 + 1
    }

    pub(crate) fn should_rotate(
        &self,
        writer: &BufWriter<File>,
        next_record_bytes: u64,
    ) -> io::Result<bool> {
        if self.max_bytes == 0 {
            return Ok(false);
        }
        let current_file_len = writer.get_ref().metadata()?.len();
        let buffered_bytes = writer.buffer().len() as u64;
        Ok(current_file_len + buffered_bytes + next_record_bytes > self.max_bytes)
    }

    pub(crate) fn rotate(&mut self, writer: &mut BufWriter<File>) -> io::Result<()> {
        writer.flush()?;
        if self.backup_count == 0 {
            let file = writer.get_mut();
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            return Ok(());
        }

        let capacity = writer.capacity();
        let original_file = self.swap_writer_with_temp(writer, capacity)?;
        let original_file = self.perform_rotation_with_rollback(writer, original_file, capacity)?;
        drop(original_file);
        self.finalise_rotation(writer, capacity)
    }

    fn swap_writer_with_temp(
        &self,
        writer: &mut BufWriter<File>,
        capacity: usize,
    ) -> io::Result<File> {
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        let temp = NamedTempFile::new_in(dir)?;
        let placeholder_file = temp.reopen()?;
        drop(temp);
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

    fn perform_rotation_with_rollback(
        &mut self,
        writer: &mut BufWriter<File>,
        original_file: File,
        capacity: usize,
    ) -> io::Result<File> {
        if let Err(err) = self.rotate_backups() {
            *writer = BufWriter::with_capacity(capacity, original_file);
            return Err(err);
        }
        Ok(original_file)
    }

    fn finalise_rotation(
        &mut self,
        writer: &mut BufWriter<File>,
        capacity: usize,
    ) -> io::Result<()> {
        if let Err(err) = Self::rename_file_if_exists(&self.path, &self.backup_path(1)) {
            *writer = BufWriter::with_capacity(capacity, Self::open_append_file(&self.path)?);
            return Err(err);
        }

        let fresh_writer = Self::open_fresh_writer(&self.path, capacity).or_else(|fresh_err| {
            let fallback = BufWriter::with_capacity(capacity, Self::open_append_file(&self.path)?);
            *writer = fallback;
            Err(fresh_err)
        })?;

        *writer = fresh_writer;
        Ok(())
    }

    fn open_fresh_writer(path: &Path, capacity: usize) -> io::Result<BufWriter<File>> {
        if let Some(reason) = take_forced_fresh_failure_reason() {
            return Err(io::Error::other(format!(
                "simulated fresh writer failure for testing ({reason})"
            )));
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(BufWriter::with_capacity(capacity, file))
    }

    fn open_append_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(path)
    }

    pub(crate) fn remove_file_if_exists(path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    pub(crate) fn rename_file_if_exists(src: &Path, dst: &Path) -> io::Result<()> {
        match fs::rename(src, dst) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    pub(crate) fn remove_excess_backups(&self) -> io::Result<()> {
        let mut extra = self.backup_count + 1;
        loop {
            let candidate = self.backup_path(extra);
            match fs::remove_file(&candidate) {
                Ok(()) => {
                    extra += 1;
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    break;
                }
                Err(err) => {
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn cascade_backups(&self) -> io::Result<()> {
        for idx in (1..self.backup_count).rev() {
            let src = self.backup_path(idx);
            if src.exists() {
                let dst = self.backup_path(idx + 1);
                Self::rename_file_if_exists(&src, &dst)?;
            }
        }
        Ok(())
    }

    pub(crate) fn rotate_backups(&self) -> io::Result<()> {
        if self.backup_count == 0 {
            return Ok(());
        }
        self.remove_excess_backups()?;
        let oldest = self.backup_path(self.backup_count);
        Self::remove_file_if_exists(&oldest)?;
        self.cascade_backups()?;
        Ok(())
    }

    pub(crate) fn backup_path(&self, index: usize) -> PathBuf {
        let mut backup = self.path.clone();
        let mut name = self
            .path
            .file_name()
            .map(|file_name| file_name.to_os_string())
            .unwrap_or_else(|| self.path.as_os_str().to_os_string());
        name.push(format!(".{index}"));
        backup.set_file_name(name);
        backup
    }
}

impl RotationStrategy<BufWriter<File>> for FileRotationStrategy {
    fn before_write(&mut self, writer: &mut BufWriter<File>, formatted: &str) -> io::Result<bool> {
        let next_bytes = Self::next_record_bytes(formatted);
        if self.should_rotate(writer, next_bytes)? {
            self.rotate(writer)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests covering rotation triggers and backup management.
    use super::*;
    use std::{fs, io::Write};
    use tempfile::tempdir;

    fn write_record(writer: &mut BufWriter<File>, message: &str) -> io::Result<()> {
        writeln!(writer, "{message}")?;
        writer.flush()
    }

    #[test]
    fn rotates_and_limits_backups() -> io::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("app.log");
        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);
        let mut strategy = FileRotationStrategy::new(path.clone(), 25, 2);

        for message in ["first record", "second record", "third record"] {
            strategy.before_write(&mut writer, message)?;
            write_record(&mut writer, message)?;
        }

        assert_eq!(fs::read_to_string(&path)?, "third record\n");
        assert_eq!(
            fs::read_to_string(strategy.backup_path(1))?,
            "second record\n"
        );
        assert_eq!(
            fs::read_to_string(strategy.backup_path(2))?,
            "first record\n"
        );
        assert!(!strategy.backup_path(3).exists());
        Ok(())
    }

    #[test]
    fn rotates_without_backups_when_disabled() -> io::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("app.log");
        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);
        let mut strategy = FileRotationStrategy::new(path.clone(), 8, 0);

        for message in ["alpha", "beta"] {
            strategy.before_write(&mut writer, message)?;
            write_record(&mut writer, message)?;
        }

        assert_eq!(fs::read_to_string(&path)?, "beta\n");
        assert!(!strategy.backup_path(1).exists());
        Ok(())
    }

    #[test]
    fn disables_rotation_when_max_bytes_is_zero() -> io::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("app.log");
        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);
        let mut strategy = FileRotationStrategy::new(path.clone(), 0, 3);

        for message in ["one", "two", "three"] {
            strategy.before_write(&mut writer, message)?;
            write_record(&mut writer, message)?;
        }

        assert_eq!(fs::read_to_string(&path)?, "one\ntwo\nthree\n");
        Ok(())
    }
}
