use crate::error::db::DbError;
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub struct SessionLock {
    _file: File,
    _path: PathBuf,
}

impl SessionLock {
    pub fn acquire(path: &Path, pid: u32, operation: &str) -> Result<Self, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;

        if let Err(err) = file.try_lock_exclusive() {
            if err.kind() == ErrorKind::WouldBlock {
                return Err(DbError::LockBusy);
            }
            return Err(DbError::Io(err));
        }

        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "pid={pid}")?;
        writeln!(file, "operation={operation}")?;

        Ok(Self {
            _file: file,
            _path: path.to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn second_lock_attempt_is_rejected_while_first_is_held() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("feat-a.lock");

        let _first = SessionLock::acquire(&path, 111, "open").expect("first lock");
        let second = SessionLock::acquire(&path, 222, "sync-back");

        assert!(second.is_err(), "second lock must fail while first is held");
    }
}
