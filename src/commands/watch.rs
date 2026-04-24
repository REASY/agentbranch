use crate::cli::WatchArgs;
use crate::db::{
    connect::open_catalog,
    events::{SessionEventRow, latest_event_id, list_events_since},
    sessions::{SessionRow, find_session, list_sessions},
};
use crate::error::{AppError, observability::ObservabilityError};
use crate::platform::host::HostContext;
use crate::types::SessionName;
use crate::util::signals::install_interrupt_flag;
use crate::util::time::utc_now;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

const WATCH_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct LockState {
    held: bool,
    pid: Option<u32>,
    operation: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SnapshotLine {
    kind: &'static str,
    session: String,
    timestamp: crate::types::Timestamp,
    event_type: String,
    lifecycle_state: String,
    sync_state: String,
    lock_state: LockState,
}

#[derive(Debug, Clone, Serialize)]
struct EventLine {
    kind: &'static str,
    session: String,
    timestamp: crate::types::Timestamp,
    event_type: String,
    lifecycle_state: String,
    sync_state: String,
    lock_state: LockState,
    level: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionFingerprint {
    lifecycle_state: String,
    sync_state: String,
    lock_state: LockState,
}

struct InitialWatchState {
    lines: Vec<String>,
    seen: BTreeMap<String, SessionFingerprint>,
    last_event_id: i64,
}

pub fn render_snapshot(session: &str, lifecycle_state: &str, sync_state: &str) -> String {
    serde_json::json!({
        "kind": "snapshot",
        "session": session,
        "timestamp": utc_now(),
        "event_type": "snapshot.initial",
        "lifecycle_state": lifecycle_state,
        "sync_state": sync_state,
        "lock_state": {
            "held": false,
            "pid": serde_json::Value::Null,
            "operation": serde_json::Value::Null,
        }
    })
    .to_string()
}

pub fn run(args: WatchArgs) -> Result<(), AppError> {
    let session_filter = args
        .session
        .as_deref()
        .map(SessionName::try_from)
        .transpose()?;
    let host = HostContext::detect()?;
    let conn = open_catalog(&host.state_roots.db).map_err(ObservabilityError::from)?;
    let initial = initial_watch_state(&conn, session_filter.as_ref(), args.json)?;

    for line in initial.lines {
        println!("{line}");
    }

    let interrupted = install_interrupt_flag()?;
    let mut seen = initial.seen;
    let mut last_event_id = initial.last_event_id;

    loop {
        if interrupted.load(Ordering::SeqCst) {
            return Ok(());
        }

        thread::sleep(WATCH_POLL_INTERVAL);

        let sessions = filtered_sessions(&conn, session_filter.as_ref())?;
        for session in &sessions {
            let next = fingerprint(session);
            let key = session.name.to_string();
            if seen.get(&key) != Some(&next) {
                emit_snapshot(session, args.json)?;
                seen.insert(key, next);
            }
        }

        let events = list_events_since(&conn, session_filter.as_ref(), last_event_id)
            .map_err(ObservabilityError::from)?;
        if let Some(last) = events.last() {
            last_event_id = last.id;
        }
        emit_events(&events, args.json, &seen)?;
    }
}

pub fn initial_snapshot_lines(args: &WatchArgs) -> Result<Vec<String>, AppError> {
    let session_filter = args
        .session
        .as_deref()
        .map(SessionName::try_from)
        .transpose()?;
    let host = HostContext::detect()?;
    let conn = open_catalog(&host.state_roots.db).map_err(ObservabilityError::from)?;
    Ok(initial_watch_state(&conn, session_filter.as_ref(), args.json)?.lines)
}

fn initial_watch_state(
    conn: &rusqlite::Connection,
    session_filter: Option<&SessionName>,
    json: bool,
) -> Result<InitialWatchState, AppError> {
    if let Some(session) = session_filter.as_ref() {
        let exists = find_session(conn, session)
            .map_err(ObservabilityError::from)?
            .is_some();
        if !exists {
            return Err(AppError::Validation(
                crate::error::ValidationError::SessionNotFound(session.to_string()),
            ));
        }
    }

    let sessions = filtered_sessions(conn, session_filter)?;
    let seen = sessions
        .iter()
        .map(|row| (row.name.to_string(), fingerprint(row)))
        .collect();
    let last_event_id = latest_event_id(conn, session_filter).map_err(ObservabilityError::from)?;
    let lines = snapshot_lines(&sessions, json).map_err(ObservabilityError::from)?;
    Ok(InitialWatchState {
        lines,
        seen,
        last_event_id,
    })
}

fn filtered_sessions(
    conn: &rusqlite::Connection,
    session: Option<&SessionName>,
) -> Result<Vec<SessionRow>, ObservabilityError> {
    let rows = if let Some(session) = session {
        find_session(conn, session)?.into_iter().collect()
    } else {
        list_sessions(conn)?
    };
    Ok(rows)
}

fn fingerprint(row: &SessionRow) -> SessionFingerprint {
    SessionFingerprint {
        lifecycle_state: row.lifecycle_state.to_string(),
        sync_state: row.sync_state.to_string(),
        lock_state: LockState {
            held: row.lock_owner_pid.is_some(),
            pid: row.lock_owner_pid.map(|pid| pid as u32),
            operation: row.lock_operation.clone(),
        },
    }
}

fn snapshot_lines(rows: &[SessionRow], json: bool) -> Result<Vec<String>, serde_json::Error> {
    rows.iter().map(|row| snapshot_line(row, json)).collect()
}

fn emit_snapshot(row: &SessionRow, json: bool) -> Result<(), ObservabilityError> {
    println!("{}", snapshot_line(row, json)?);
    Ok(())
}

fn snapshot_line(row: &SessionRow, json: bool) -> Result<String, serde_json::Error> {
    let line = SnapshotLine {
        kind: "snapshot",
        session: row.name.to_string(),
        timestamp: utc_now(),
        event_type: "snapshot.initial".to_owned(),
        lifecycle_state: row.lifecycle_state.to_string(),
        sync_state: row.sync_state.to_string(),
        lock_state: fingerprint(row).lock_state,
    };
    if json {
        serde_json::to_string(&line)
    } else {
        Ok(format!(
            "snapshot\t{}\t{}\t{}\tlocked={}",
            line.session, line.lifecycle_state, line.sync_state, line.lock_state.held
        ))
    }
}

fn emit_events(
    events: &[SessionEventRow],
    json: bool,
    states: &BTreeMap<String, SessionFingerprint>,
) -> Result<(), ObservabilityError> {
    for line in event_lines(events, json, states)? {
        println!("{line}");
    }
    Ok(())
}

fn event_lines(
    events: &[SessionEventRow],
    json: bool,
    states: &BTreeMap<String, SessionFingerprint>,
) -> Result<Vec<String>, serde_json::Error> {
    events
        .iter()
        .map(|event| {
            let state =
                states
                    .get(event.session_id.as_str())
                    .cloned()
                    .unwrap_or(SessionFingerprint {
                        lifecycle_state: "unknown".to_owned(),
                        sync_state: "unknown".to_owned(),
                        lock_state: LockState {
                            held: false,
                            pid: None,
                            operation: None,
                        },
                    });
            let line = EventLine {
                kind: "event",
                session: event.session_id.to_string(),
                timestamp: event.at,
                event_type: event.kind.clone(),
                lifecycle_state: state.lifecycle_state,
                sync_state: state.sync_state,
                lock_state: state.lock_state,
                level: event.level.to_string(),
                message: event.message.clone(),
            };
            if json {
                serde_json::to_string(&line)
            } else {
                Ok(format!(
                    "event\t{}\t{}\t{}\t{}\t{}\t{}",
                    line.session,
                    line.timestamp,
                    line.event_type,
                    line.lifecycle_state,
                    line.sync_state,
                    line.message
                ))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{event_lines, initial_watch_state};
    use crate::cli::WatchArgs;
    use crate::db::connect::open_catalog;
    use crate::db::events::{SessionEventRow, append_event};
    use crate::db::models::{EventLevel, LifecycleState, RepoSyncMode, SessionMode, SyncState};
    use crate::db::sessions::{
        InsertSession, insert_session, update_lifecycle_state_with_timestamps, update_sync_state,
    };
    use crate::types::{GuestPath, HostPath, SessionName, Timestamp, VmName};
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn initial_watch_state_returns_lines_seen_and_latest_event_id_from_one_connection() {
        let dir = tempdir().expect("tempdir");
        let db = dir.path().join("state.db");
        let conn = open_catalog(&db).expect("catalog");
        let session = SessionName::try_from("watch-initial").expect("session");
        let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("timestamp");
        insert_session(
            &conn,
            &InsertSession {
                name: session.clone(),
                vm_name: VmName::for_session(&session),
                session_mode: SessionMode::Repo,
                repo_sync_mode: Some(RepoSyncMode::GitNative),
                host_context_path: Some(HostPath::new("/tmp/host")),
                guest_workspace_path: GuestPath::new("/home/test/repo"),
                seed_host_path: None,
                host_git_root: None,
                host_head_oid_at_open: None,
                host_head_ref_at_open: None,
                host_dirty_at_open: false,
                base_ref: None,
                review_branch: None,
                session_ref_base: None,
                session_ref_head: None,
                provider_kind: None,
                imported_provider_files_json: "[]".to_owned(),
                guest_tmux_socket_path: None,
                shell_window_name: None,
                agent_window_name: None,
                agent_launch_preset: None,
                created_at,
            },
        )
        .expect("insert");
        update_lifecycle_state_with_timestamps(
            &conn,
            &session,
            LifecycleState::Running,
            &created_at,
            Some(&created_at),
            None,
            None,
        )
        .expect("lifecycle");
        update_sync_state(&conn, &session, SyncState::Pending, &created_at).expect("sync state");
        append_event(
            &conn,
            &session,
            EventLevel::Info,
            "session.opened",
            "opened",
            created_at,
        )
        .expect("event");

        let initial = initial_watch_state(&conn, Some(&session), true).expect("initial state");
        assert_eq!(initial.lines.len(), 1);
        assert_eq!(initial.last_event_id, 1);
        let fingerprint = initial
            .seen
            .get(session.as_str())
            .expect("session fingerprint");
        assert_eq!(fingerprint.lifecycle_state, "running");
        assert_eq!(fingerprint.sync_state, "pending");
        assert!(!fingerprint.lock_state.held);
    }

    #[test]
    fn event_lines_render_human_and_json_shapes() {
        let timestamp = Timestamp::parse_rfc3339("2026-04-15T00:30:00Z").expect("timestamp");
        let events = vec![SessionEventRow {
            id: 7,
            session_id: SessionName::try_from("feat-a").expect("session"),
            at: timestamp,
            level: EventLevel::Warn,
            kind: "sync.blocked".to_owned(),
            message: "host drift".to_owned(),
        }];
        let states = BTreeMap::from([(
            "feat-a".to_owned(),
            super::SessionFingerprint {
                lifecycle_state: "running".to_owned(),
                sync_state: "blocked".to_owned(),
                lock_state: super::LockState {
                    held: true,
                    pid: Some(4242),
                    operation: Some("sync-back".to_owned()),
                },
            },
        )]);

        let human = event_lines(&events, false, &states).expect("human lines");
        assert_eq!(
            human,
            vec!["event\tfeat-a\t2026-04-15T00:30:00Z\tsync.blocked\trunning\tblocked\thost drift"]
        );

        let json = event_lines(&events, true, &states).expect("json lines");
        let value: serde_json::Value = serde_json::from_str(&json[0]).expect("json");
        assert_eq!(value["kind"], "event");
        assert_eq!(value["session"], "feat-a");
        assert_eq!(value["event_type"], "sync.blocked");
        assert_eq!(value["lifecycle_state"], "running");
        assert_eq!(value["sync_state"], "blocked");
        assert_eq!(value["level"], "warn");
        assert_eq!(value["message"], "host drift");
        assert_eq!(value["lock_state"]["held"], true);
        assert_eq!(value["lock_state"]["pid"], 4242);
        assert_eq!(value["lock_state"]["operation"], "sync-back");
    }

    #[test]
    fn watch_snapshot_is_ndjson_ready() {
        let line = render_snapshot("feat-a", "running", "pending");
        assert!(line.contains("\"kind\":\"snapshot\""));
        assert!(line.contains("\"session\":\"feat-a\""));
        assert!(line.contains("\"lifecycle_state\":\"running\""));
        assert!(line.contains("\"sync_state\":\"pending\""));
    }

    #[test]
    fn watch_snapshot_contains_lock_state_and_rfc3339_timestamp() {
        let line = render_snapshot("feat-b", "stopped", "clean");
        let value: serde_json::Value = serde_json::from_str(&line).expect("valid json");
        assert_eq!(value["kind"], "snapshot");
        assert_eq!(value["event_type"], "snapshot.initial");
        assert_eq!(value["session"], "feat-b");
        assert_eq!(value["lock_state"]["held"], false);
        assert!(value["lock_state"]["pid"].is_null());
        assert!(value["lock_state"]["operation"].is_null());
        let timestamp = value["timestamp"].as_str().expect("timestamp string");
        assert!(
            time::OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339)
                .is_ok(),
            "timestamp should be RFC3339, got: {timestamp}"
        );
    }

    #[test]
    fn watch_initial_snapshot_lines_cover_human_and_json_modes() {
        let dir = tempdir().expect("tempdir");
        let state_root = dir.path().join("state");
        fs::create_dir_all(&state_root).expect("state root");
        let db = state_root.join("state.db");
        let conn = open_catalog(&db).expect("catalog");
        let session = SessionName::try_from("watch-task5").expect("session");
        let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("timestamp");
        insert_session(
            &conn,
            &InsertSession {
                name: session.clone(),
                vm_name: VmName::for_session(&session),
                session_mode: SessionMode::Repo,
                repo_sync_mode: Some(RepoSyncMode::GitNative),
                host_context_path: Some(HostPath::new("/tmp/host")),
                guest_workspace_path: GuestPath::new("/home/test/repo"),
                seed_host_path: None,
                host_git_root: None,
                host_head_oid_at_open: None,
                host_head_ref_at_open: None,
                host_dirty_at_open: false,
                base_ref: None,
                review_branch: None,
                session_ref_base: None,
                session_ref_head: None,
                provider_kind: None,
                imported_provider_files_json: "[]".to_owned(),
                guest_tmux_socket_path: None,
                shell_window_name: None,
                agent_window_name: None,
                agent_launch_preset: None,
                created_at,
            },
        )
        .expect("insert");
        update_lifecycle_state_with_timestamps(
            &conn,
            &session,
            LifecycleState::Running,
            &created_at,
            None,
            None,
            None,
        )
        .expect("lifecycle");
        update_sync_state(&conn, &session, SyncState::Pending, &created_at).expect("sync state");

        let human_lines = temp_env::with_var(
            "AGBRANCH_STATE_ROOT",
            Some(state_root.as_os_str()),
            || -> Vec<String> {
                initial_snapshot_lines(&WatchArgs {
                    session: Some(session.to_string()),
                    json: false,
                })
                .expect("human snapshot")
            },
        );
        assert_eq!(human_lines.len(), 1);
        assert_eq!(
            human_lines[0],
            "snapshot\twatch-task5\trunning\tpending\tlocked=false"
        );

        let json_lines = temp_env::with_var(
            "AGBRANCH_STATE_ROOT",
            Some(state_root.as_os_str()),
            || -> Vec<String> {
                initial_snapshot_lines(&WatchArgs {
                    session: Some(session.to_string()),
                    json: true,
                })
                .expect("json snapshot")
            },
        );
        assert_eq!(json_lines.len(), 1);
        let value: serde_json::Value = serde_json::from_str(&json_lines[0]).expect("valid json");
        assert_eq!(value["kind"], "snapshot");
        assert_eq!(value["event_type"], "snapshot.initial");
        assert_eq!(value["session"], "watch-task5");
        assert_eq!(value["lifecycle_state"], "running");
        assert_eq!(value["sync_state"], "pending");
        assert_eq!(value["lock_state"]["held"], false);
    }
}
