#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    Sync,
    Discard,
}

#[derive(Debug, thiserror::Error)]
pub enum CloseValidationError {
    #[error("close requires exactly one of --sync or --discard")]
    InvalidOutcomeFlags,
}

impl From<CloseValidationError> for AppError {
    fn from(err: CloseValidationError) -> Self {
        AppError::Validation(ValidationError::StepFailed {
            step: "close argument validation",
            detail: err.to_string(),
        })
    }
}

pub fn validate_close_args(
    sync: bool,
    discard: bool,
) -> Result<CloseOutcome, CloseValidationError> {
    match (sync, discard) {
        (true, false) => Ok(CloseOutcome::Sync),
        (false, true) => Ok(CloseOutcome::Discard),
        _ => Err(CloseValidationError::InvalidOutcomeFlags),
    }
}

pub fn should_sync_before_close(sync_state: SyncState) -> bool {
    !matches!(sync_state, SyncState::Clean | SyncState::Discarded)
}

pub fn close_mode_error(
    session_mode: SessionMode,
    outcome: CloseOutcome,
) -> Result<(), ValidationError> {
    if session_mode == SessionMode::Sandbox && outcome == CloseOutcome::Sync {
        return Err(ValidationError::CloseRequiresDiscardForSandbox);
    }
    Ok(())
}

pub fn render_close_json(
    session: &str,
    outcome: &str,
    destroy_result: &str,
) -> Result<String, AppError> {
    Ok(serde_json::to_string(&serde_json::json!({
        "session": session,
        "outcome": outcome,
        "destroy_result": destroy_result,
    }))
    .map_err(crate::error::observability::ObservabilityError::from)?)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CloseExecution<'a> {
    pub(crate) conn: &'a rusqlite::Connection,
    pub(crate) session_name: &'a SessionName,
    pub(crate) vm_name: &'a VmName,
    pub(crate) lifecycle_state: LifecycleState,
    pub(crate) yes: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SyncCloseExecution<'a> {
    pub(crate) close: CloseExecution<'a>,
    pub(crate) host: &'a HostContext,
    pub(crate) sync_state: SyncState,
}

use crate::cli::CloseArgs;
use crate::db::connect::open_catalog;
use crate::db::locks::SessionLock;
use crate::db::models::{EventLevel, LifecycleState, SessionMode, SyncState};
use crate::db::sessions::{
    find_session, update_lifecycle_state, update_lifecycle_state_with_timestamps, update_sync_state,
};
use crate::error::{AppError, ValidationError};
use crate::lima::instance::delete_instance;
use crate::platform::host::HostContext;
use crate::session::orchestration::LockMetadataGuard;
use crate::sync;
use crate::types::{SessionName, Timestamp, VmName};
use crate::util::process::RealCommandRunner;
use crate::util::time::utc_now;

pub fn run(args: CloseArgs) -> Result<(), AppError> {
    validate_close_args(args.sync, args.discard)
        .map_err(|err: CloseValidationError| -> AppError { err.into() })?;
    let _ = SessionName::try_from(args.session.resolve()?.to_owned().as_str())?;

    let host = HostContext::detect()?;
    run_with_host_at_with(args, &host, run_sync_close, run_discard_close, |payload| {
        println!("{payload}");
        Ok(())
    })
}

