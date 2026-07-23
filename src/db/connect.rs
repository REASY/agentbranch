use crate::db::migrate;
use crate::error::db::DbError;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

const CATALOG_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub fn open_catalog(path: &Path) -> Result<Connection, DbError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;

    conn.busy_timeout(CATALOG_BUSY_TIMEOUT)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    if is_pre_migration_catalog(&conn)? {
        return Err(DbError::PreMigrationCatalog {
            path: path.to_path_buf(),
        });
    }

    migrate::run(&mut conn)?;
    Ok(conn)
}

fn is_pre_migration_catalog(conn: &Connection) -> Result<bool, DbError> {
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if user_version != 0 {
        return Ok(false);
    }
    let sessions_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !sessions_exists {
        return Ok(false);
    }
    let row_count: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
    Ok(row_count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn pre_migration_catalog_is_refused() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("state.db");

        {
            let conn = Connection::open(&path).expect("open");
            conn.execute_batch(
                "CREATE TABLE sessions (name TEXT PRIMARY KEY, vm_name TEXT NOT NULL);
                 INSERT INTO sessions (name, vm_name) VALUES ('old', 'agbranch-old');",
            )
            .expect("seed");
        }

        let err = open_catalog(&path).expect_err("should refuse pre-migration db");
        match err {
            DbError::PreMigrationCatalog {
                path: returned_path,
            } => {
                assert_eq!(returned_path, path);
            }
            other => panic!("expected PreMigrationCatalog, got {other:?}"),
        }
    }

    #[test]
    fn empty_pre_existing_sessions_table_is_fine() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("state.db");

        {
            let conn = Connection::open(&path).expect("open");
            conn.execute_batch(
                "CREATE TABLE sessions (name TEXT PRIMARY KEY, vm_name TEXT NOT NULL);",
            )
            .expect("seed empty");
        }

        let _conn = open_catalog(&path)
            .expect("empty pre-existing sessions table should NOT trigger PreMigrationCatalog");
    }

    #[test]
    fn fresh_file_migrates_cleanly() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("state.db");

        let conn = open_catalog(&path).expect("open");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("version");
        assert_eq!(version, 3);
    }

    #[test]
    fn catalog_waits_for_a_concurrent_writer() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("state.db");
        let mut first = open_catalog(&path).expect("first connection");
        let second = open_catalog(&path).expect("second connection");
        first
            .execute_batch("CREATE TABLE concurrency_probe (value INTEGER NOT NULL)")
            .expect("probe table");
        let tx = first.transaction().expect("writer transaction");
        tx.execute("INSERT INTO concurrency_probe VALUES (1)", [])
            .expect("first insert");

        let (started_tx, started_rx) = mpsc::channel();
        let writer = thread::spawn(move || {
            started_tx.send(()).expect("started");
            second.execute("INSERT INTO concurrency_probe VALUES (2)", [])
        });
        started_rx.recv().expect("writer started");
        thread::sleep(Duration::from_millis(100));
        tx.commit().expect("release writer lock");

        assert_eq!(
            writer
                .join()
                .expect("writer thread")
                .expect("second insert"),
            1
        );
        let count: i64 = first
            .query_row("SELECT COUNT(*) FROM concurrency_probe", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 2);
    }
}
