use agbranch::db::connect::open_catalog;
use agbranch::db::events::append_event;
use agbranch::db::models::{
    EventLevel, LifecycleState, RepoSyncMode, SessionMode, SyncDirection, SyncRunResult, SyncState,
};
use agbranch::db::sessions::{
    InsertSession, find_session, insert_session, update_lifecycle_state_with_timestamps,
    update_sync_state,
};
use agbranch::db::sync_runs::{finish_sync_run, insert_sync_run};
use agbranch::types::{GuestPath, HostPath, SessionName, Timestamp, VmName};
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{TempDir, tempdir};

#[test]
fn show_renders_human_and_json_output() {
    let state = setup_state().expect("state");
    let session = seed_session(
        &state.db_path,
        "ops-show",
        LifecycleState::Running,
        SyncState::Pending,
    )
    .expect("seed session");

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &state.root)
        .args(["show", "--session", session.as_str()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Session: ops-show"))
        .stdout(predicate::str::contains("VM: agbranch-ops-show"))
        .stdout(predicate::str::contains("Mode: repo"))
        .stdout(predicate::str::contains("Lifecycle: running"))
        .stdout(predicate::str::contains("Sync: pending"))
        .stdout(predicate::str::contains("VM status: unavailable"))
        .stdout(predicate::str::contains("Runtime: unknown"));

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &state.root)
        .args(["show", "--session", session.as_str(), "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\":\"ops-show\""))
        .stdout(predicate::str::contains("\"lifecycle_state\":\"running\""))
        .stdout(predicate::str::contains("\"sync_state\":\"pending\""))
        .stdout(predicate::str::contains("\"runtime\""));
}

#[test]
fn show_missing_session_returns_validation_error() {
    let state = setup_state().expect("state");

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &state.root)
        .args(["show", "--session", "missing-session"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "session `missing-session` was not found",
        ));
}

#[test]
fn start_and_stop_update_session_timestamps_and_call_limactl() {
    let state = setup_state().expect("state");
    let session = seed_session(
        &state.db_path,
        "ops-cycle",
        LifecycleState::Stopped,
        SyncState::Pending,
    )
    .expect("seed session");
    let limactl_bin = install_fake_limactl(state.path()).expect("fake limactl");
    let limactl_log = state.path().join("limactl.log");
    let path_env = prepend_path(limactl_bin.parent().expect("parent"));

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &state.root)
        .env("PATH", path_env.clone())
        .env("LIMACTL_LOG", &limactl_log)
        .args(["start", "--session", session.as_str()])
        .assert()
        .success();

    let conn = open_catalog(&state.db_path).expect("catalog");
    let after_start = find_session(&conn, &session)
        .expect("lookup")
        .expect("session row");
    assert_eq!(after_start.lifecycle_state, LifecycleState::Running);
    assert!(after_start.last_started_at.is_some());

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &state.root)
        .env("PATH", path_env)
        .env("LIMACTL_LOG", &limactl_log)
        .args(["stop", "--session", session.as_str()])
        .assert()
        .success();

    let conn = open_catalog(&state.db_path).expect("catalog");
    let after_stop = find_session(&conn, &session)
        .expect("lookup")
        .expect("session row");
    assert_eq!(after_stop.lifecycle_state, LifecycleState::Stopped);
    assert!(after_stop.last_stopped_at.is_some());

    let limactl_calls = fs::read_to_string(&limactl_log).expect("limactl log");
    let lines: Vec<&str> = limactl_calls.lines().collect();
    assert_eq!(lines.len(), 2, "expected exactly two limactl calls");
    assert_eq!(lines[0], "start agbranch-ops-cycle");
    assert_eq!(lines[1], "stop agbranch-ops-cycle");
}

