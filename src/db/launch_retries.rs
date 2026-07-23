use crate::error::db::DbError;
use crate::types::{SessionName, Timestamp};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRetryRow {
    pub checkpoint: String,
    pub last_error: Option<String>,
    pub updated_at: Timestamp,
}

pub fn save_launch_checkpoint(
    conn: &Connection,
    session: &SessionName,
    checkpoint: &str,
    at: &Timestamp,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO session_launch_retries (
            session_name, checkpoint, last_error, updated_at
         ) VALUES (?1, ?2, NULL, ?3)
         ON CONFLICT(session_name) DO UPDATE SET
            checkpoint = excluded.checkpoint,
            last_error = NULL,
            updated_at = excluded.updated_at",
        params![session, checkpoint, at],
    )?;
    Ok(())
}

pub fn save_launch_error(
    conn: &Connection,
    session: &SessionName,
    detail: &str,
    at: &Timestamp,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE session_launch_retries
         SET last_error = ?1, updated_at = ?2
         WHERE session_name = ?3",
        params![detail, at, session],
    )?;
    Ok(())
}

pub fn find_launch_retry(
    conn: &Connection,
    session: &SessionName,
) -> Result<Option<LaunchRetryRow>, DbError> {
    conn.query_row(
        "SELECT checkpoint, last_error, updated_at
         FROM session_launch_retries
         WHERE session_name = ?1",
        params![session],
        |row| {
            Ok(LaunchRetryRow {
                checkpoint: row.get(0)?,
                last_error: row.get(1)?,
                updated_at: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(DbError::from)
}

pub fn delete_launch_retry(conn: &Connection, session: &SessionName) -> Result<(), DbError> {
    conn.execute(
        "DELETE FROM session_launch_retries WHERE session_name = ?1",
        params![session],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::open_catalog;
    use crate::db::sessions::{InsertSession, insert_session};
    use tempfile::tempdir;

    #[test]
    fn launch_retry_round_trips_updates_and_cascades() {
        let dir = tempdir().expect("tempdir");
        let conn = open_catalog(&dir.path().join("state.db")).expect("catalog");
        let session = SessionName::try_from("retry-demo").expect("session");
        insert_session(
            &conn,
            &InsertSession {
                name: session.clone(),
                ..crate::testing::test_sandbox_session(
                    "retry-demo",
                    crate::testing::ts("2026-07-23T00:00:00Z"),
                )
            },
        )
        .expect("session");

        let first = crate::testing::ts("2026-07-23T00:00:01Z");
        save_launch_checkpoint(&conn, &session, "vm_cloned", &first).expect("checkpoint");
        save_launch_error(&conn, &session, "start failed", &first).expect("error");
        assert_eq!(
            find_launch_retry(&conn, &session)
                .expect("find")
                .expect("retry"),
            LaunchRetryRow {
                checkpoint: "vm_cloned".to_owned(),
                last_error: Some("start failed".to_owned()),
                updated_at: first,
            }
        );

        let second = crate::testing::ts("2026-07-23T00:00:02Z");
        save_launch_checkpoint(&conn, &session, "vm_started", &second).expect("advance");
        let advanced = find_launch_retry(&conn, &session)
            .expect("find")
            .expect("retry");
        assert_eq!(advanced.checkpoint, "vm_started");
        assert_eq!(advanced.last_error, None);

        conn.execute("DELETE FROM sessions WHERE name = ?1", params![session])
            .expect("delete session");
        assert!(
            find_launch_retry(&conn, &session)
                .expect("find after delete")
                .is_none()
        );
    }

    #[test]
    fn delete_launch_retry_removes_checkpoint_only() {
        let dir = tempdir().expect("tempdir");
        let conn = open_catalog(&dir.path().join("state.db")).expect("catalog");
        let session = SessionName::try_from("retry-delete").expect("session");
        insert_session(
            &conn,
            &InsertSession {
                name: session.clone(),
                ..crate::testing::test_sandbox_session(
                    "retry-delete",
                    crate::testing::ts("2026-07-23T00:00:00Z"),
                )
            },
        )
        .expect("session");
        save_launch_checkpoint(
            &conn,
            &session,
            "vm_cloned",
            &crate::testing::ts("2026-07-23T00:00:01Z"),
        )
        .expect("checkpoint");

        delete_launch_retry(&conn, &session).expect("delete retry");
        assert!(
            find_launch_retry(&conn, &session)
                .expect("find retry")
                .is_none()
        );
        assert!(
            crate::db::sessions::find_session(&conn, &session)
                .expect("find session")
                .is_some()
        );
    }
}