pub(crate) fn run_with_host_at_with<RunSyncCloseFn, RunDiscardCloseFn, EmitJsonFn>(
    args: CloseArgs,
    host: &HostContext,
    run_sync_close_fn: RunSyncCloseFn,
    run_discard_close_fn: RunDiscardCloseFn,
    emit_json_fn: EmitJsonFn,
) -> Result<(), AppError>
where
    RunSyncCloseFn: FnOnce(
        &HostContext,
        &rusqlite::Connection,
        &SessionName,
        &VmName,
        LifecycleState,
        SyncState,
        bool,
    ) -> Result<(), AppError>,
    RunDiscardCloseFn: FnOnce(
        &rusqlite::Connection,
        &SessionName,
        &VmName,
        LifecycleState,
        bool,
    ) -> Result<(), AppError>,
    EmitJsonFn: FnOnce(String) -> Result<(), AppError>,
{
    let outcome = validate_close_args(args.sync, args.discard)
        .map_err(|err: CloseValidationError| -> AppError { err.into() })?;
    let session_name_raw = args.session.resolve_owned()?;
    let session_name = SessionName::try_from(session_name_raw.as_str())?;
    let outcome_name = match outcome {
        CloseOutcome::Sync => "sync",
        CloseOutcome::Discard => "discard",
    };

    std::fs::create_dir_all(&host.state_roots.locks)?;
    let lock_path = host
        .state_roots
        .locks
        .join(format!("{}.lock", session_name.as_str()));
    let _lock = SessionLock::acquire(&lock_path, std::process::id(), "close")?;

    let conn = open_catalog(&host.state_roots.db)?;
    let session = find_session(&conn, &session_name)?.ok_or_else(|| {
        AppError::Validation(ValidationError::SessionNotFound(session_name_raw.clone()))
    })?;
    close_mode_error(session.session_mode, outcome).map_err(AppError::Validation)?;
    let lock_guard = LockMetadataGuard::acquire(&conn, &session_name, std::process::id(), "close")?;

    let result = match outcome {
        CloseOutcome::Sync => run_sync_close_fn(
            host,
            &conn,
            &session_name,
            &session.vm_name,
            session.lifecycle_state,
            session.sync_state,
            args.yes,
        ),
        CloseOutcome::Discard => run_discard_close_fn(
            &conn,
            &session_name,
            &session.vm_name,
            session.lifecycle_state,
            args.yes,
        ),
    };

    lock_guard.commit()?;
    result?;

    if args.json {
        emit_json_fn(render_close_json(
            session_name.as_str(),
            outcome_name,
            "destroyed",
        )?)?;
    }

    Ok(())
}

fn run_sync_close(
    host: &HostContext,
    conn: &rusqlite::Connection,
    session_name: &SessionName,
    vm_name: &VmName,
    lifecycle_state: LifecycleState,
    sync_state: SyncState,
    yes: bool,
) -> Result<(), AppError> {
    run_sync_close_at_with(
        SyncCloseExecution {
            close: CloseExecution {
                conn,
                session_name,
                vm_name,
                lifecycle_state,
                yes,
            },
            host,
            sync_state,
        },
        sync::run_with_catalog,
        delete_vm,
        utc_now,
    )
}

pub(crate) fn run_sync_close_at_with<SyncBackFn, DeleteVmFn, NowFn>(
    execution: SyncCloseExecution<'_>,
    mut sync_back_fn: SyncBackFn,
    delete_vm_fn: DeleteVmFn,
    now_fn: NowFn,
) -> Result<(), AppError>
where
    SyncBackFn: FnMut(
        crate::cli::SyncBackArgs,
        &HostContext,
        &rusqlite::Connection,
        &SessionName,
    ) -> Result<(), AppError>,
    DeleteVmFn: FnOnce(&VmName) -> Result<(), AppError>,
    NowFn: FnOnce() -> Timestamp,
{
    if should_sync_before_close(execution.sync_state) {
        sync_back_fn(
            crate::cli::SyncBackArgs {
                session: crate::cli::SessionSelector::from_session(
                    execution.close.session_name.to_string(),
                ),
                yes: true,
                export_patch: None,
                json: false,
            },
            execution.host,
            execution.close.conn,
            execution.close.session_name,
        )?;
    }
    finish_close_at_with(
        execution.close,
        SyncState::Clean,
        "session closed after sync-back",
        delete_vm_fn,
        now_fn,
    )
}

fn run_discard_close(
    conn: &rusqlite::Connection,
    session_name: &SessionName,
    vm_name: &VmName,
    lifecycle_state: LifecycleState,
    yes: bool,
) -> Result<(), AppError> {
    run_discard_close_at_with(
        CloseExecution {
            conn,
            session_name,
            vm_name,
            lifecycle_state,
            yes,
        },
        delete_vm,
        utc_now,
    )
}

