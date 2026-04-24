pub(crate) mod artifacts;
pub(crate) mod git_native;

pub use artifacts::SyncBackOutcome;

use crate::cli::SyncBackArgs;
use crate::db::connect::open_catalog;
use crate::db::locks::SessionLock;
use crate::db::models::{RepoSyncMode, SessionMode};
use crate::db::sessions::find_session;
use crate::error::{AppError, ValidationError};
use crate::platform::host::HostContext;
use crate::session::orchestration::LockMetadataGuard;
use crate::sync::artifacts::emit_sync_back_output;
use crate::sync::git_native::{GitNativeSyncExecution, run_git_native_sync_back};
use crate::types::{SessionName, Timestamp};
use crate::util::process::{CommandRunner, RealCommandRunner};
use crate::util::time::utc_now;
use std::fs;

pub(crate) struct SyncBackExecution<'a> {
    pub(crate) args: SyncBackArgs,
    pub(crate) host: &'a HostContext,
    pub(crate) catalog: &'a rusqlite::Connection,
    pub(crate) session_name: &'a SessionName,
}

pub(crate) struct SyncBackHooks<'a, NowFn, EmitFn> {
    pub(crate) runner: &'a dyn CommandRunner,
    pub(crate) now_fn: NowFn,
    pub(crate) emit_outcome_fn: EmitFn,
}

pub fn run_git_native(args: SyncBackArgs) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let catalog = open_catalog(&host.state_roots.db)?;
    run_with_host_and_catalog_at_with(
        args,
        &host,
        &catalog,
        std::process::id(),
        |inner_args, inner_host, inner_catalog, session_name| {
            run_with_catalog(inner_args, inner_host, inner_catalog, session_name)
        },
    )
}

pub(crate) fn run_with_host_and_catalog_at_with<RunWithCatalogFn>(
    args: SyncBackArgs,
    host: &HostContext,
    catalog: &rusqlite::Connection,
    pid: u32,
    run_with_catalog_fn: RunWithCatalogFn,
) -> Result<(), AppError>
where
    RunWithCatalogFn: FnOnce(
        SyncBackArgs,
        &HostContext,
        &rusqlite::Connection,
        &SessionName,
    ) -> Result<(), AppError>,
{
    let session_name_raw = args.session.resolve_owned()?;
    let session_name = SessionName::try_from(session_name_raw.as_str())?;
    fs::create_dir_all(&host.state_roots.staging)?;
    fs::create_dir_all(&host.state_roots.locks)?;

    let lock_path = host
        .state_roots
        .locks
        .join(format!("{}.lock", session_name.as_str()));
    let _lock = SessionLock::acquire(&lock_path, pid, "sync-back")?;
    let lock_guard = LockMetadataGuard::acquire(catalog, &session_name, pid, "sync-back")?;

    let result = run_with_catalog_fn(args, host, catalog, &session_name);
    lock_guard.commit()?;
    result
}

pub(crate) fn run_with_catalog(
    args: SyncBackArgs,
    host: &HostContext,
    catalog: &rusqlite::Connection,
    session_name: &SessionName,
) -> Result<(), AppError> {
    let runner = RealCommandRunner;
    run_with_catalog_at_with(
        SyncBackExecution {
            args,
            host,
            catalog,
            session_name,
        },
        SyncBackHooks {
            runner: &runner,
            now_fn: utc_now,
            emit_outcome_fn: emit_sync_back_output,
        },
    )
}

