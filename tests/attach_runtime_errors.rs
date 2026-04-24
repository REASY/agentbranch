use agbranch::db::connect::open_catalog;
use agbranch::db::models::{LifecycleState, ProviderKind, RepoSyncMode, SessionMode, SyncState};
use agbranch::db::sessions::{
    InsertSession, insert_session, update_lifecycle_state_with_timestamps, update_sync_state,
};
use agbranch::types::{GuestPath, HostPath, SessionName, Timestamp, VmName};
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{TempDir, tempdir};

#[test]
fn attach_agent_reports_missing_agent_window_after_restart() {
    let state = setup_state().expect("state");
    let session = seed_agent_session(&state.db_path, "ops-attach").expect("seed session");
    let limactl_bin =
        install_fake_limactl_for_shell_only_runtime(state.path()).expect("fake limactl");
    let path_env = prepend_path(limactl_bin.parent().expect("parent"));

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &state.root)
        .env("LIMACTL_FIXTURE_ROOT", state.path())
        .env("PATH", path_env)
        .env("HOME", state.path().join("home"))
        .args(["attach", "--session", session.as_str(), "--agent"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "cannot attach to agent for session `ops-attach`",
        ))
        .stderr(predicate::str::contains(
            "agbranch agent start --session ops-attach --provider codex",
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
    fs::create_dir_all(dir.path().join("home"))?;
    let db_path = root.join("state.db");
    let _ = open_catalog(&db_path)?;
    Ok(TestState {
        _dir: dir,
        root,
        db_path,
    })
}

fn seed_agent_session(
    db_path: &Path,
    session_name: &str,
) -> Result<SessionName, Box<dyn std::error::Error>> {
    let conn = open_catalog(db_path)?;
    let session = SessionName::try_from(session_name)?;
    let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z")?;
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
            review_branch: Some(format!("agbranch/{session_name}")),
            session_ref_base: Some(format!("refs/agbranch/sessions/{session_name}/base")),
            session_ref_head: Some(format!("refs/agbranch/sessions/{session_name}/head")),
            provider_kind: Some(ProviderKind::Codex),
            imported_provider_files_json: "[]".to_owned(),
            guest_tmux_socket_path: Some(GuestPath::new(format!(
                "/home/tester.guest/.agbranch/tmux/{session_name}.sock"
            ))),
            shell_window_name: Some("shell".to_owned()),
            agent_window_name: Some("agent".to_owned()),
            agent_launch_preset: None,
            created_at,
        },
    )?;
    update_lifecycle_state_with_timestamps(
        &conn,
        &session,
        LifecycleState::Running,
        &created_at,
        Some(&created_at),
        None,
        None,
    )?;
    update_sync_state(&conn, &session, SyncState::Pending, &created_at)?;
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

fn install_fake_limactl_for_shell_only_runtime(
    temp_root: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let bin_dir = temp_root.join("bin");
    fs::create_dir_all(&bin_dir)?;
    let ssh_config = temp_root.join("ssh.config");
    fs::write(
        &ssh_config,
        "Host lima-agbranch-ops-attach\n  HostName 127.0.0.1\n",
    )?;

    let limactl = bin_dir.join("limactl");
    fs::write(
        &limactl,
        r#"#!/bin/sh
case "$1" in
  list)
    cat <<EOF
[{"name":"agbranch-ops-attach","status":"Running","vmType":"vz","dir":"$PWD/vm","sshConfigFile":"$LIMACTL_FIXTURE_ROOT/ssh.config"}]
EOF
    ;;
  shell)
    printf 'shell|0\n'
    ;;
  *)
    printf 'unexpected limactl args: %s\n' "$*" >&2
    exit 1
    ;;
esac
"#,
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
