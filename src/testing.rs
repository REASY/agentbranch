//! Shared test fixtures. Gated by `#[cfg(test)]`; not available in release builds.

use crate::db::models::{RepoSyncMode, SessionMode};
use crate::db::sessions::InsertSession;
use crate::platform::detect::HostPlatform;
use crate::platform::host::HostContext;
use crate::platform::paths::StateRoots;
use crate::types::{GuestPath, HostPath, SessionName, Timestamp, VmName};
use tempfile::TempDir;

/// Canonical test `HostContext` rooted at `dir` with `state-root/` + `home/`
/// subdirectories.
pub(crate) fn host_context(dir: &TempDir) -> HostContext {
    let base = dir.path().join("state-root");
    HostContext {
        platform: HostPlatform::current().expect("supported platform"),
        home_dir: dir.path().join("home"),
        xdg_state_home: None,
        state_roots: StateRoots::from_base(&base),
    }
}

/// Shorthand for parsing RFC3339 timestamp literals in tests.
pub(crate) fn ts(value: &str) -> Timestamp {
    Timestamp::parse_rfc3339(value).expect("valid RFC3339 timestamp")
}

/// A repo-mode `InsertSession` populated with sensible defaults.
///
/// Callers override specific fields via struct-update syntax:
/// ```ignore
/// InsertSession {
///     host_head_oid_at_open: Some("custom".to_owned()),
///     ..test_repo_session("foo", ts("2026-04-24T00:00:00Z"))
/// }
/// ```
pub(crate) fn test_repo_session(name: &str, created_at: Timestamp) -> InsertSession {
    let session = SessionName::try_from(name).expect("session name");
    InsertSession {
        name: session.clone(),
        vm_name: VmName::for_session(&session),
        session_mode: SessionMode::Repo,
        repo_sync_mode: Some(RepoSyncMode::GitNative),
        host_context_path: Some(HostPath::new("/tmp/repo")),
        guest_workspace_path: GuestPath::new(format!("/home/lima/workspaces/{name}/repo")),
        seed_host_path: None,
        host_git_root: Some(HostPath::new("/tmp/repo")),
        host_head_oid_at_open: Some("abc123".to_owned()),
        host_head_ref_at_open: Some("refs/heads/main".to_owned()),
        host_dirty_at_open: false,
        base_ref: Some("refs/heads/main".to_owned()),
        review_branch: Some(format!("agbranch/{name}")),
        session_ref_base: Some(format!("refs/agbranch/sessions/{name}/base")),
        session_ref_head: Some(format!("refs/agbranch/sessions/{name}/head")),
        provider_kind: None,
        imported_provider_files_json: "[]".to_owned(),
        guest_tmux_socket_path: None,
        shell_window_name: Some("shell".to_owned()),
        agent_window_name: Some("agent".to_owned()),
        agent_launch_preset: None,
        created_at,
    }
}

/// A sandbox-mode `InsertSession` populated with sensible defaults.
pub(crate) fn test_sandbox_session(name: &str, created_at: Timestamp) -> InsertSession {
    let session = SessionName::try_from(name).expect("session name");
    InsertSession {
        name: session.clone(),
        vm_name: VmName::for_session(&session),
        session_mode: SessionMode::Sandbox,
        repo_sync_mode: None,
        host_context_path: None,
        guest_workspace_path: GuestPath::new(format!("/home/lima/workspaces/{name}")),
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
        agent_launch_preset: None,
        created_at,
    }
}