pub(crate) fn run_with_catalog_at_with<NowFn, EmitFn>(
    execution: SyncBackExecution<'_>,
    hooks: SyncBackHooks<'_, NowFn, EmitFn>,
) -> Result<(), AppError>
where
    NowFn: FnMut() -> Timestamp,
    EmitFn: FnMut(String) -> Result<(), AppError>,
{
    let SyncBackExecution {
        args,
        host,
        catalog,
        session_name,
    } = execution;
    let SyncBackHooks {
        runner,
        mut now_fn,
        mut emit_outcome_fn,
    } = hooks;

    let runtime_session = find_session(catalog, session_name)?.ok_or_else(|| {
        AppError::Validation(ValidationError::SessionNotFound(session_name.to_string()))
    })?;

    if runtime_session.session_mode == SessionMode::Sandbox {
        return Err(AppError::Validation(
            ValidationError::SyncBackRequiresGitNative,
        ));
    }
    if runtime_session.repo_sync_mode != Some(RepoSyncMode::GitNative) {
        return Err(AppError::Validation(
            ValidationError::SyncBackRequiresGitNative,
        ));
    }

    run_git_native_sync_back(
        GitNativeSyncExecution {
            args,
            host,
            catalog,
            session_name,
            session: &runtime_session,
        },
        runner,
        &mut now_fn,
        &mut emit_outcome_fn,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::open_catalog;
    use crate::db::models::{LifecycleState, RepoSyncMode, SessionMode, SyncState, Timestamp};
    use crate::db::sessions::{
        InsertSession, find_session, insert_session, update_lifecycle_state_with_timestamps,
        update_sync_state,
    };
    use crate::types::{GuestPath, HostPath, SessionName, VmName};
    use std::path::Path;
    use tempfile::tempdir;

    #[derive(Debug, Clone, Copy)]
    struct SeedSession<'a> {
        name: &'a str,
        host_head: Option<&'a str>,
        created_at: Timestamp,
        lifecycle_state: LifecycleState,
        sync_state: SyncState,
    }

    #[test]
    fn sync_back_wrapper_sets_and_clears_lock_metadata_around_inner_work() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let conn = open_catalog(&host.state_roots.db).expect("catalog");
        let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("created_at");
        let session = seed_session(
            &conn,
            SeedSession {
                name: "sync-wrapper",
                host_head: None,
                created_at,
                lifecycle_state: LifecycleState::Running,
                sync_state: SyncState::Pending,
            },
            dir.path(),
            &GuestPath::new("/home/tester.guest/workspaces/sync-wrapper/repo"),
        );

        run_with_host_and_catalog_at_with(
            SyncBackArgs {
                session: crate::cli::SessionSelector::from_session(session.to_string()),
                yes: true,
                export_patch: None,
                json: false,
            },
            &host,
            &conn,
            4242,
            |args, inner_host, inner_catalog, inner_session| {
                assert_eq!(args.session.resolve().expect("session"), "sync-wrapper");
                assert_eq!(inner_session.as_str(), "sync-wrapper");
                assert!(inner_host.state_roots.staging.exists());
                assert!(inner_host.state_roots.locks.exists());

                let row = find_session(inner_catalog, inner_session)
                    .expect("find session")
                    .expect("session row");
                assert_eq!(row.lock_owner_pid, Some(4242));
                assert_eq!(row.lock_operation.as_deref(), Some("sync-back"));
                Ok(())
            },
        )
        .expect("wrapper should succeed");

        let row = find_session(&conn, &session)
            .expect("find session")
            .expect("session row");
        assert_eq!(row.lock_owner_pid, None);
        assert_eq!(row.lock_operation, None);
        assert!(host.state_roots.locks.join("sync-wrapper.lock").exists());
    }

    #[test]
    fn sync_back_wrapper_clears_lock_metadata_after_inner_error() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let conn = open_catalog(&host.state_roots.db).expect("catalog");
        let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("created_at");
        let session = seed_session(
            &conn,
            SeedSession {
                name: "sync-wrapper-error",
                host_head: None,
                created_at,
                lifecycle_state: LifecycleState::Running,
                sync_state: SyncState::Pending,
            },
            dir.path(),
            &GuestPath::new("/home/tester.guest/workspaces/sync-wrapper-error/repo"),
        );

        let err = run_with_host_and_catalog_at_with(
            SyncBackArgs {
                session: crate::cli::SessionSelector::from_session(session.to_string()),
                yes: true,
                export_patch: None,
                json: false,
            },
            &host,
            &conn,
            4243,
            |_args, _inner_host, _inner_catalog, _inner_session| {
                Err(AppError::Blocked("inner sync failed".to_owned()))
            },
        )
        .expect_err("wrapper should return the inner error");
        assert!(matches!(err, AppError::Blocked(message) if message == "inner sync failed"));

        let row = find_session(&conn, &session)
            .expect("find session")
            .expect("session row");
        assert_eq!(row.lock_owner_pid, None);
        assert_eq!(row.lock_operation, None);
    }

    use crate::testing::host_context;

    fn seed_session(
        conn: &rusqlite::Connection,
        input: SeedSession<'_>,
        repo_host_path: &Path,
        repo_guest_path: &GuestPath,
    ) -> SessionName {
        let session = SessionName::try_from(input.name).expect("session");
        insert_session(
            conn,
            &InsertSession {
                name: session.clone(),
                vm_name: VmName::for_session(&session),
                session_mode: SessionMode::Repo,
                repo_sync_mode: Some(RepoSyncMode::GitNative),
                host_context_path: Some(HostPath::new(repo_host_path)),
                guest_workspace_path: repo_guest_path.clone(),
                seed_host_path: None,
                host_git_root: Some(HostPath::new(repo_host_path)),
                host_head_oid_at_open: input.host_head.map(ToOwned::to_owned),
                host_head_ref_at_open: Some("refs/heads/main".to_owned()),
                host_dirty_at_open: false,
                base_ref: Some("refs/heads/main".to_owned()),
                review_branch: None,
                session_ref_base: None,
                session_ref_head: None,
                provider_kind: None,
                imported_provider_files_json: "[]".to_owned(),
                guest_tmux_socket_path: None,
                shell_window_name: None,
                agent_window_name: None,
                agent_launch_preset: None,
                created_at: input.created_at,
            },
        )
        .expect("insert session");
        update_lifecycle_state_with_timestamps(
            conn,
            &session,
            input.lifecycle_state,
            &input.created_at,
            Some(&input.created_at),
            None,
            None,
        )
        .expect("update lifecycle state");
        update_sync_state(conn, &session, input.sync_state, &input.created_at)
            .expect("update sync state");
        session
    }
}