pub(crate) fn run_discard_close_at_with<DeleteVmFn, NowFn>(
    execution: CloseExecution<'_>,
    delete_vm_fn: DeleteVmFn,
    now_fn: NowFn,
) -> Result<(), AppError>
where
    DeleteVmFn: FnOnce(&VmName) -> Result<(), AppError>,
    NowFn: FnOnce() -> Timestamp,
{
    finish_close_at_with(
        execution,
        SyncState::Discarded,
        "session closed with discard outcome",
        delete_vm_fn,
        now_fn,
    )
}

fn finish_close_at_with<DeleteVmFn, NowFn>(
    execution: CloseExecution<'_>,
    sync_state: SyncState,
    message: &str,
    delete_vm_fn: DeleteVmFn,
    now_fn: NowFn,
) -> Result<(), AppError>
where
    DeleteVmFn: FnOnce(&VmName) -> Result<(), AppError>,
    NowFn: FnOnce() -> Timestamp,
{
    if !execution.yes {
        return Err(AppError::Blocked(
            "close requires confirmation; rerun with --yes".to_owned(),
        ));
    }

    let destroying_at = utc_now();
    update_lifecycle_state(
        execution.conn,
        execution.session_name,
        LifecycleState::Destroying,
        &destroying_at,
    )?;

    delete_vm_fn(execution.vm_name)?;

    let completed_at = now_fn();
    let stopped_at = if should_record_stop_timestamp(execution.lifecycle_state) {
        Some(&completed_at)
    } else {
        None
    };
    update_lifecycle_state_with_timestamps(
        execution.conn,
        execution.session_name,
        LifecycleState::Closed,
        &completed_at,
        None,
        stopped_at,
        Some(&completed_at),
    )?;
    update_sync_state(
        execution.conn,
        execution.session_name,
        sync_state,
        &completed_at,
    )?;

    crate::db::events::append_event(
        execution.conn,
        execution.session_name,
        EventLevel::Info,
        "session.closed",
        message,
        completed_at,
    )?;
    Ok(())
}

fn should_record_stop_timestamp(lifecycle_state: LifecycleState) -> bool {
    !matches!(
        lifecycle_state,
        LifecycleState::Stopped | LifecycleState::Closed
    )
}

