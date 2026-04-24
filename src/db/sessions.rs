use crate::db::models::{
    AgentLaunchPreset, LifecycleState, ProviderKind, RepoSyncMode, SessionMode, SyncState,
    lifecycle_state_name, sync_state_name,
};
use crate::error::db::DbError;
use crate::types::{GuestPath, HostPath, SessionName, Timestamp, VmName};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use std::io;

pub struct InsertSession {
    pub name: SessionName,
    pub vm_name: VmName,
    pub session_mode: SessionMode,
    pub repo_sync_mode: Option<RepoSyncMode>,
    pub host_context_path: Option<HostPath>,
    pub guest_workspace_path: GuestPath,
    pub seed_host_path: Option<HostPath>,
    pub host_git_root: Option<HostPath>,
    pub host_head_oid_at_open: Option<String>,
    pub host_head_ref_at_open: Option<String>,
    pub host_dirty_at_open: bool,
    pub base_ref: Option<String>,
    pub review_branch: Option<String>,
    pub session_ref_base: Option<String>,
    pub session_ref_head: Option<String>,
    pub provider_kind: Option<ProviderKind>,
    pub imported_provider_files_json: String,
    pub guest_tmux_socket_path: Option<GuestPath>,
    pub shell_window_name: Option<String>,
    pub agent_window_name: Option<String>,
    pub agent_launch_preset: Option<AgentLaunchPreset>,
    pub created_at: Timestamp,
}

pub fn insert_session(conn: &Connection, input: &InsertSession) -> Result<(), DbError> {
    validate_session(input)?;

    conn.execute(
        "INSERT INTO sessions (
            name, vm_name,
            session_mode, repo_sync_mode,
            host_context_path, guest_workspace_path, seed_host_path,
            host_git_root, host_head_oid_at_open, host_head_ref_at_open, host_dirty_at_open,
            base_ref, review_branch, session_ref_base, session_ref_head,
            provider_kind, imported_provider_files_json, guest_tmux_socket_path,
            shell_window_name, agent_window_name, agent_launch_preset,
            lifecycle_state, sync_state,
            created_at, last_used_at
        ) VALUES (
            ?1, ?2,
            ?3, ?4,
            ?5, ?6, ?7,
            ?8, ?9, ?10, ?11,
            ?12, ?13, ?14, ?15,
            ?16, ?17, ?18,
            ?19, ?20, ?21,
            ?22, ?23,
            ?24, ?25
        )",
        params![
            &input.name,
            &input.vm_name,
            input.session_mode,
            input.repo_sync_mode,
            &input.host_context_path,
            &input.guest_workspace_path,
            &input.seed_host_path,
            &input.host_git_root,
            &input.host_head_oid_at_open,
            &input.host_head_ref_at_open,
            input.host_dirty_at_open,
            &input.base_ref,
            &input.review_branch,
            &input.session_ref_base,
            &input.session_ref_head,
            input.provider_kind,
            &input.imported_provider_files_json,
            &input.guest_tmux_socket_path,
            &input.shell_window_name,
            &input.agent_window_name,
            input.agent_launch_preset,
            lifecycle_state_name(LifecycleState::Cloning),
            sync_state_name(SyncState::Pending),
            &input.created_at,
            &input.created_at,
        ],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    pub name: SessionName,
    pub vm_name: VmName,
    pub session_mode: SessionMode,
    pub repo_sync_mode: Option<RepoSyncMode>,
    pub host_context_path: Option<HostPath>,
    pub guest_workspace_path: GuestPath,
    pub seed_host_path: Option<HostPath>,
    pub host_git_root: Option<HostPath>,
    pub host_head_oid_at_open: Option<String>,
    pub host_head_ref_at_open: Option<String>,
    pub host_dirty_at_open: bool,
    pub base_ref: Option<String>,
    pub review_branch: Option<String>,
    pub session_ref_base: Option<String>,
    pub session_ref_head: Option<String>,
    pub provider_kind: Option<ProviderKind>,
    pub imported_provider_files_json: String,
    pub guest_tmux_socket_path: Option<GuestPath>,
    pub shell_window_name: Option<String>,
    pub agent_window_name: Option<String>,
    pub agent_launch_preset: Option<AgentLaunchPreset>,
    pub lifecycle_state: LifecycleState,
    pub sync_state: SyncState,
    pub lock_owner_pid: Option<i64>,
    pub lock_operation: Option<String>,
    pub created_at: Timestamp,
    pub last_started_at: Option<Timestamp>,
    pub last_stopped_at: Option<Timestamp>,
    pub closed_at: Option<Timestamp>,
}

