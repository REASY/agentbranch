use crate::db::models::{SyncDirection, SyncRunResult};
use crate::error::db::DbError;
use crate::types::{SessionName, Timestamp};
use rusqlite::{Connection, params};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SyncRunRow {
    pub id: i64,
    pub session_id: SessionName,
    pub direction: SyncDirection,
    pub result: SyncRunResult,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    pub staging_path: Option<String>,
    pub patch_path: Option<String>,
    pub error_text: Option<String>,
}

pub fn insert_sync_run(
    conn: &Connection,
    session_id: &SessionName,
    direction: SyncDirection,
    result: SyncRunResult,
    started_at: Timestamp,
) -> Result<i64, DbError> {
    conn.execute(
        "INSERT INTO sync_runs (session_name, direction, result, started_at) VALUES (?1, ?2, ?3, ?4)",
        params![session_id, direction, result, started_at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn finish_sync_run(
    conn: &Connection,
    id: i64,
    result: SyncRunResult,
    finished_at: Timestamp,
    staging_path: Option<&str>,
    patch_path: Option<&str>,
    error_text: Option<&str>,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE sync_runs
         SET result = ?1, finished_at = ?2, staging_path = ?3, patch_path = ?4, error_text = ?5
         WHERE id = ?6",
        params![
            result,
            finished_at,
            staging_path,
            patch_path,
            error_text,
            id
        ],
    )?;
    Ok(())
}

pub fn list_sync_runs_for_session(
    conn: &Connection,
    session_id: &SessionName,
) -> Result<Vec<SyncRunRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, session_name, direction, result, started_at, finished_at,
                staging_path, patch_path, error_text
         FROM sync_runs
         WHERE session_name = ?1
         ORDER BY id DESC",
    )?;
    let iter = stmt.query_map(params![session_id], map_sync_run_row)?;
    let mut rows = Vec::new();
    for row in iter {
        rows.push(row?);
    }
    Ok(rows)
}

fn map_sync_run_row(row: &rusqlite::Row<'_>) -> Result<SyncRunRow, rusqlite::Error> {
    Ok(SyncRunRow {
        id: row.get("id")?,
        session_id: row.get("session_name")?,
        direction: row.get("direction")?,
        result: row.get("result")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
        staging_path: row.get("staging_path")?,
        patch_path: row.get("patch_path")?,
        error_text: row.get("error_text")?,
    })
}