fn delete_vm(vm_name: &VmName) -> Result<(), AppError> {
    delete_instance(&RealCommandRunner, vm_name).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::open_catalog;
    use crate::db::events::list_events;
    use crate::db::models::{RepoSyncMode, SessionMode};
    use crate::db::sessions::{InsertSession, find_session, insert_session};
    use crate::platform::detect::HostPlatform;
    use crate::platform::paths::StateRoots;
    use crate::types::{GuestPath, HostPath};
    use std::cell::{Cell, RefCell};
    use tempfile::{TempDir, tempdir};

    #[derive(Debug)]
    struct TestHost {
        _dir: TempDir,
        host: HostContext,
    }

    #[derive(Debug, Clone, Copy)]
    struct SeedSession<'a> {
        name: &'a str,
        lifecycle_state: LifecycleState,
        sync_state: SyncState,
        created_at: Timestamp,
        last_started_at: Option<Timestamp>,
        last_stopped_at: Option<Timestamp>,
        closed_at: Option<Timestamp>,
    }

    #[test]
    fn sync_close_returns_blocked_when_sync_back_blocks() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let db = dir.path().join("state.db");
        let conn = open_catalog(&db).expect("catalog");
        let created_at = ts("2026-04-15T00:00:00Z");
        let session = seed_session(
            &conn,
            SeedSession {
                name: "feat-blocked",
                lifecycle_state: LifecycleState::Running,
                sync_state: SyncState::Pending,
                created_at,
                last_started_at: Some(created_at),
                last_stopped_at: None,
                closed_at: None,
            },
        );

        let sync_calls = Cell::new(0);
        let delete_calls = Cell::new(0);
        let expected_session = session.to_string();
        let vm_name = VmName::for_session(&session);

        let result = run_sync_close_at_with(
            SyncCloseExecution {
                close: CloseExecution {
                    conn: &conn,
                    session_name: &session,
                    vm_name: &vm_name,
                    lifecycle_state: LifecycleState::Running,
                    yes: true,
                },
                host: &host,
                sync_state: SyncState::Pending,
            },
            |args, _, _, called_session: &SessionName| {
                sync_calls.set(sync_calls.get() + 1);
                assert_eq!(args.session.resolve().expect("session"), expected_session);
                assert!(args.yes);
                assert!(args.export_patch.is_none());
                assert!(!args.json);
                assert_eq!(called_session.as_str(), "feat-blocked");
                Err(AppError::Blocked("sync blocked for drift".to_owned()))
            },
            |_vm_name| {
                delete_calls.set(delete_calls.get() + 1);
                Ok(())
            },
            || -> Timestamp { panic!("clock should not be read when sync-back blocks") },
        );

        assert!(matches!(
            result,
            Err(AppError::Blocked(message)) if message == "sync blocked for drift"
        ));
        assert_eq!(sync_calls.get(), 1);
        assert_eq!(delete_calls.get(), 0);

        let session_row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(session_row.lifecycle_state, LifecycleState::Running);
        assert_eq!(session_row.sync_state, SyncState::Pending);
        assert_eq!(session_row.last_stopped_at, None);
        assert_eq!(session_row.closed_at, None);

        let events = list_events(&conn, Some(&session)).expect("events");
        assert!(events.is_empty());
    }

    #[test]
    fn clean_sync_close_skips_resync_and_records_close_effects() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let db = dir.path().join("state.db");
        let conn = open_catalog(&db).expect("catalog");
        let created_at = ts("2026-04-15T00:00:00Z");
        let close_at = ts("2026-04-15T01:00:00Z");
        let session = seed_session(
            &conn,
            SeedSession {
                name: "feat-clean",
                lifecycle_state: LifecycleState::Running,
                sync_state: SyncState::Clean,
                created_at,
                last_started_at: Some(created_at),
                last_stopped_at: None,
                closed_at: None,
            },
        );

        let sync_calls = Cell::new(0);
        let delete_calls = Cell::new(0);
        let vm_name = VmName::for_session(&session);

        run_sync_close_at_with(
            SyncCloseExecution {
                close: CloseExecution {
                    conn: &conn,
                    session_name: &session,
                    vm_name: &vm_name,
                    lifecycle_state: LifecycleState::Running,
                    yes: true,
                },
                host: &host,
                sync_state: SyncState::Clean,
            },
            |_args, _, _, _| {
                sync_calls.set(sync_calls.get() + 1);
                Ok(())
            },
            |deleted_vm: &VmName| {
                delete_calls.set(delete_calls.get() + 1);
                assert_eq!(deleted_vm.as_str(), vm_name.as_str());
                Ok(())
            },
            || close_at,
        )
        .expect("clean close should succeed");

        assert_eq!(sync_calls.get(), 0);
        assert_eq!(delete_calls.get(), 1);

        let session_row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(session_row.lifecycle_state, LifecycleState::Closed);
        assert_eq!(session_row.sync_state, SyncState::Clean);
        assert_eq!(session_row.last_stopped_at, Some(close_at));
        assert_eq!(session_row.closed_at, Some(close_at));

        let events = list_events(&conn, Some(&session)).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].level, EventLevel::Info);
        assert_eq!(events[0].kind, "session.closed");
        assert_eq!(events[0].message, "session closed after sync-back");
        assert_eq!(events[0].at, close_at);
    }

    #[test]
    fn sync_close_records_completion_timestamp_after_sync_back_and_delete() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let db = dir.path().join("state.db");
        let conn = open_catalog(&db).expect("catalog");
        let created_at = ts("2026-04-15T00:00:00Z");
        let completed_at = ts("2026-04-15T01:10:00Z");
        let session = seed_session(
            &conn,
            SeedSession {
                name: "feat-sync-close-complete",
                lifecycle_state: LifecycleState::Running,
                sync_state: SyncState::Pending,
                created_at,
                last_started_at: Some(created_at),
                last_stopped_at: None,
                closed_at: None,
            },
        );

        let order = RefCell::new(Vec::new());
        let vm_name = VmName::for_session(&session);

        run_sync_close_at_with(
            SyncCloseExecution {
                close: CloseExecution {
                    conn: &conn,
                    session_name: &session,
                    vm_name: &vm_name,
                    lifecycle_state: LifecycleState::Running,
                    yes: true,
                },
                host: &host,
                sync_state: SyncState::Pending,
            },
            |_args, _, _, called_session: &SessionName| {
                order.borrow_mut().push("sync");
                assert_eq!(called_session.as_str(), "feat-sync-close-complete");
                Ok(())
            },
            |deleted_vm: &VmName| {
                order.borrow_mut().push("delete");
                assert_eq!(deleted_vm.as_str(), vm_name.as_str());
                Ok(())
            },
            || {
                assert_eq!(&*order.borrow(), &["sync", "delete"]);
                completed_at
            },
        )
        .expect("sync-close should succeed");

        let session_row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(session_row.lifecycle_state, LifecycleState::Closed);
        assert_eq!(session_row.sync_state, SyncState::Clean);
        assert_eq!(session_row.last_stopped_at, Some(completed_at));
        assert_eq!(session_row.closed_at, Some(completed_at));

        let events = list_events(&conn, Some(&session)).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "session.closed");
        assert_eq!(events[0].message, "session closed after sync-back");
        assert_eq!(events[0].at, completed_at);
    }

    #[test]
    fn discarded_sync_close_skips_resync_and_records_clean_close_effects() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let db = dir.path().join("state.db");
        let conn = open_catalog(&db).expect("catalog");
        let created_at = ts("2026-04-15T00:00:00Z");
        let close_at = ts("2026-04-15T01:00:00Z");
        let session = seed_session(
            &conn,
            SeedSession {
                name: "feat-discarded-sync-close",
                lifecycle_state: LifecycleState::Running,
                sync_state: SyncState::Discarded,
                created_at,
                last_started_at: Some(created_at),
                last_stopped_at: None,
                closed_at: None,
            },
        );

        let sync_calls = Cell::new(0);
        let delete_calls = Cell::new(0);
        let vm_name = VmName::for_session(&session);

        run_sync_close_at_with(
            SyncCloseExecution {
                close: CloseExecution {
                    conn: &conn,
                    session_name: &session,
                    vm_name: &vm_name,
                    lifecycle_state: LifecycleState::Running,
                    yes: true,
                },
                host: &host,
                sync_state: SyncState::Discarded,
            },
            |_args, _, _, _| {
                sync_calls.set(sync_calls.get() + 1);
                Ok(())
            },
            |deleted_vm: &VmName| {
                delete_calls.set(delete_calls.get() + 1);
                assert_eq!(deleted_vm.as_str(), vm_name.as_str());
                Ok(())
            },
            || close_at,
        )
        .expect("discarded sync-close should succeed");

        assert_eq!(sync_calls.get(), 0);
        assert_eq!(delete_calls.get(), 1);

        let session_row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(session_row.lifecycle_state, LifecycleState::Closed);
        assert_eq!(session_row.sync_state, SyncState::Clean);
        assert_eq!(session_row.last_stopped_at, Some(close_at));
        assert_eq!(session_row.closed_at, Some(close_at));

        let events = list_events(&conn, Some(&session)).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].level, EventLevel::Info);
        assert_eq!(events[0].kind, "session.closed");
        assert_eq!(events[0].message, "session closed after sync-back");
        assert_eq!(events[0].at, close_at);
    }

    #[test]
    fn discard_close_marks_session_discarded_without_overwriting_stop_time() {
        let dir = tempdir().expect("tempdir");
        let db = dir.path().join("state.db");
        let conn = open_catalog(&db).expect("catalog");
        let created_at = ts("2026-04-15T00:00:00Z");
        let stopped_at = ts("2026-04-15T00:45:00Z");
        let close_at = ts("2026-04-15T01:00:00Z");
        let session = seed_session(
            &conn,
            SeedSession {
                name: "feat-discard",
                lifecycle_state: LifecycleState::Stopped,
                sync_state: SyncState::Pending,
                created_at,
                last_started_at: Some(created_at),
                last_stopped_at: Some(stopped_at),
                closed_at: None,
            },
        );

        let delete_calls = Cell::new(0);
        let vm_name = VmName::for_session(&session);

        run_discard_close_at_with(
            CloseExecution {
                conn: &conn,
                session_name: &session,
                vm_name: &vm_name,
                lifecycle_state: LifecycleState::Stopped,
                yes: true,
            },
            |deleted_vm: &VmName| {
                delete_calls.set(delete_calls.get() + 1);
                assert_eq!(deleted_vm.as_str(), vm_name.as_str());
                Ok(())
            },
            || close_at,
        )
        .expect("discard close should succeed");

        assert_eq!(delete_calls.get(), 1);

        let session_row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(session_row.lifecycle_state, LifecycleState::Closed);
        assert_eq!(session_row.sync_state, SyncState::Discarded);
        assert_eq!(session_row.last_stopped_at, Some(stopped_at));
        assert_eq!(session_row.closed_at, Some(close_at));

        let events = list_events(&conn, Some(&session)).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "session.closed");
        assert_eq!(events[0].message, "session closed with discard outcome");
        assert_eq!(events[0].at, close_at);
    }

    #[test]
    fn close_wrapper_sync_json_emits_payload_and_clears_lock_metadata() {
        let test_host = setup_host();
        let session = seed_session(
            &open_catalog(&test_host.host.state_roots.db).expect("catalog"),
            SeedSession {
                name: "wrapper-sync-json",
                lifecycle_state: LifecycleState::Running,
                sync_state: SyncState::Pending,
                created_at: ts("2026-04-15T00:00:00Z"),
                last_started_at: Some(ts("2026-04-15T00:00:00Z")),
                last_stopped_at: None,
                closed_at: None,
            },
        );
        let emitted = RefCell::new(None);
        let observed_lock = Cell::new(false);
        let lock_path = lock_path(&test_host.host, &session);

        run_with_host_at_with(
            CloseArgs {
                session: crate::cli::SessionSelector::from_session(session.to_string()),
                sync: true,
                discard: false,
                yes: true,
                json: true,
            },
            &test_host.host,
            |_host, conn, called_session, vm_name, lifecycle_state, sync_state, yes| {
                assert_eq!(called_session.as_str(), session.as_str());
                assert_eq!(vm_name.as_str(), VmName::for_session(&session).as_str());
                assert_eq!(lifecycle_state, LifecycleState::Running);
                assert_eq!(sync_state, SyncState::Pending);
                assert!(yes);
                assert!(lock_path.exists());
                let row = find_session(conn, &session)
                    .expect("lookup session")
                    .expect("session row");
                assert_eq!(row.lock_owner_pid, Some(std::process::id() as i64));
                assert_eq!(row.lock_operation.as_deref(), Some("close"));
                observed_lock.set(true);
                Ok(())
            },
            |_, _, _, _, _| panic!("discard path should not run"),
            |payload| {
                emitted.replace(Some(payload));
                Ok(())
            },
        )
        .expect("sync close wrapper should succeed");

        assert!(
            observed_lock.get(),
            "sync wrapper should observe lock metadata"
        );
        let conn = open_catalog(&test_host.host.state_roots.db).expect("catalog");
        let row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(row.lock_owner_pid, None);
        assert_eq!(row.lock_operation, None);

        let value: serde_json::Value =
            serde_json::from_str(emitted.borrow().as_deref().expect("json payload"))
                .expect("valid close json");
        assert_eq!(value["session"], session.as_str());
        assert_eq!(value["outcome"], "sync");
        assert_eq!(value["destroy_result"], "destroyed");
    }

    #[test]
    fn close_wrapper_missing_session_returns_validation_error() {
        let test_host = setup_host();
        let result = run_with_host_at_with(
            CloseArgs {
                session: crate::cli::SessionSelector::from_session("missing-session"),
                sync: true,
                discard: false,
                yes: true,
                json: false,
            },
            &test_host.host,
            |_host, _, _, _, _, _, _| panic!("sync close should not run"),
            |_, _, _, _, _| panic!("discard close should not run"),
            |_payload| panic!("json output should not run"),
        );

        assert!(matches!(
            result,
            Err(AppError::Validation(ValidationError::SessionNotFound(name)))
                if name == "missing-session"
        ));
        assert!(test_host.host.state_roots.locks.exists());
    }

    #[test]
    fn close_wrapper_clears_lock_metadata_when_sync_close_errors() {
        let test_host = setup_host();
        let session = seed_session(
            &open_catalog(&test_host.host.state_roots.db).expect("catalog"),
            SeedSession {
                name: "wrapper-sync-blocked",
                lifecycle_state: LifecycleState::Running,
                sync_state: SyncState::Pending,
                created_at: ts("2026-04-15T00:00:00Z"),
                last_started_at: Some(ts("2026-04-15T00:00:00Z")),
                last_stopped_at: None,
                closed_at: None,
            },
        );

        let result = run_with_host_at_with(
            CloseArgs {
                session: crate::cli::SessionSelector::from_session(session.to_string()),
                sync: true,
                discard: false,
                yes: true,
                json: false,
            },
            &test_host.host,
            |_host, conn, _, _, _, _, _| {
                let row = find_session(conn, &session)
                    .expect("lookup session")
                    .expect("session row");
                assert_eq!(row.lock_owner_pid, Some(std::process::id() as i64));
                assert_eq!(row.lock_operation.as_deref(), Some("close"));
                Err(AppError::Blocked("sync blocked for drift".to_owned()))
            },
            |_, _, _, _, _| panic!("discard close should not run"),
            |_payload| panic!("json output should not run"),
        );

        assert!(matches!(
            result,
            Err(AppError::Blocked(message)) if message == "sync blocked for drift"
        ));
        let conn = open_catalog(&test_host.host.state_roots.db).expect("catalog");
        let row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(row.lock_owner_pid, None);
        assert_eq!(row.lock_operation, None);
    }

    #[test]
    fn close_wrapper_discard_requires_confirmation_and_clears_lock_metadata() {
        let test_host = setup_host();
        let session = seed_session(
            &open_catalog(&test_host.host.state_roots.db).expect("catalog"),
            SeedSession {
                name: "wrapper-discard-confirm",
                lifecycle_state: LifecycleState::Running,
                sync_state: SyncState::Pending,
                created_at: ts("2026-04-15T00:00:00Z"),
                last_started_at: Some(ts("2026-04-15T00:00:00Z")),
                last_stopped_at: None,
                closed_at: None,
            },
        );

        let result = run_with_host_at_with(
            CloseArgs {
                session: crate::cli::SessionSelector::from_session(session.to_string()),
                sync: false,
                discard: true,
                yes: false,
                json: false,
            },
            &test_host.host,
            |_host, _, _, _, _, _, _| panic!("sync close should not run"),
            |conn, called_session, vm_name, lifecycle_state, yes| {
                let row = find_session(conn, &session)
                    .expect("lookup session")
                    .expect("session row");
                assert_eq!(called_session.as_str(), session.as_str());
                assert_eq!(vm_name.as_str(), VmName::for_session(&session).as_str());
                assert_eq!(lifecycle_state, LifecycleState::Running);
                assert!(!yes);
                assert_eq!(row.lock_owner_pid, Some(std::process::id() as i64));
                assert_eq!(row.lock_operation.as_deref(), Some("close"));
                run_discard_close_at_with(
                    CloseExecution {
                        conn,
                        session_name: called_session,
                        vm_name,
                        lifecycle_state,
                        yes,
                    },
                    |_vm_name| panic!("discard close should not delete without confirmation"),
                    || panic!("blocked confirmation should not read completion clock"),
                )
            },
            |_payload| panic!("json output should not run"),
        );

        assert!(matches!(
            result,
            Err(AppError::Blocked(message))
                if message == "close requires confirmation; rerun with --yes"
        ));

        let conn = open_catalog(&test_host.host.state_roots.db).expect("catalog");
        let row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(row.lock_owner_pid, None);
        assert_eq!(row.lock_operation, None);
        assert_eq!(row.lifecycle_state, LifecycleState::Running);
        assert_eq!(row.closed_at, None);
    }

    fn setup_host() -> TestHost {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("state-root");
        TestHost {
            _dir: dir,
            host: HostContext {
                platform: HostPlatform::current().expect("supported platform"),
                home_dir: base.join("home"),
                xdg_state_home: None,
                state_roots: StateRoots::from_base(&base),
            },
        }
    }

    use crate::testing::host_context;

    fn lock_path(host: &HostContext, session: &SessionName) -> std::path::PathBuf {
        host.state_roots
            .locks
            .join(format!("{}.lock", session.as_str()))
    }

    fn seed_session(conn: &rusqlite::Connection, input: SeedSession<'_>) -> SessionName {
        let session = SessionName::try_from(input.name).expect("session");
        insert_session(
            conn,
            &InsertSession {
                name: session.clone(),
                vm_name: VmName::for_session(&session),
                session_mode: SessionMode::Repo,
                repo_sync_mode: Some(RepoSyncMode::GitNative),
                host_context_path: Some(HostPath::new("/tmp/host")),
                guest_workspace_path: GuestPath::new("/tmp/guest"),
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
                created_at: input.created_at,
            },
        )
        .expect("insert session");
        update_lifecycle_state_with_timestamps(
            conn,
            &session,
            input.lifecycle_state,
            &input.created_at,
            input.last_started_at.as_ref(),
            input.last_stopped_at.as_ref(),
            input.closed_at.as_ref(),
        )
        .expect("update lifecycle state");
        update_sync_state(conn, &session, input.sync_state, &input.created_at)
            .expect("update sync state");
        session
    }

    fn ts(value: &str) -> Timestamp {
        Timestamp::parse_rfc3339(value).expect("timestamp")
    }

    #[test]
    fn close_requires_exactly_one_outcome() {
        assert!(validate_close_args(false, false).is_err());
        assert!(validate_close_args(true, true).is_err());
        assert_eq!(
            validate_close_args(true, false).expect("sync outcome"),
            CloseOutcome::Sync
        );
        assert_eq!(
            validate_close_args(false, true).expect("discard outcome"),
            CloseOutcome::Discard
        );
    }

    #[test]
    fn close_sync_skips_resync_for_clean_sessions() {
        assert!(!should_sync_before_close(SyncState::Clean));
        assert!(should_sync_before_close(SyncState::Pending));
        assert!(should_sync_before_close(SyncState::Blocked));
    }

    #[test]
    fn sandbox_sessions_reject_sync_close() {
        assert_eq!(
            close_mode_error(SessionMode::Sandbox, CloseOutcome::Sync)
                .expect_err("sandbox sync should be rejected")
                .to_string(),
            "sandbox sessions must export artifacts and then close with --discard"
        );
        assert!(close_mode_error(SessionMode::Sandbox, CloseOutcome::Discard).is_ok());
        assert!(close_mode_error(SessionMode::Repo, CloseOutcome::Sync).is_ok());
    }

    #[test]
    fn close_json_contains_session_outcome_and_destroy_result() {
        let rendered = render_close_json("agbranch-smoke-discard", "discard", "destroyed")
            .expect("close json");

        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid json");
        assert_eq!(value["session"], "agbranch-smoke-discard");
        assert_eq!(value["outcome"], "discard");
        assert_eq!(value["destroy_result"], "destroyed");
    }
}
