use agbranch::db::connect::open_catalog;
use agbranch::db::models::{AgentLaunchPreset, ProviderKind, RepoSyncMode, SessionMode};
use agbranch::db::sessions::{InsertSession, insert_session};
use agbranch::types::{GuestPath, HostPath, SessionName, Timestamp, VmName};
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

#[test]
fn show_json_surfaces_runtime_metadata() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("state");
    fs::create_dir_all(&root).expect("state root");
    fs::create_dir_all(root.join("logs")).expect("logs");
    fs::create_dir_all(root.join("staging")).expect("staging");
    fs::create_dir_all(root.join("locks")).expect("locks");
    let db_path = root.join("state.db");
    let conn = open_catalog(&db_path).expect("catalog");

    let session = SessionName::try_from("feature-x").expect("session");
    insert_session(
        &conn,
        &InsertSession {
            name: session.clone(),
            vm_name: VmName::new("agbranch-feature-x"),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new("/host/repo")),
            guest_workspace_path: GuestPath::new("/home/tester.guest/workspaces/feature-x/repo"),
            seed_host_path: None,
            host_git_root: Some(HostPath::new("/host/repo")),
            host_head_oid_at_open: Some("abc123".to_owned()),
            host_head_ref_at_open: Some("refs/heads/main".to_owned()),
            host_dirty_at_open: false,
            base_ref: Some("refs/heads/main".to_owned()),
            review_branch: Some("agbranch/feature-x".to_owned()),
            session_ref_base: Some("refs/agbranch/sessions/feature-x/base".to_owned()),
            session_ref_head: Some("refs/agbranch/sessions/feature-x/head".to_owned()),
            provider_kind: Some(ProviderKind::Claude),
            imported_provider_files_json: serde_json::to_string(&vec![serde_json::json!({
                "host_path": "/Users/tester/.claude.json",
                "guest_path": "/home/tester.guest/.claude.json"
            })])
            .expect("imports"),
            guest_tmux_socket_path: Some(GuestPath::new(
                "/home/tester.guest/.agbranch/tmux/feature-x.sock",
            )),
            shell_window_name: Some("shell".to_owned()),
            agent_window_name: Some("agent".to_owned()),
            agent_launch_preset: Some(AgentLaunchPreset::Unrestricted),
            created_at: Timestamp::parse_rfc3339("2026-04-18T00:00:00Z").expect("timestamp"),
        },
    )
    .expect("insert session");

    let limactl_bin = install_runtime_limactl(temp.path()).expect("fake limactl");
    let limactl_list_json = temp.path().join("limactl-list.json");
    fs::write(
        &limactl_list_json,
        r#"[{
  "name": "agbranch-feature-x",
  "dir": "~/.lima/agbranch-feature-x",
  "sshConfigFile": "~/.lima/agbranch-feature-x/ssh.config",
  "vmType": "vz",
  "status": "Running",
  "arch": "aarch64",
  "cpus": 4,
  "memory": 4294967296,
  "disk": 107374182400,
  "sshLocalPort": 60022,
  "sshAddress": "127.0.0.1",
  "config": {
    "mounts": []
  }
}]"#,
    )
    .expect("list json");
    let path_env = prepend_path(limactl_bin.parent().expect("parent"));

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &root)
        .env("PATH", path_env)
        .env("LIMACTL_LIST_JSON", &limactl_list_json)
        .env("LIMACTL_TMUX_OUTPUT", "shell|0\nagent|0\n")
        .args(["show", "--session", session.as_str(), "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"session_mode\":\"repo\""))
        .stdout(predicate::str::contains(
            "\"review_branch\":\"agbranch/feature-x\"",
        ))
        .stdout(predicate::str::contains("\"provider\":\"claude\""))
        .stdout(predicate::str::contains(
            "\"socket\":\"/home/tester.guest/.agbranch/tmux/feature-x.sock\"",
        ))
        .stdout(predicate::str::contains("\"runtime\""))
        .stdout(predicate::str::contains("\"status\":\"running\""))
        .stdout(predicate::str::contains("\"agent\":\"claude\""))
        .stdout(predicate::str::contains("\"state\":\"live\""))
        .stdout(predicate::str::contains(
            "\"guest_path\":\"/home/tester.guest/.claude.json\"",
        ));
}

