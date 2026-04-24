use agbranch::db::connect::open_catalog;
use agbranch::db::models::{LifecycleState, RepoSyncMode, SessionMode, SyncState};
use agbranch::db::sessions::{
    InsertSession, insert_session, update_lifecycle_state, update_lifecycle_state_with_timestamps,
    update_sync_state,
};
use agbranch::types::{GuestPath, HostPath, SessionName, Timestamp, VmName};
use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn ps_defaults_to_active_sessions_only() {
    let state_dir = seed_catalog().expect("catalog");

    let output = Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["ps", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let rows = session_names(&output);
    assert_eq!(rows, vec!["running-now", "needs-attention"]);
}

#[test]
fn ps_all_includes_stopped_and_closed_history() {
    let state_dir = seed_catalog().expect("catalog");

    let output = Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["ps", "--all", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let rows = session_names(&output);
    assert_eq!(
        rows,
        vec![
            "running-now",
            "needs-attention",
            "stopped-earlier",
            "closed-earlier"
        ]
    );
}

#[test]
fn ps_short_all_flag_includes_stopped_and_closed_history() {
    let state_dir = seed_catalog().expect("catalog");

    let output = Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["ps", "-a", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let rows = session_names(&output);
    assert_eq!(
        rows,
        vec![
            "running-now",
            "needs-attention",
            "stopped-earlier",
            "closed-earlier"
        ]
    );
}

#[test]
fn ps_without_active_sessions_shows_hint_for_all_flag() {
    let state_dir = seed_closed_only_catalog().expect("catalog");

    Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["ps"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "No active sessions. Use `agbranch ps -a` to show all sessions.",
        ));
}

#[test]
fn ps_all_without_active_sessions_shows_history_table() {
    let state_dir = seed_closed_only_catalog().expect("catalog");

    Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["ps", "--all"])
        .assert()
        .success()
        .stdout(predicates::str::contains("SESSION"))
        .stdout(predicates::str::contains("closed-earlier"));
}

#[test]
fn ps_json_includes_timing_fields_as_rfc3339_strings() {
    let state_dir = seed_catalog().expect("catalog");

    let output = Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["ps", "--all", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed = serde_json::from_slice::<Value>(&output).expect("valid json");
    let running = parsed
        .as_array()
        .expect("session array")
        .iter()
        .find(|row| row.get("name") == Some(&Value::String("running-now".to_owned())))
        .expect("running row");

    assert_eq!(running["created_at"], "2026-04-15T00:00:00Z");
    assert_eq!(running["last_started_at"], "2026-04-15T00:54:55Z");
    assert_eq!(running["last_stopped_at"], Value::Null);
    assert_eq!(running["closed_at"], Value::Null);
}

#[test]
fn ps_all_human_output_includes_timeline_columns() {
    let state_dir = seed_catalog().expect("catalog");

    Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["ps", "--all"])
        .assert()
        .success()
        .stdout(predicates::str::contains("CREATED"))
        .stdout(predicates::str::contains("LAST STARTED"))
        .stdout(predicates::str::contains("LAST STOPPED"))
        .stdout(predicates::str::contains("CLOSED"))
        .stdout(predicates::str::contains("2026-04-15T00:54:55Z"));
}

#[test]
fn ps_includes_running_sandbox_sessions() {
    let state_dir = tempdir().expect("tempdir");
    let conn = open_catalog(&state_dir.path().join("state.db")).expect("catalog");
    let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("timestamp");
    let started_at = Timestamp::parse_rfc3339("2026-04-15T00:10:00Z").expect("timestamp");

    insert_session(
        &conn,
        &InsertSession {
            name: SessionName::try_from("scratch-pad").expect("session"),
            vm_name: VmName::new("agbranch-scratch-pad"),
            session_mode: SessionMode::Sandbox,
            repo_sync_mode: None,
            host_context_path: None,
            guest_workspace_path: GuestPath::new("/home/alice.guest/sandbox/scratch-pad"),
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
            guest_tmux_socket_path: Some(GuestPath::new(
                "/home/alice.guest/.agbranch/tmux/scratch-pad.sock",
            )),
            shell_window_name: Some("shell".to_owned()),
            agent_window_name: Some("agent".to_owned()),
            agent_launch_preset: None,
            created_at,
        },
    )
    .expect("insert session");
    update_lifecycle_state_with_timestamps(
        &conn,
        &SessionName::try_from("scratch-pad").expect("session"),
        LifecycleState::Running,
        &started_at,
        Some(&started_at),
        None,
        None,
    )
    .expect("mark running");

    let output = Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["ps", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let rows = session_names(&output);
    assert_eq!(rows, vec!["scratch-pad"]);
}

fn seed_catalog() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let conn = open_catalog(&dir.path().join("state.db"))?;
    ensure_timing_columns(&conn)?;
    let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z")?;

    insert_session(
        &conn,
        &InsertSession {
            name: SessionName::try_from("running-now")?,
            vm_name: VmName::new("agbranch-running-now"),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new("/repo/running")),
            guest_workspace_path: GuestPath::new("/guest/running"),
            seed_host_path: None,
            host_git_root: Some(HostPath::new("/repo/running")),
            host_head_oid_at_open: Some("abc123".to_owned()),
            host_head_ref_at_open: Some("refs/heads/main".to_owned()),
            host_dirty_at_open: false,
            base_ref: Some("refs/heads/main".to_owned()),
            review_branch: Some("agbranch/running-now".to_owned()),
            session_ref_base: Some("refs/agbranch/sessions/running-now/base".to_owned()),
            session_ref_head: Some("refs/agbranch/sessions/running-now/head".to_owned()),
            provider_kind: None,
            imported_provider_files_json: "[]".to_owned(),
            guest_tmux_socket_path: None,
            shell_window_name: None,
            agent_window_name: None,
            agent_launch_preset: None,
            created_at,
        },
    )?;
    update_lifecycle_state(
        &conn,
        &SessionName::try_from("running-now")?,
        LifecycleState::Running,
        &created_at,
    )?;
    set_timing(
        &conn,
        "running-now",
        "2026-04-15T00:00:00Z",
        Some("2026-04-15T00:54:55Z"),
        None,
        None,
    )?;

    insert_session(
        &conn,
        &InsertSession {
            name: SessionName::try_from("needs-attention")?,
            vm_name: VmName::new("agbranch-needs-attention"),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new("/repo/error")),
            guest_workspace_path: GuestPath::new("/guest/error"),
            seed_host_path: None,
            host_git_root: Some(HostPath::new("/repo/error")),
            host_head_oid_at_open: Some("abc123".to_owned()),
            host_head_ref_at_open: Some("refs/heads/main".to_owned()),
            host_dirty_at_open: false,
            base_ref: Some("refs/heads/main".to_owned()),
            review_branch: Some("agbranch/needs-attention".to_owned()),
            session_ref_base: Some("refs/agbranch/sessions/needs-attention/base".to_owned()),
            session_ref_head: Some("refs/agbranch/sessions/needs-attention/head".to_owned()),
            provider_kind: None,
            imported_provider_files_json: "[]".to_owned(),
            guest_tmux_socket_path: None,
            shell_window_name: None,
            agent_window_name: None,
            agent_launch_preset: None,
            created_at,
        },
    )?;
    update_lifecycle_state(
        &conn,
        &SessionName::try_from("needs-attention")?,
        LifecycleState::Error,
        &created_at,
    )?;
    update_sync_state(
        &conn,
        &SessionName::try_from("needs-attention")?,
        SyncState::Error,
        &created_at,
    )?;
    set_timing(
        &conn,
        "needs-attention",
        "2026-04-15T00:00:00Z",
        Some("2026-04-15T00:30:00Z"),
        None,
        None,
    )?;

    insert_session(
        &conn,
        &InsertSession {
            name: SessionName::try_from("stopped-earlier")?,
            vm_name: VmName::new("agbranch-stopped-earlier"),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new("/repo/stopped")),
            guest_workspace_path: GuestPath::new("/guest/stopped"),
            seed_host_path: None,
            host_git_root: Some(HostPath::new("/repo/stopped")),
            host_head_oid_at_open: Some("abc123".to_owned()),
            host_head_ref_at_open: Some("refs/heads/main".to_owned()),
            host_dirty_at_open: false,
            base_ref: Some("refs/heads/main".to_owned()),
            review_branch: Some("agbranch/stopped-earlier".to_owned()),
            session_ref_base: Some("refs/agbranch/sessions/stopped-earlier/base".to_owned()),
            session_ref_head: Some("refs/agbranch/sessions/stopped-earlier/head".to_owned()),
            provider_kind: None,
            imported_provider_files_json: "[]".to_owned(),
            guest_tmux_socket_path: None,
            shell_window_name: None,
            agent_window_name: None,
            agent_launch_preset: None,
            created_at,
        },
    )?;
    update_lifecycle_state(
        &conn,
        &SessionName::try_from("stopped-earlier")?,
        LifecycleState::Stopped,
        &created_at,
    )?;
    update_sync_state(
        &conn,
        &SessionName::try_from("stopped-earlier")?,
        SyncState::Clean,
        &created_at,
    )?;
    set_timing(
        &conn,
        "stopped-earlier",
        "2026-04-15T00:00:00Z",
        Some("2026-04-15T00:10:00Z"),
        Some("2026-04-15T01:15:00Z"),
        None,
    )?;

    insert_session(
        &conn,
        &InsertSession {
            name: SessionName::try_from("closed-earlier")?,
            vm_name: VmName::new("agbranch-closed-earlier"),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new("/repo/closed")),
            guest_workspace_path: GuestPath::new("/guest/closed"),
            seed_host_path: None,
            host_git_root: Some(HostPath::new("/repo/closed")),
            host_head_oid_at_open: Some("abc123".to_owned()),
            host_head_ref_at_open: Some("refs/heads/main".to_owned()),
            host_dirty_at_open: false,
            base_ref: Some("refs/heads/main".to_owned()),
            review_branch: Some("agbranch/closed-earlier".to_owned()),
            session_ref_base: Some("refs/agbranch/sessions/closed-earlier/base".to_owned()),
            session_ref_head: Some("refs/agbranch/sessions/closed-earlier/head".to_owned()),
            provider_kind: None,
            imported_provider_files_json: "[]".to_owned(),
            guest_tmux_socket_path: None,
            shell_window_name: None,
            agent_window_name: None,
            agent_launch_preset: None,
            created_at,
        },
    )?;
    update_lifecycle_state(
        &conn,
        &SessionName::try_from("closed-earlier")?,
        LifecycleState::Closed,
        &created_at,
    )?;
    update_sync_state(
        &conn,
        &SessionName::try_from("closed-earlier")?,
        SyncState::Discarded,
        &created_at,
    )?;
    set_timing(
        &conn,
        "closed-earlier",
        "2026-04-15T00:00:00Z",
        Some("2026-04-15T00:10:00Z"),
        Some("2026-04-15T01:20:00Z"),
        Some("2026-04-15T01:20:00Z"),
    )?;

    Ok(dir)
}

