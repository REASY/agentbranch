use crate::error::db::DbError;
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};
use std::sync::LazyLock;

static MIGRATIONS: LazyLock<Migrations<'static>> =
    LazyLock::new(|| Migrations::new(vec![M::up(include_str!("../../migrations/0001_init.sql"))]));

pub fn run(conn: &mut Connection) -> Result<(), DbError> {
    MIGRATIONS.to_latest(conn).map_err(DbError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn clean_boot_applies_migration_and_creates_sessions_table() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("state.db");
        let mut conn = Connection::open(&path).expect("open");

        run(&mut conn).expect("migrate");

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("read user_version");
        assert_eq!(version, 1);

        let has_sessions: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        assert!(has_sessions);
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("state.db");
        let mut conn = Connection::open(&path).expect("open");

        run(&mut conn).expect("migrate 1");
        run(&mut conn).expect("migrate 2 — no-op");

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("read user_version");
        assert_eq!(version, 1);
    }

    #[test]
    fn new_schema_has_no_legacy_columns() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("state.db");
        let mut conn = Connection::open(&path).expect("open");
        run(&mut conn).expect("migrate");

        let mut stmt = conn
            .prepare("PRAGMA table_info(sessions)")
            .expect("table_info");
        let names: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect");

        assert!(
            !names.iter().any(|n| n == "id"),
            "column `id` should be gone"
        );
        assert!(
            !names.iter().any(|n| n == "repo_host_path"),
            "column `repo_host_path` should be gone"
        );
        assert!(
            !names.iter().any(|n| n == "repo_guest_path"),
            "column `repo_guest_path` should be gone"
        );
        assert!(names.iter().any(|n| n == "name"));
        assert!(names.iter().any(|n| n == "session_mode"));
    }
}
