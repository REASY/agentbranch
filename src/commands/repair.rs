use crate::cli::SessionArgs;
use crate::db::connect::open_catalog;
use crate::db::events::append_event;
use crate::db::locks::SessionLock;
use crate::db::models::{EventLevel, LifecycleState};
use crate::db::sessions::{find_session, update_lifecycle_state_with_timestamps};
use crate::error::{AppError, ValidationError};
use crate::lima::instance::{delete_instance, start_instance};
use crate::platform::host::HostContext;
use crate::session::orchestration::LockMetadataGuard;
use crate::session::reconcile::{RepairAction, repair_action_for_state};
use crate::types::{SessionName, Timestamp, VmName};
use crate::util::process::RealCommandRunner;
use crate::util::time::utc_now;

#[derive(Debug, Clone, Copy)]
pub(crate) struct RepairExecution<'a> {
    pub(crate) conn: &'a rusqlite::Connection,
    pub(crate) session_name: &'a SessionName,
    pub(crate) vm_name: &'a VmName,
    pub(crate) action: RepairAction,
    pub(crate) lifecycle_state: LifecycleState,
}

pub fn run(args: SessionArgs) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    run_with_host_at_with(args, &host, start_vm, delete_vm, utc_now, |payload| {
        println!("{payload}");
        Ok(())
    })
}

pub(crate) fn run_with_host_at_with<StartVmFn, DeleteVmFn, NowFn, EmitObservationFn>(
    args: SessionArgs,
    host: &HostContext,
    start_vm_fn: StartVmFn,
    delete_vm_fn: DeleteVmFn,
    now_fn: NowFn,
    emit_observation_fn: EmitObservationFn,
) -> Result<(), AppError>
where
    StartVmFn: FnOnce(&VmName) -> Result<(), AppError>,
    DeleteVmFn: FnOnce(&VmName) -> Result<(), AppError>,
    NowFn: FnOnce() -> Timestamp,
    EmitObservationFn: FnOnce(serde_json::Value) -> Result<(), AppError>,
{
    let session_name_raw = args.session.resolve_owned()?;
    let session_name = SessionName::try_from(session_name_raw.as_str())?;
    std::fs::create_dir_all(&host.state_roots.locks)?;
    let lock_path = host.state_roots.locks.join(format!("{session_name}.lock"));
    let _lock = SessionLock::acquire(&lock_path, std::process::id(), "repair")?;

    let conn = open_catalog(&host.state_roots.db)?;
    let lock_guard =
        LockMetadataGuard::acquire(&conn, &session_name, std::process::id(), "repair")?;

    let session = find_session(&conn, &session_name)?.ok_or_else(|| {
        AppError::Validation(ValidationError::SessionNotFound(session_name_raw.clone()))
    })?;
    let action = repair_action_for_state(session.lifecycle_state);

    let result = if args.json {
        emit_observation_fn(serde_json::json!({
            "session": session_name,
            "lifecycle_state": session.lifecycle_state.as_str(),
            "action": action.as_str(),
        }))
    } else {
        apply_repair_action_at_with(
            RepairExecution {
                conn: &conn,
                session_name: &session_name,
                vm_name: &session.vm_name,
                action,
                lifecycle_state: session.lifecycle_state,
            },
            start_vm_fn,
            delete_vm_fn,
            now_fn,
        )
    };

    lock_guard.commit()?;
    result
}