fn seed_closed_only_catalog() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let conn = open_catalog(&dir.path().join("state.db"))?;
    ensure_timing_columns(&conn)?;
    let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z")?;

    insert_session(
        &conn,
        &InsertSession {
            name: SessionName::try_from("closed-earlier")?,
            vm_name: VmName::new("agbranch-closed-earlier"),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new("/repo/closed")),
            guest_workspace_path: GuestPath::new("/guest/closed"),
            seed_host_path: None,
            host_git_root: Some(HostPath::new("/repo/closed")),
            host_head_oid_at_open: Some("abc123".to_owned()),
            host_head_ref_at_open: Some("refs/heads/main".to_owned()),
            host_dirty_at_open: false,
            base_ref: Some("refs/heads/main".to_owned()),
            review_branch: Some("agbranch/closed-earlier".to_owned()),
            session_ref_base: Some("refs/agbranch/sessions/closed-earlier/base".to_owned()),
            session_ref_head: Some("refs/agbranch/sessions/closed-earlier/head".to_owned()),
            provider_kind: None,
            imported_provider_files_json: "[]".to_owned(),
            guest_tmux_socket_path: None,
            shell_window_name: None,
            agent_window_name: None,
            agent_launch_preset: None,
            created_at,
        },
    )?;
    update_lifecycle_state(
        &conn,
        &SessionName::try_from("closed-earlier")?,
        LifecycleState::Closed,
        &created_at,
    )?;
    update_sync_state(
        &conn,
        &SessionName::try_from("closed-earlier")?,
        SyncState::Discarded,
        &created_at,
    )?;
    set_timing(
        &conn,
        "closed-earlier",
        "2026-04-15T00:00:00Z",
        Some("2026-04-15T00:10:00Z"),
        Some("2026-04-15T01:20:00Z"),
        Some("2026-04-15T01:20:00Z"),
    )?;

    Ok(dir)
}