#[test]
fn start_rehydrates_tmux_shell_for_sessions_with_runtime_metadata() {
    let state = setup_state().expect("state");
    let conn = open_catalog(&state.db_path).expect("catalog");
    let session = SessionName::try_from("ops-rehydrate").expect("session");
    let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("created_at");
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
            host_git_root: Some(HostPath::new("/tmp/host")),
            host_head_oid_at_open: Some("abc123".to_owned()),
            host_head_ref_at_open: Some("refs/heads/main".to_owned()),
            host_dirty_at_open: false,
            base_ref: Some("refs/heads/main".to_owned()),
            review_branch: Some("agbranch/ops-rehydrate".to_owned()),
            session_ref_base: Some("refs/agbranch/sessions/ops-rehydrate/base".to_owned()),
            session_ref_head: Some("refs/agbranch/sessions/ops-rehydrate/head".to_owned()),
            provider_kind: None,
            imported_provider_files_json: "[]".to_owned(),
            guest_tmux_socket_path: Some(GuestPath::new(
                "/home/tester.guest/.agbranch/tmux/ops-rehydrate.sock",
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
        &session,
        LifecycleState::Stopped,
        &created_at,
        None,
        None,
        None,
    )
    .expect("mark stopped");
    update_sync_state(&conn, &session, SyncState::Pending, &created_at).expect("sync state");

    let limactl_bin = install_fake_limactl(state.path()).expect("fake limactl");
    let limactl_log = state.path().join("limactl.log");
    let path_env = prepend_path(limactl_bin.parent().expect("parent"));

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &state.root)
        .env("PATH", path_env)
        .env("LIMACTL_LOG", &limactl_log)
        .args(["start", "--session", session.as_str()])
        .assert()
        .success();

    let limactl_calls = fs::read_to_string(&limactl_log).expect("limactl log");
    let lines: Vec<&str> = limactl_calls.lines().collect();
    assert_eq!(lines.len(), 2, "expected start plus tmux rebootstrap");
    assert_eq!(lines[0], "start agbranch-ops-rehydrate");
    assert!(
        lines[1].starts_with("shell agbranch-ops-rehydrate -- bash -lc "),
        "expected guest bootstrap shell command, got {:?}",
        lines[1]
    );
    assert!(
        lines[1].contains("mkdir -p /home/test/repo"),
        "workspace should be recreated before tmux bootstrap: {:?}",
        lines[1]
    );
    assert!(
        lines[1].contains("tmux -S '/home/tester.guest/.agbranch/tmux/ops-rehydrate.sock'"),
        "tmux bootstrap should target the stored session socket: {:?}",
        lines[1]
    );
    assert!(
        lines[1].contains("new-session -d -s ops-rehydrate -n 'shell' -c /home/test/repo"),
        "start should recreate the detached shell session: {:?}",
        lines[1]
    );
}

#[test]
fn start_emits_progress_logs_for_vm_and_shell_rehydration() {
    let state = setup_state().expect("state");
    let conn = open_catalog(&state.db_path).expect("catalog");
    let session = SessionName::try_from("ops-progress").expect("session");
    let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("created_at");
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
            host_git_root: Some(HostPath::new("/tmp/host")),
            host_head_oid_at_open: Some("abc123".to_owned()),
            host_head_ref_at_open: Some("refs/heads/main".to_owned()),
            host_dirty_at_open: false,
            base_ref: Some("refs/heads/main".to_owned()),
            review_branch: Some("agbranch/ops-progress".to_owned()),
            session_ref_base: Some("refs/agbranch/sessions/ops-progress/base".to_owned()),
            session_ref_head: Some("refs/agbranch/sessions/ops-progress/head".to_owned()),
            provider_kind: None,
            imported_provider_files_json: "[]".to_owned(),
            guest_tmux_socket_path: Some(GuestPath::new(
                "/home/tester.guest/.agbranch/tmux/ops-progress.sock",
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
        &session,
        LifecycleState::Stopped,
        &created_at,
        None,
        None,
        None,
    )
    .expect("mark stopped");
    update_sync_state(&conn, &session, SyncState::Pending, &created_at).expect("sync state");

    let limactl_bin = install_fake_limactl(state.path()).expect("fake limactl");
    let limactl_log = state.path().join("limactl.log");
    let path_env = prepend_path(limactl_bin.parent().expect("parent"));

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &state.root)
        .env("PATH", path_env)
        .env("LIMACTL_LOG", &limactl_log)
        .args(["start", "--session", session.as_str()])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "start ops-progress: start-vm (phase ",
        ))
        .stderr(predicate::str::contains(
            "start ops-progress: ensure-shell (phase ",
        ))
        .stderr(predicate::str::contains(
            "start ops-progress: update-state (phase ",
        ))
        .stderr(predicate::str::contains(", total "));
}

#[test]
fn logs_events_json_emits_event_rows() {
    let state = setup_state().expect("state");
    let session = seed_session(
        &state.db_path,
        "ops-events",
        LifecycleState::Running,
        SyncState::Pending,
    )
    .expect("seed session");
    let conn = open_catalog(&state.db_path).expect("catalog");
    let at = Timestamp::parse_rfc3339("2026-04-15T00:30:00Z").expect("timestamp");
    append_event(
        &conn,
        &session,
        EventLevel::Info,
        "session.opened",
        "opened for log test",
        at,
    )
    .expect("append event");

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &state.root)
        .args([
            "logs",
            "--session",
            session.as_str(),
            "--source",
            "events",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"source\":\"events\""))
        .stdout(predicate::str::contains("\"kind\":\"session.opened\""))
        .stdout(predicate::str::contains(
            "\"message\":\"opened for log test\"",
        ));
}