pub fn list_sessions(conn: &Connection) -> Result<Vec<SessionRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT name, vm_name, session_mode, repo_sync_mode,
                host_context_path, guest_workspace_path, seed_host_path,
                host_git_root, host_head_oid_at_open, host_head_ref_at_open, host_dirty_at_open,
                base_ref, review_branch, session_ref_base, session_ref_head,
                provider_kind, imported_provider_files_json, guest_tmux_socket_path,
                shell_window_name, agent_window_name, agent_launch_preset,
                lifecycle_state, sync_state, lock_owner_pid, lock_operation,
                created_at, last_started_at, last_stopped_at, closed_at
         FROM sessions
         ORDER BY last_used_at DESC",
    )?;
    let rows = stmt
        .query_map([], map_session_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn find_session(
    conn: &Connection,
    session_name: &SessionName,
) -> Result<Option<SessionRow>, DbError> {
    conn.query_row(
        "SELECT name, vm_name, session_mode, repo_sync_mode,
                host_context_path, guest_workspace_path, seed_host_path,
                host_git_root, host_head_oid_at_open, host_head_ref_at_open, host_dirty_at_open,
                base_ref, review_branch, session_ref_base, session_ref_head,
                provider_kind, imported_provider_files_json, guest_tmux_socket_path,
                shell_window_name, agent_window_name, agent_launch_preset,
                lifecycle_state, sync_state, lock_owner_pid, lock_operation,
                created_at, last_started_at, last_stopped_at, closed_at
         FROM sessions
         WHERE name = ?1",
        params![session_name],
        map_session_row,
    )
    .optional()
    .map_err(DbError::from)
}

pub fn find_session_by_vm_name(
    conn: &Connection,
    vm_name: &VmName,
) -> Result<Option<SessionRow>, DbError> {
    conn.query_row(
        "SELECT name, vm_name, session_mode, repo_sync_mode,
                host_context_path, guest_workspace_path, seed_host_path,
                host_git_root, host_head_oid_at_open, host_head_ref_at_open, host_dirty_at_open,
                base_ref, review_branch, session_ref_base, session_ref_head,
                provider_kind, imported_provider_files_json, guest_tmux_socket_path,
                shell_window_name, agent_window_name, agent_launch_preset,
                lifecycle_state, sync_state, lock_owner_pid, lock_operation,
                created_at, last_started_at, last_stopped_at, closed_at
         FROM sessions
         WHERE vm_name = ?1",
        params![vm_name],
        map_session_row,
    )
    .optional()
    .map_err(DbError::from)
}

pub fn update_lifecycle_state(
    conn: &Connection,
    session_name: &SessionName,
    lifecycle_state: LifecycleState,
    at: &Timestamp,
) -> Result<(), DbError> {
    update_lifecycle_state_with_timestamps(
        conn,
        session_name,
        lifecycle_state,
        at,
        None,
        None,
        None,
    )
}

pub fn update_lifecycle_state_with_timestamps(
    conn: &Connection,
    session_name: &SessionName,
    lifecycle_state: LifecycleState,
    at: &Timestamp,
    last_started_at: Option<&Timestamp>,
    last_stopped_at: Option<&Timestamp>,
    closed_at: Option<&Timestamp>,
) -> Result<(), DbError> {
    let last_started_at = last_started_at.map(Timestamp::as_rfc3339);
    let last_stopped_at = last_stopped_at.map(Timestamp::as_rfc3339);
    let closed_at = closed_at.map(Timestamp::as_rfc3339);
    conn.execute(
        "UPDATE sessions
         SET lifecycle_state = ?1,
             last_started_at = COALESCE(?2, last_started_at),
             last_stopped_at = COALESCE(?3, last_stopped_at),
             closed_at = COALESCE(?4, closed_at),
             last_used_at = ?5
         WHERE name = ?6",
        params![
            lifecycle_state_name(lifecycle_state),
            last_started_at,
            last_stopped_at,
            closed_at,
            at,
            session_name
        ],
    )?;
    Ok(())
}

pub fn set_lock_metadata(
    conn: &Connection,
    session_id: &SessionName,
    pid: u32,
    operation: &str,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE sessions SET lock_owner_pid = ?1, lock_operation = ?2 WHERE name = ?3",
        params![i64::from(pid), operation, session_id],
    )?;
    Ok(())
}