fn ensure_timing_columns(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('sessions')")?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if !names.iter().any(|name| name == "last_started_at") {
        conn.execute("ALTER TABLE sessions ADD COLUMN last_started_at TEXT", [])?;
    }
    if !names.iter().any(|name| name == "last_stopped_at") {
        conn.execute("ALTER TABLE sessions ADD COLUMN last_stopped_at TEXT", [])?;
    }
    Ok(())
}

fn set_timing(
    conn: &Connection,
    session: &str,
    created_at: &str,
    last_started_at: Option<&str>,
    last_stopped_at: Option<&str>,
    closed_at: Option<&str>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE sessions
         SET created_at = ?1,
             last_started_at = ?2,
             last_stopped_at = ?3,
             closed_at = ?4
         WHERE name = ?5",
        rusqlite::params![
            created_at,
            last_started_at,
            last_stopped_at,
            closed_at,
            session
        ],
    )?;
    Ok(())
}

fn session_names(stdout: &[u8]) -> Vec<String> {
    let parsed = serde_json::from_slice::<Value>(stdout).expect("valid json");
    parsed
        .as_array()
        .expect("session array")
        .iter()
        .map(|row| {
            row.get("name")
                .and_then(Value::as_str)
                .expect("name field")
                .to_owned()
        })
        .collect()
}
