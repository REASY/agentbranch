use crate::db::models::EventLevel;
use crate::error::db::DbError;
use crate::types::{SessionName, Timestamp};
use rusqlite::{Connection, params};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SessionEventRow {
    pub id: i64,
    pub session_id: SessionName,
    pub at: Timestamp,
    pub level: EventLevel,
    pub kind: String,
    pub message: String,
}

pub fn append_event(
    conn: &Connection,
    session_id: &SessionName,
    level: EventLevel,
    kind: &str,
    message: &str,
    at: Timestamp,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO session_events (session_name, at, level, kind, message) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![session_id, at, level, kind, message],
    )?;
    Ok(())
}

pub fn list_events(
    conn: &Connection,
    session: Option<&SessionName>,
) -> Result<Vec<SessionEventRow>, DbError> {
    list_events_since(conn, session, 0)
}

pub fn list_events_since(
    conn: &Connection,
    session: Option<&SessionName>,
    after_id: i64,
) -> Result<Vec<SessionEventRow>, DbError> {
    let mut rows = Vec::new();
    if let Some(session) = session {
        let mut stmt = conn.prepare(
            "SELECT id, session_name, at, level, kind, message
             FROM session_events
             WHERE session_name = ?1 AND id > ?2
             ORDER BY id ASC",
        )?;
        let iter = stmt.query_map(params![session, after_id], map_session_event_row)?;
        for row in iter {
            rows.push(row?);
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, session_name, at, level, kind, message
             FROM session_events
             WHERE id > ?1
             ORDER BY id ASC",
        )?;
        let iter = stmt.query_map(params![after_id], map_session_event_row)?;
        for row in iter {
            rows.push(row?);
        }
    }
    Ok(rows)
}

pub fn latest_event_id(conn: &Connection, session: Option<&SessionName>) -> Result<i64, DbError> {
    let value = if let Some(session) = session {
        conn.query_row(
            "SELECT COALESCE(MAX(id), 0) AS latest_id FROM session_events WHERE session_name = ?1",
            params![session],
            |row| row.get("latest_id"),
        )?
    } else {
        conn.query_row(
            "SELECT COALESCE(MAX(id), 0) AS latest_id FROM session_events",
            [],
            |row| row.get("latest_id"),
        )?
    };
    Ok(value)
}

fn map_session_event_row(row: &rusqlite::Row<'_>) -> Result<SessionEventRow, rusqlite::Error> {
    Ok(SessionEventRow {
        id: row.get("id")?,
        session_id: row.get("session_name")?,
        at: row.get("at")?,
        level: row.get("level")?,
        kind: row.get("kind")?,
        message: row.get("message")?,
    })
}