#[test]
fn logs_sync_reads_file_when_sync_log_exists() {
    let state = setup_state().expect("state");
    let session = seed_session(
        &state.db_path,
        "ops-sync-file",
        LifecycleState::Running,
        SyncState::Pending,
    )
    .expect("seed session");
    let conn = open_catalog(&state.db_path).expect("catalog");
    let started_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("started timestamp");
    let finished_at = Timestamp::parse_rfc3339("2026-04-15T00:01:00Z").expect("finish timestamp");
    let run_id = insert_sync_run(
        &conn,
        &session,
        SyncDirection::SyncBack,
        SyncRunResult::Blocked,
        started_at,
    )
    .expect("insert sync run");
    finish_sync_run(
        &conn,
        run_id,
        SyncRunResult::Error,
        finished_at,
        Some("/tmp/staging"),
        None,
        Some("host drift"),
    )
    .expect("finish sync run");

    let session_log_dir = state.root.join("logs").join(session.as_str());
    fs::create_dir_all(&session_log_dir).expect("session log dir");
    fs::write(session_log_dir.join("sync.log"), "sync log from file\n").expect("sync log");

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &state.root)
        .args(["logs", "--session", session.as_str(), "--source", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync log from file"))
        .stdout(predicate::str::contains("host drift").not());
}

#[test]
fn logs_missing_source_returns_validation_error() {
    let state = setup_state().expect("state");
    let session = seed_session(
        &state.db_path,
        "ops-missing-log",
        LifecycleState::Running,
        SyncState::Pending,
    )
    .expect("seed session");

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &state.root)
        .args([
            "logs",
            "--session",
            session.as_str(),
            "--source",
            "provision",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "log source `provision` is not available for session `ops-missing-log`",
        ));
}

struct TestState {
    _dir: TempDir,
    root: PathBuf,
    db_path: PathBuf,
}

impl TestState {
    fn path(&self) -> &Path {
        self._dir.path()
    }
}

fn setup_state() -> Result<TestState, Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let root = dir.path().join("state");
    fs::create_dir_all(&root)?;
    fs::create_dir_all(root.join("logs"))?;
    fs::create_dir_all(root.join("staging"))?;
    fs::create_dir_all(root.join("locks"))?;
    let db_path = root.join("state.db");
    let _ = open_catalog(&db_path)?;
    Ok(TestState {
        _dir: dir,
        root,
        db_path,
    })
}

fn seed_session(
    db_path: &Path,
    session_name: &str,
    lifecycle: LifecycleState,
    sync: SyncState,
) -> Result<SessionName, Box<dyn std::error::Error>> {
    let conn = open_catalog(db_path)?;
    let session = SessionName::try_from(session_name)?;
    let vm_name = VmName::for_session(&session);
    let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z")?;
    insert_session(
        &conn,
        &InsertSession {
            name: session.clone(),
            vm_name,
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new("/tmp/host")),
            guest_workspace_path: GuestPath::new("/home/test/repo"),
            seed_host_path: None,
            host_git_root: Some(HostPath::new("/tmp/host")),
            host_head_oid_at_open: Some("abc123".to_owned()),
            host_head_ref_at_open: Some("refs/heads/main".to_owned()),
            host_dirty_at_open: false,
            base_ref: Some("refs/heads/main".to_owned()),
            review_branch: Some(format!("agbranch/{session_name}")),
            session_ref_base: Some(format!("refs/agbranch/sessions/{session_name}/base")),
            session_ref_head: Some(format!("refs/agbranch/sessions/{session_name}/head")),
            provider_kind: None,
            imported_provider_files_json: "[]".to_owned(),
            guest_tmux_socket_path: None,
            shell_window_name: None,
            agent_window_name: None,
            agent_launch_preset: None,
            created_at,
        },
    )?;
    update_lifecycle_state_with_timestamps(
        &conn,
        &session,
        lifecycle,
        &created_at,
        None,
        None,
        None,
    )?;
    update_sync_state(&conn, &session, sync, &created_at)?;
    Ok(session)
}

fn prepend_path(dir: &Path) -> String {
    let existing = std::env::var("PATH").unwrap_or_default();
    if existing.is_empty() {
        dir.display().to_string()
    } else {
        format!("{}:{existing}", dir.display())
    }
}

fn install_fake_limactl(temp_root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let bin_dir = temp_root.join("bin");
    fs::create_dir_all(&bin_dir)?;
    let limactl = bin_dir.join("limactl");
    fs::write(
        &limactl,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$LIMACTL_LOG\"\nexit 0\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&limactl)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&limactl, perms)?;
    }
    Ok(limactl)
}
