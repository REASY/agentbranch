use crate::db::migrate;
use crate::error::db::DbError;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

pub fn open_catalog(path: &Path) -> Result<Connection, DbError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;

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
        assert_eq!(version, 1);
    }
}
