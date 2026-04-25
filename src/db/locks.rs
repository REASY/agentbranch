use crate::error::AppError;
use crate::error::db::DbError;
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    Exclusive,
    Shared,
}

#[derive(Debug)]
pub struct SessionLock {
    _file: File,
    _path: PathBuf,
}

impl SessionLock {
    pub fn acquire(path: &Path, pid: u32, operation: &str) -> Result<Self, DbError> {
        Self::acquire_with_mode(path, pid, operation, LockMode::Exclusive)
    }

    pub fn acquire_with_mode(
        path: &Path,
        pid: u32,
        operation: &str,
        mode: LockMode,
    ) -> Result<Self, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;

        let lock_result = match mode {
            LockMode::Exclusive => FileExt::try_lock_exclusive(&file),
            LockMode::Shared => FileExt::try_lock_shared(&file),
        };
        if let Err(err) = lock_result {
            if err.kind() == ErrorKind::WouldBlock {
                return Err(DbError::LockBusy);
            }
            return Err(DbError::Io(err));
        }

        write_lock_metadata(&mut file, pid, operation)?;

        Ok(Self {
            _file: file,
            _path: path.to_path_buf(),
        })
    }
}

pub fn acquire_base_lock(
    path: &Path,
    pid: u32,
    operation: &str,
    mode: LockMode,
) -> Result<SessionLock, AppError> {
    SessionLock::acquire_with_mode(path, pid, operation, mode).map_err(|err| match err {
        DbError::LockBusy => AppError::Blocked(render_base_busy_message(path)),
        other => other.into(),
    })
}

fn write_lock_metadata(file: &mut File, pid: u32, operation: &str) -> Result<(), DbError> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    writeln!(file, "pid={pid}")?;
    writeln!(file, "operation={operation}")?;
    file.flush()?;
    Ok(())
}

fn render_base_busy_message(path: &Path) -> String {
    if let Some(metadata) = read_lock_metadata(path) {
        format!(
            "base is busy: locked by pid {} (operation={})",
            metadata.pid, metadata.operation
        )
    } else {
        "base is busy: lock is held by another process".to_owned()
    }
}

struct LockMetadata {
    pid: u32,
    operation: String,
}

fn read_lock_metadata(path: &Path) -> Option<LockMetadata> {
    let contents = std::fs::read_to_string(path).ok()?;
    let mut pid = None;
    let mut operation = None;
    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("pid=") {
            pid = value.parse::<u32>().ok();
            continue;
        }
        if let Some(value) = line.strip_prefix("operation=")
            && !value.is_empty()
        {
            operation = Some(value.to_owned());
        }
    }
    Some(LockMetadata {
        pid: pid?,
        operation: operation?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use std::fs::OpenOptions;
    use tempfile::tempdir;

    #[test]
    fn second_lock_attempt_is_rejected_while_first_is_held() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("feat-a.lock");

        let _first = SessionLock::acquire(&path, 111, "open").expect("first lock");
        let second = SessionLock::acquire(&path, 222, "sync-back");

        assert!(second.is_err(), "second lock must fail while first is held");
    }

    #[test]
    fn shared_base_locks_can_coexist() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("base.lock");

        let _first =
            acquire_base_lock(&path, 111, "launch clone", LockMode::Shared).expect("first lock");
        let _second =
            acquire_base_lock(&path, 222, "open clone", LockMode::Shared).expect("second lock");
    }

    #[test]
    fn exclusive_base_lock_is_rejected_while_shared_holder_exists() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("base.lock");

        let _shared =
            acquire_base_lock(&path, 111, "launch clone", LockMode::Shared).expect("shared lock");
        let err = acquire_base_lock(&path, 222, "base prepare", LockMode::Exclusive)
            .expect_err("exclusive lock must fail");

        assert!(matches!(
            err,
            AppError::Blocked(message)
                if message == "base is busy: locked by pid 111 (operation=launch clone)"
        ));
    }

    #[test]
    fn shared_base_lock_is_rejected_while_exclusive_holder_exists() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("base.lock");

        let _exclusive = acquire_base_lock(&path, 111, "base prepare", LockMode::Exclusive)
            .expect("exclusive lock");
        let err = acquire_base_lock(&path, 222, "open clone", LockMode::Shared)
            .expect_err("shared lock must fail");

        assert!(matches!(
            err,
            AppError::Blocked(message)
                if message == "base is busy: locked by pid 111 (operation=base prepare)"
        ));
    }

    #[test]
    fn busy_base_lock_falls_back_when_metadata_is_malformed() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("base.lock");
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .expect("open lock file");
        file.try_lock_exclusive().expect("hold lock");
        file.set_len(0).expect("truncate");
        writeln!(file, "broken").expect("write malformed metadata");

        let err = acquire_base_lock(&path, 222, "base prepare", LockMode::Exclusive)
            .expect_err("lock must fail");

        assert!(matches!(
            err,
            AppError::Blocked(message)
                if message == "base is busy: lock is held by another process"
        ));
    }

    #[test]
    fn base_lock_is_created_lazily_on_first_acquire() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("locks").join("base.lock");

        assert!(!path.exists(), "lock file should not exist before acquire");

        let _lock =
            acquire_base_lock(&path, 111, "base prepare", LockMode::Exclusive).expect("lock");

        assert!(path.exists(), "lock file should be created on demand");
    }
}