pub fn clear_lock_metadata(conn: &Connection, session_id: &SessionName) -> Result<(), DbError> {
    conn.execute(
        "UPDATE sessions SET lock_owner_pid = NULL, lock_operation = NULL WHERE name = ?1",
        params![session_id],
    )?;
    Ok(())
}

pub fn delete_session(conn: &Connection, session_id: &SessionName) -> Result<(), DbError> {
    conn.execute("DELETE FROM sessions WHERE name = ?1", params![session_id])?;
    Ok(())
}

pub fn update_sync_state(
    conn: &Connection,
    session_name: &SessionName,
    sync_state: SyncState,
    at: &Timestamp,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE sessions
         SET sync_state = ?1, last_used_at = ?2
         WHERE name = ?3",
        params![sync_state_name(sync_state), at, session_name],
    )?;
    Ok(())
}

pub fn update_agent_metadata(
    conn: &Connection,
    session_name: &SessionName,
    provider_kind: ProviderKind,
    imported_provider_files_json: &str,
    agent_launch_preset: AgentLaunchPreset,
    at: &Timestamp,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE sessions
         SET provider_kind = ?1,
             imported_provider_files_json = ?2,
             agent_launch_preset = ?3,
             last_used_at = ?4
         WHERE name = ?5",
        params![
            provider_kind,
            imported_provider_files_json,
            agent_launch_preset,
            at,
            session_name
        ],
    )?;
    Ok(())
}

fn map_session_row(row: &Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        name: row.get("name")?,
        vm_name: row.get("vm_name")?,
        session_mode: row.get("session_mode")?,
        repo_sync_mode: row.get("repo_sync_mode")?,
        host_context_path: row.get("host_context_path")?,
        guest_workspace_path: row.get("guest_workspace_path")?,
        seed_host_path: row.get("seed_host_path")?,
        host_git_root: row.get("host_git_root")?,
        host_head_oid_at_open: row.get("host_head_oid_at_open")?,
        host_head_ref_at_open: row.get("host_head_ref_at_open")?,
        host_dirty_at_open: row.get("host_dirty_at_open")?,
        base_ref: row.get("base_ref")?,
        review_branch: row.get("review_branch")?,
        session_ref_base: row.get("session_ref_base")?,
        session_ref_head: row.get("session_ref_head")?,
        provider_kind: row.get("provider_kind")?,
        imported_provider_files_json: row.get("imported_provider_files_json")?,
        guest_tmux_socket_path: row.get("guest_tmux_socket_path")?,
        shell_window_name: row.get("shell_window_name")?,
        agent_window_name: row.get("agent_window_name")?,
        agent_launch_preset: row.get("agent_launch_preset")?,
        lifecycle_state: row.get("lifecycle_state")?,
        sync_state: row.get("sync_state")?,
        lock_owner_pid: row.get("lock_owner_pid")?,
        lock_operation: row.get("lock_operation")?,
        created_at: row.get("created_at")?,
        last_started_at: row.get("last_started_at")?,
        last_stopped_at: row.get("last_stopped_at")?,
        closed_at: row.get("closed_at")?,
    })
}