pub(crate) fn apply_repair_action_at_with<StartVmFn, DeleteVmFn, NowFn>(
    execution: RepairExecution<'_>,
    start_vm_fn: StartVmFn,
    delete_vm_fn: DeleteVmFn,
    now_fn: NowFn,
) -> Result<(), AppError>
where
    StartVmFn: FnOnce(&VmName) -> Result<(), AppError>,
    DeleteVmFn: FnOnce(&VmName) -> Result<(), AppError>,
    NowFn: FnOnce() -> Timestamp,
{
    let mut now_fn = Some(now_fn);

    match execution.action {
        RepairAction::Noop => Ok(()),
        RepairAction::Restart => {
            start_vm_fn(execution.vm_name)?;
            let completed_at = now_fn
                .take()
                .expect("repair completion clock should only be read once")(
            );
            update_lifecycle_state_with_timestamps(
                execution.conn,
                execution.session_name,
                LifecycleState::Running,
                &completed_at,
                Some(&completed_at),
                None,
                None,
            )?;
            append_event(
                execution.conn,
                execution.session_name,
                EventLevel::Info,
                "session.repaired",
                "repair restarted the session VM",
                completed_at,
            )?;
            Ok(())
        }
        RepairAction::FinishDestroy => {
            delete_vm_fn(execution.vm_name)?;
            let completed_at = now_fn
                .take()
                .expect("repair completion clock should only be read once")(
            );
            let stopped_at = if matches!(
                execution.lifecycle_state,
                LifecycleState::Stopped | LifecycleState::Closed
            ) {
                None
            } else {
                Some(&completed_at)
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
            append_event(
                execution.conn,
                execution.session_name,
                EventLevel::Info,
                "session.repaired",
                "repair finished the destroy transition",
                completed_at,
            )?;
            Ok(())
        }
        _ => Err(AppError::Blocked(format!(
            "repair requires manual action: {}",
            execution.action.as_str()
        ))),
    }
}

fn start_vm(vm_name: &VmName) -> Result<(), AppError> {
    start_instance(&RealCommandRunner, vm_name).map_err(Into::into)
}

fn delete_vm(vm_name: &VmName) -> Result<(), AppError> {
    delete_instance(&RealCommandRunner, vm_name).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::open_catalog;
    use crate::db::events::list_events;
    use crate::db::models::SyncState;
    use crate::db::sessions::{InsertSession, insert_session, update_sync_state};
    use crate::platform::detect::HostPlatform;
    use crate::platform::paths::StateRoots;
    use crate::types::{GuestPath, HostPath};
    use std::cell::{Cell, RefCell};
    use std::fs;
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
        created_at: Timestamp,
        last_started_at: Option<Timestamp>,
        last_stopped_at: Option<Timestamp>,
        closed_at: Option<Timestamp>,
    }

    #[test]
    fn repair_json_reports_action_without_mutating_session_and_clears_lock_metadata() {
        let test_host = setup_host();
        let session = seed_session(
            &test_host.host,
            SeedSession {
                name: "repair-json",
                lifecycle_state: LifecycleState::Starting,
                created_at: ts("2026-04-15T00:00:00Z"),
                last_started_at: None,
                last_stopped_at: None,
                closed_at: None,
            },
        );
        let lock_path = lock_path(&test_host.host, &session);
        let emitted = RefCell::new(None);
        let observed_lock = Cell::new(false);

        run_with_host_at_with(
            SessionArgs {
                session: crate::cli::SessionSelector::from_session(session.to_string()),
                json: true,
            },
            &test_host.host,
            |_vm| panic!("json observation should not start a VM"),
            |_vm| panic!("json observation should not delete a VM"),
            || panic!("json observation should not consult the clock"),
            |payload| {
                let conn = open_catalog(&test_host.host.state_roots.db).expect("catalog");
                let row = find_session(&conn, &session)
                    .expect("lookup session")
                    .expect("session row");
                assert_eq!(row.lock_owner_pid, Some(std::process::id() as i64));
                assert_eq!(row.lock_operation.as_deref(), Some("repair"));
                observed_lock.set(true);
                emitted.replace(Some(payload));
                Ok(())
            },
        )
        .expect("json repair should succeed");

        assert!(
            observed_lock.get(),
            "json path should observe lock metadata"
        );
        let payload = emitted.into_inner().expect("json payload");
        assert_eq!(payload["session"], session.as_str());
        assert_eq!(payload["lifecycle_state"], "starting");
        assert_eq!(payload["action"], "restart");

        let conn = open_catalog(&test_host.host.state_roots.db).expect("catalog");
        let row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(row.lifecycle_state, LifecycleState::Starting);
        assert_eq!(row.last_started_at, None);
        assert_eq!(row.lock_owner_pid, None);
        assert_eq!(row.lock_operation, None);
        assert!(
            list_events(&conn, Some(&session))
                .expect("events")
                .is_empty()
        );

        let lock_contents = fs::read_to_string(&lock_path).expect("lock file");
        assert!(lock_contents.contains(&format!("pid={}", std::process::id())));
        assert!(lock_contents.contains("operation=repair"));
    }

    #[test]
    fn repair_missing_session_returns_validation_error_after_acquiring_lock() {
        let test_host = setup_host();
        let session = SessionName::try_from("missing-repair").expect("session");
        let lock_path = lock_path(&test_host.host, &session);

        let err = run_with_host_at_with(
            SessionArgs {
                session: crate::cli::SessionSelector::from_session(session.to_string()),
                json: false,
            },
            &test_host.host,
            |_vm| panic!("missing session should not start a VM"),
            |_vm| panic!("missing session should not delete a VM"),
            || panic!("missing session should not consult the clock"),
            |_payload| panic!("missing session should not emit json"),
        )
        .expect_err("missing session should fail");

        assert!(matches!(
            err,
            AppError::Validation(ValidationError::SessionNotFound(name)) if name == "missing-repair"
        ));
        let lock_contents = fs::read_to_string(&lock_path).expect("lock file");
        assert!(lock_contents.contains(&format!("pid={}", std::process::id())));
        assert!(lock_contents.contains("operation=repair"));
    }

    #[test]
    fn repair_noop_leaves_session_unchanged() {
        let test_host = setup_host();
        let created_at = ts("2026-04-15T00:00:00Z");
        let last_started_at = ts("2026-04-15T00:10:00Z");
        let session = seed_session(
            &test_host.host,
            SeedSession {
                name: "repair-noop",
                lifecycle_state: LifecycleState::Running,
                created_at,
                last_started_at: Some(last_started_at),
                last_stopped_at: None,
                closed_at: None,
            },
        );

        run_with_host_at_with(
            SessionArgs {
                session: crate::cli::SessionSelector::from_session(session.to_string()),
                json: false,
            },
            &test_host.host,
            |_vm| panic!("noop repair should not start a VM"),
            |_vm| panic!("noop repair should not delete a VM"),
            || panic!("noop repair should not consult the clock"),
            |_payload| panic!("noop repair should not emit json"),
        )
        .expect("noop repair should succeed");

        let conn = open_catalog(&test_host.host.state_roots.db).expect("catalog");
        let row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(row.lifecycle_state, LifecycleState::Running);
        assert_eq!(row.last_started_at, Some(last_started_at));
        assert_eq!(row.last_stopped_at, None);
        assert_eq!(row.closed_at, None);
        assert_eq!(row.lock_owner_pid, None);
        assert_eq!(row.lock_operation, None);
        assert!(
            list_events(&conn, Some(&session))
                .expect("events")
                .is_empty()
        );
    }

    #[test]
    fn repair_restart_updates_lifecycle_event_and_lock_metadata() {
        let test_host = setup_host();
        let created_at = ts("2026-04-15T00:00:00Z");
        let completed_at = ts("2026-04-15T01:15:00Z");
        let session = seed_session(
            &test_host.host,
            SeedSession {
                name: "repair-restart",
                lifecycle_state: LifecycleState::Starting,
                created_at,
                last_started_at: None,
                last_stopped_at: None,
                closed_at: None,
            },
        );
        let start_calls = Cell::new(0);
        let observed_lock = Cell::new(false);

        run_with_host_at_with(
            SessionArgs {
                session: crate::cli::SessionSelector::from_session(session.to_string()),
                json: false,
            },
            &test_host.host,
            |vm_name| {
                start_calls.set(start_calls.get() + 1);
                assert_eq!(vm_name.as_str(), VmName::for_session(&session).as_str());

                let conn = open_catalog(&test_host.host.state_roots.db).expect("catalog");
                let row = find_session(&conn, &session)
                    .expect("lookup session")
                    .expect("session row");
                assert_eq!(row.lock_owner_pid, Some(std::process::id() as i64));
                assert_eq!(row.lock_operation.as_deref(), Some("repair"));
                observed_lock.set(true);
                Ok(())
            },
            |_vm| panic!("restart repair should not delete the VM"),
            || completed_at,
            |_payload| panic!("non-json repair should not emit json"),
        )
        .expect("restart repair should succeed");

        assert_eq!(start_calls.get(), 1);
        assert!(
            observed_lock.get(),
            "restart path should observe lock metadata"
        );

        let conn = open_catalog(&test_host.host.state_roots.db).expect("catalog");
        let row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(row.lifecycle_state, LifecycleState::Running);
        assert_eq!(row.last_started_at, Some(completed_at));
        assert_eq!(row.last_stopped_at, None);
        assert_eq!(row.closed_at, None);
        assert_eq!(row.lock_owner_pid, None);
        assert_eq!(row.lock_operation, None);

        let events = list_events(&conn, Some(&session)).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].level, EventLevel::Info);
        assert_eq!(events[0].kind, "session.repaired");
        assert_eq!(events[0].message, "repair restarted the session VM");
        assert_eq!(events[0].at, completed_at);
    }

    #[test]
    fn repair_finish_destroy_updates_lifecycle_event_and_lock_metadata() {
        let test_host = setup_host();
        let created_at = ts("2026-04-15T00:00:00Z");
        let completed_at = ts("2026-04-15T01:45:00Z");
        let session = seed_session(
            &test_host.host,
            SeedSession {
                name: "repair-destroy",
                lifecycle_state: LifecycleState::Destroying,
                created_at,
                last_started_at: Some(created_at),
                last_stopped_at: None,
                closed_at: None,
            },
        );
        let delete_calls = Cell::new(0);
        let observed_lock = Cell::new(false);

        run_with_host_at_with(
            SessionArgs {
                session: crate::cli::SessionSelector::from_session(session.to_string()),
                json: false,
            },
            &test_host.host,
            |_vm| panic!("finish-destroy repair should not start the VM"),
            |vm_name| {
                delete_calls.set(delete_calls.get() + 1);
                assert_eq!(vm_name.as_str(), VmName::for_session(&session).as_str());

                let conn = open_catalog(&test_host.host.state_roots.db).expect("catalog");
                let row = find_session(&conn, &session)
                    .expect("lookup session")
                    .expect("session row");
                assert_eq!(row.lock_owner_pid, Some(std::process::id() as i64));
                assert_eq!(row.lock_operation.as_deref(), Some("repair"));
                observed_lock.set(true);
                Ok(())
            },
            || completed_at,
            |_payload| panic!("non-json repair should not emit json"),
        )
        .expect("finish-destroy repair should succeed");

        assert_eq!(delete_calls.get(), 1);
        assert!(
            observed_lock.get(),
            "finish-destroy path should observe lock metadata"
        );

        let conn = open_catalog(&test_host.host.state_roots.db).expect("catalog");
        let row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(row.lifecycle_state, LifecycleState::Closed);
        assert_eq!(row.last_started_at, Some(created_at));
        assert_eq!(row.last_stopped_at, Some(completed_at));
        assert_eq!(row.closed_at, Some(completed_at));
        assert_eq!(row.lock_owner_pid, None);
        assert_eq!(row.lock_operation, None);

        let events = list_events(&conn, Some(&session)).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].level, EventLevel::Info);
        assert_eq!(events[0].kind, "session.repaired");
        assert_eq!(events[0].message, "repair finished the destroy transition");
        assert_eq!(events[0].at, completed_at);
    }

    #[test]
    fn repair_manual_action_returns_blocked_without_mutation() {
        let test_host = setup_host();
        let created_at = ts("2026-04-15T00:00:00Z");
        let session = seed_session(
            &test_host.host,
            SeedSession {
                name: "repair-blocked",
                lifecycle_state: LifecycleState::Applying,
                created_at,
                last_started_at: Some(created_at),
                last_stopped_at: None,
                closed_at: None,
            },
        );

        let err = run_with_host_at_with(
            SessionArgs {
                session: crate::cli::SessionSelector::from_session(session.to_string()),
                json: false,
            },
            &test_host.host,
            |_vm| panic!("blocked repair should not start a VM"),
            |_vm| panic!("blocked repair should not delete a VM"),
            || panic!("blocked repair should not consult the clock"),
            |_payload| panic!("blocked repair should not emit json"),
        )
        .expect_err("manual action repair should block");

        assert!(matches!(
            err,
            AppError::Blocked(message) if message == "repair requires manual action: rollback_or_resume"
        ));

        let conn = open_catalog(&test_host.host.state_roots.db).expect("catalog");
        let row = find_session(&conn, &session)
            .expect("lookup session")
            .expect("session row");
        assert_eq!(row.lifecycle_state, LifecycleState::Applying);
        assert_eq!(row.last_started_at, Some(created_at));
        assert_eq!(row.last_stopped_at, None);
        assert_eq!(row.closed_at, None);
        assert_eq!(row.lock_owner_pid, None);
        assert_eq!(row.lock_operation, None);
        assert!(
            list_events(&conn, Some(&session))
                .expect("events")
                .is_empty()
        );
    }

    fn setup_host() -> TestHost {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("state");
        let host = HostContext {
            platform: HostPlatform::Macos,
            home_dir: dir.path().to_path_buf(),
            xdg_state_home: None,
            state_roots: StateRoots::from_base(&root),
        };
        let _conn = open_catalog(&host.state_roots.db).expect("catalog");
        TestHost { _dir: dir, host }
    }

    fn seed_session(host: &HostContext, seed: SeedSession<'_>) -> SessionName {
        let conn = open_catalog(&host.state_roots.db).expect("catalog");
        let session = SessionName::try_from(seed.name).expect("session");
        let vm_name = VmName::for_session(&session);

        insert_session(
            &conn,
            &InsertSession {
                name: session.clone(),
                vm_name,
                session_mode: crate::db::models::SessionMode::Repo,
                repo_sync_mode: Some(crate::db::models::RepoSyncMode::GitNative),
                host_context_path: Some(HostPath::new("/tmp/host")),
                guest_workspace_path: GuestPath::new("/home/tester.guest/repo"),
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
                created_at: seed.created_at,
            },
        )
        .expect("insert session");
        update_lifecycle_state_with_timestamps(
            &conn,
            &session,
            seed.lifecycle_state,
            &seed.created_at,
            seed.last_started_at.as_ref(),
            seed.last_stopped_at.as_ref(),
            seed.closed_at.as_ref(),
        )
        .expect("update lifecycle");
        update_sync_state(&conn, &session, SyncState::Pending, &seed.created_at)
            .expect("update sync state");
        session
    }

    fn lock_path(host: &HostContext, session: &SessionName) -> std::path::PathBuf {
        host.state_roots
            .locks
            .join(format!("{}.lock", session.as_str()))
    }

    fn ts(value: &str) -> Timestamp {
        Timestamp::parse_rfc3339(value).expect("timestamp")
    }
}