#[test]
fn show_human_surfaces_live_runtime_details() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("state");
    fs::create_dir_all(&root).expect("state root");
    fs::create_dir_all(root.join("logs")).expect("logs");
    fs::create_dir_all(root.join("staging")).expect("staging");
    fs::create_dir_all(root.join("locks")).expect("locks");
    let db_path = root.join("state.db");
    let conn = open_catalog(&db_path).expect("catalog");

    let session = SessionName::try_from("ops-show").expect("session");
    insert_session(
        &conn,
        &InsertSession {
            name: session.clone(),
            vm_name: VmName::new("agbranch-ops-show"),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new("/host/repo")),
            guest_workspace_path: GuestPath::new("/home/tester.guest/workspaces/ops-show/repo"),
            seed_host_path: None,
            host_git_root: Some(HostPath::new("/host/repo")),
            host_head_oid_at_open: Some("abc123".to_owned()),
            host_head_ref_at_open: Some("refs/heads/main".to_owned()),
            host_dirty_at_open: false,
            base_ref: Some("refs/heads/main".to_owned()),
            review_branch: Some("agbranch/ops-show".to_owned()),
            session_ref_base: Some("refs/agbranch/sessions/ops-show/base".to_owned()),
            session_ref_head: Some("refs/agbranch/sessions/ops-show/head".to_owned()),
            provider_kind: Some(ProviderKind::Codex),
            imported_provider_files_json: serde_json::to_string(&vec![serde_json::json!({
                "host_path": "/Users/tester/.codex/auth.json",
                "guest_path": "/home/tester.guest/.codex/auth.json"
            })])
            .expect("imports"),
            guest_tmux_socket_path: Some(GuestPath::new(
                "/home/tester.guest/.agbranch/tmux/ops-show.sock",
            )),
            shell_window_name: Some("shell".to_owned()),
            agent_window_name: Some("agent".to_owned()),
            agent_launch_preset: Some(AgentLaunchPreset::Unrestricted),
            created_at: Timestamp::parse_rfc3339("2026-04-18T00:00:00Z").expect("timestamp"),
        },
    )
    .expect("insert session");

    let limactl_bin = install_runtime_limactl(temp.path()).expect("fake limactl");
    let limactl_list_json = temp.path().join("limactl-list.json");
    fs::write(
        &limactl_list_json,
        r#"[{
  "name": "agbranch-ops-show",
  "dir": "~/.lima/agbranch-ops-show",
  "sshConfigFile": "~/.lima/agbranch-ops-show/ssh.config",
  "vmType": "vz",
  "status": "Running",
  "arch": "aarch64",
  "cpus": 4,
  "memory": 4294967296,
  "disk": 107374182400,
  "sshLocalPort": 60022,
  "sshAddress": "127.0.0.1",
  "config": {
    "mounts": []
  }
}]"#,
    )
    .expect("list json");
    let path_env = prepend_path(limactl_bin.parent().expect("parent"));

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", &root)
        .env("PATH", path_env)
        .env("LIMACTL_LIST_JSON", &limactl_list_json)
        .env("LIMACTL_TMUX_OUTPUT", "shell|0\nagent|0\n")
        .args(["show", session.as_str()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Session: ops-show"))
        .stdout(predicate::str::contains("Mode: repo (git-native)"))
        .stdout(predicate::str::contains("VM status: Running"))
        .stdout(predicate::str::contains("Runtime: codex"))
        .stdout(predicate::str::contains("Shell window: shell (live)"))
        .stdout(predicate::str::contains("Agent window: agent (live)"))
        .stdout(predicate::str::contains(
            "/Users/tester/.codex/auth.json -> /home/tester.guest/.codex/auth.json",
        ));
}

fn prepend_path(dir: &Path) -> String {
    let existing = std::env::var("PATH").unwrap_or_default();
    if existing.is_empty() {
        dir.display().to_string()
    } else {
        format!("{}:{existing}", dir.display())
    }
}

fn install_runtime_limactl(temp_root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let bin_dir = temp_root.join("bin");
    fs::create_dir_all(&bin_dir)?;
    let limactl = bin_dir.join("limactl");
    fs::write(
        &limactl,
        r#"#!/bin/sh
if [ "$1" = "list" ] && [ "$2" = "--json" ]; then
  cat "$LIMACTL_LIST_JSON"
  exit 0
fi
if [ "$1" = "shell" ]; then
  printf '%b' "$LIMACTL_TMUX_OUTPUT"
  exit 0
fi
echo "unexpected limactl args: $*" >&2
exit 1
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