fn validate_session(input: &InsertSession) -> Result<(), DbError> {
    match input.session_mode {
        SessionMode::Repo => {
            if input.host_context_path.is_none() {
                return Err(invalid_session("repo sessions require a host context path"));
            }
            if input.repo_sync_mode.is_none() {
                return Err(invalid_session("repo sessions require a repo sync mode"));
            }
        }
        SessionMode::Sandbox => {
            let sandbox_has_repo_metadata = input.repo_sync_mode.is_some()
                || input.host_context_path.is_some()
                || input.host_git_root.is_some()
                || input.host_head_oid_at_open.is_some()
                || input.host_head_ref_at_open.is_some()
                || input.base_ref.is_some()
                || input.review_branch.is_some()
                || input.session_ref_base.is_some()
                || input.session_ref_head.is_some();
            if sandbox_has_repo_metadata {
                return Err(invalid_session(
                    "sandbox sessions cannot carry repo metadata",
                ));
            }
        }
    }
    Ok(())
}

fn invalid_session(reason: &str) -> DbError {
    DbError::Io(io::Error::new(
        io::ErrorKind::InvalidInput,
        reason.to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::open_catalog;
    use crate::db::models::{AgentLaunchPreset, ProviderKind, RepoSyncMode, SessionMode};
    use tempfile::tempdir;

    #[test]
    fn session_rows_round_trip_agent_metadata() {
        let tmp = tempdir().expect("tempdir");
        let conn = open_catalog(&tmp.path().join("state.db")).expect("catalog");
        let created_at = Timestamp::parse_rfc3339("2026-04-18T00:00:00Z").expect("timestamp");
        let session_name = SessionName::try_from("agent-runtime").expect("session");

        insert_session(
            &conn,
            &InsertSession {
                name: session_name.clone(),
                vm_name: VmName::new("agbranch-agent-runtime"),
                session_mode: SessionMode::Repo,
                repo_sync_mode: Some(RepoSyncMode::GitNative),
                host_context_path: Some(HostPath::new("/tmp/repo")),
                guest_workspace_path: GuestPath::new("/home/lima/workspaces/agent-runtime/repo"),
                seed_host_path: None,
                host_git_root: Some(HostPath::new("/tmp/repo")),
                host_head_oid_at_open: Some("abc123".to_owned()),
                host_head_ref_at_open: Some("refs/heads/main".to_owned()),
                host_dirty_at_open: false,
                base_ref: Some("refs/heads/main".to_owned()),
                review_branch: Some("agbranch/agent-runtime".to_owned()),
                session_ref_base: Some("refs/agbranch/sessions/agent-runtime/base".to_owned()),
                session_ref_head: Some("refs/agbranch/sessions/agent-runtime/head".to_owned()),
                provider_kind: Some(ProviderKind::Codex),
                imported_provider_files_json: "[]".to_owned(),
                guest_tmux_socket_path: Some(GuestPath::new(
                    "/home/lima/.agbranch/tmux/agent-runtime.sock",
                )),
                shell_window_name: Some("shell".to_owned()),
                agent_window_name: Some("agent".to_owned()),
                agent_launch_preset: Some(AgentLaunchPreset::Unrestricted),
                created_at,
            },
        )
        .expect("insert session");

        let row = find_session(&conn, &session_name)
            .expect("lookup")
            .expect("row");
        assert_eq!(row.session_mode, SessionMode::Repo);
        assert_eq!(row.repo_sync_mode, Some(RepoSyncMode::GitNative));
        assert_eq!(row.review_branch.as_deref(), Some("agbranch/agent-runtime"));
        assert_eq!(row.provider_kind, Some(ProviderKind::Codex));
        assert_eq!(row.shell_window_name.as_deref(), Some("shell"));
        assert_eq!(
            row.agent_launch_preset,
            Some(AgentLaunchPreset::Unrestricted)
        );
    }

    #[test]
    fn session_validation_rejects_invalid_mode_metadata_combinations() {
        let tmp = tempdir().expect("tempdir");
        let conn = open_catalog(&tmp.path().join("state.db")).expect("catalog");
        let created_at = Timestamp::parse_rfc3339("2026-04-18T00:00:00Z").expect("timestamp");
        let session_name = SessionName::try_from("sandbox-invalid").expect("session");

        let sandbox_err = insert_session(
            &conn,
            &InsertSession {
                name: session_name.clone(),
                vm_name: VmName::new("agbranch-sandbox-invalid"),
                session_mode: SessionMode::Sandbox,
                repo_sync_mode: Some(RepoSyncMode::GitNative),
                host_context_path: Some(HostPath::new("/tmp/repo")),
                guest_workspace_path: GuestPath::new("/home/lima/sandbox/sandbox-invalid"),
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
                shell_window_name: Some("shell".to_owned()),
                agent_window_name: Some("agent".to_owned()),
                agent_launch_preset: Some(AgentLaunchPreset::Unrestricted),
                created_at,
            },
        )
        .expect_err("sandbox metadata should be rejected");
        assert!(
            sandbox_err
                .to_string()
                .contains("sandbox sessions cannot carry repo metadata")
        );

        let repo_err = insert_session(
            &conn,
            &InsertSession {
                name: SessionName::try_from("repo-invalid").expect("session"),
                vm_name: VmName::new("agbranch-repo-invalid"),
                session_mode: SessionMode::Repo,
                repo_sync_mode: None,
                host_context_path: None,
                guest_workspace_path: GuestPath::new("/home/lima/workspaces/repo-invalid/repo"),
                seed_host_path: None,
                host_git_root: Some(HostPath::new("/tmp/repo")),
                host_head_oid_at_open: Some("abc123".to_owned()),
                host_head_ref_at_open: Some("refs/heads/main".to_owned()),
                host_dirty_at_open: false,
                base_ref: Some("refs/heads/main".to_owned()),
                review_branch: Some("agbranch/repo-invalid".to_owned()),
                session_ref_base: Some("refs/agbranch/sessions/repo-invalid/base".to_owned()),
                session_ref_head: Some("refs/agbranch/sessions/repo-invalid/head".to_owned()),
                provider_kind: Some(ProviderKind::Codex),
                imported_provider_files_json: "[]".to_owned(),
                guest_tmux_socket_path: Some(GuestPath::new(
                    "/home/lima/.agbranch/tmux/repo-invalid.sock",
                )),
                shell_window_name: Some("shell".to_owned()),
                agent_window_name: Some("agent".to_owned()),
                agent_launch_preset: Some(AgentLaunchPreset::Unrestricted),
                created_at,
            },
        )
        .expect_err("repo sessions require git-native metadata");
        let repo_err_text = repo_err.to_string();
        assert!(
            repo_err_text.contains("repo sessions require a host context path")
                || repo_err_text.contains("repo sessions require a repo sync mode")
        );
    }

    #[test]
    fn open_records_a_running_session_after_clone_and_seed() {
        let dir = tempdir().expect("tempdir");
        let _conn = open_catalog(&dir.path().join("state.db")).expect("catalog");

        let result = crate::session::state::transition_after_open();
        assert_eq!(result, LifecycleState::Running);
    }

    fn seed(conn: &rusqlite::Connection, name: &str, mode: SessionMode) -> SessionName {
        let created_at = crate::testing::ts("2026-04-23T00:00:00Z");
        let record = match mode {
            SessionMode::Repo => crate::testing::test_repo_session(name, created_at),
            SessionMode::Sandbox => crate::testing::test_sandbox_session(name, created_at),
        };
        let session = record.name.clone();
        insert_session(conn, &record).expect("insert");
        session
    }

    #[test]
    fn sandbox_session_is_findable_by_find_session() {
        let dir = tempdir().expect("tempdir");
        let conn = open_catalog(&dir.path().join("state.db")).expect("catalog");
        let session = seed(&conn, "sandbox-one", SessionMode::Sandbox);

        let row = find_session(&conn, &session).expect("find").expect("row");
        assert_eq!(row.session_mode, SessionMode::Sandbox);
    }

    #[test]
    fn list_sessions_includes_sandbox_and_repo_rows() {
        let dir = tempdir().expect("tempdir");
        let conn = open_catalog(&dir.path().join("state.db")).expect("catalog");
        seed(&conn, "sandbox-one", SessionMode::Sandbox);
        seed(&conn, "repo-one", SessionMode::Repo);

        let rows = list_sessions(&conn).expect("list");
        let names: Vec<String> = rows.iter().map(|r| r.name.as_str().to_owned()).collect();
        assert!(names.contains(&"sandbox-one".to_owned()));
        assert!(names.contains(&"repo-one".to_owned()));
    }
}
