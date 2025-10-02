//! Size-based rotation strategy for rotating file handlers.

use std::{
    fs::{self, File},
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

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
        self.rotate_backups()?;
        if self.path.exists() {
            fs::copy(&self.path, self.backup_path(1))?;
        }
        let file = writer.get_mut();
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        Ok(())
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
    fn before_write(&mut self, writer: &mut BufWriter<File>, formatted: &str) -> io::Result<()> {
        let next_bytes = Self::next_record_bytes(formatted);
        if self.should_rotate(writer, next_bytes)? {
            self.rotate(writer)?;
        }
        Ok(())
    }
}
