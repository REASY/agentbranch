#![allow(dead_code, unused_imports)]

#[path = "../src/commands/repair.rs"]
mod repair_impl;

mod cli {
    pub use agbranch::cli::*;
}

mod db {
    pub mod connect {
        pub use agbranch::db::connect::*;
    }

    pub mod events {
        pub use agbranch::db::events::*;
    }

    pub mod locks {
        pub use agbranch::db::locks::*;
    }

    pub mod models {
        pub use agbranch::db::models::*;
    }

    pub mod sessions {
        pub use agbranch::db::sessions::*;
    }

    pub mod sync_runs {
        pub use agbranch::db::sync_runs::*;
    }
}

mod error {
    pub use agbranch::error::{AppError, ValidationError};

    pub mod lima {
        pub use agbranch::error::lima::*;
    }
}

mod lima {
    pub mod instance {
        pub use agbranch::lima::instance::*;
    }
}

mod platform {
    pub mod detect {
        pub use agbranch::platform::detect::*;
    }

    pub mod host {
        pub use agbranch::platform::host::*;
    }

    pub mod paths {
        pub use agbranch::platform::paths::*;
    }
}

mod session {
    pub mod orchestration {
        pub use agbranch::session::orchestration::*;
    }
    pub mod reconcile {
        pub use agbranch::session::reconcile::*;
    }
}

mod types {
    pub use agbranch::types::*;
}

mod util {
    pub mod process {
        pub use agbranch::util::process::*;
    }

    pub mod time {
        pub use agbranch::util::time::*;
    }
}

use db::connect::open_catalog;
use db::events::list_events;
use db::models::{
    EventLevel, LifecycleState, RepoSyncMode, SessionMode, SyncDirection, SyncRunResult, SyncState,
};
use db::sessions::{
    InsertSession, find_session, insert_session, update_lifecycle_state_with_timestamps,
    update_sync_state,
};
use db::sync_runs::{finish_sync_run, insert_sync_run, list_sync_runs_for_session};
use error::AppError;
use repair_impl::{RepairExecution, apply_repair_action_at_with};
use session::reconcile::{RepairAction, repair_action_for_state};
use std::cell::{Cell, RefCell};
use tempfile::tempdir;
use types::{GuestPath, HostPath, SessionName, Timestamp, VmName};

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
fn sync_run_db_helpers_round_trip_finished_metadata() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("state.db");
    let conn = open_catalog(&db).expect("catalog");
    let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("created_at");
    let started_at = Timestamp::parse_rfc3339("2026-04-15T00:30:00Z").expect("started_at");
    let finished_at = Timestamp::parse_rfc3339("2026-04-15T00:35:00Z").expect("finished_at");
    let session = seed_session(
        &conn,
        SeedSession {
            name: "feat-sync-run",
            lifecycle_state: LifecycleState::Running,
            sync_state: SyncState::Pending,
            created_at,
            last_started_at: Some(created_at),
            last_stopped_at: None,
            closed_at: None,
        },
    );

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
        SyncRunResult::Blocked,
        finished_at,
        Some("/tmp/staging"),
        Some("/tmp/patch.diff"),
        Some("host drift detected"),
    )
    .expect("finish sync run");

    let runs = list_sync_runs_for_session(&conn, &session).expect("list sync runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, run_id);
    assert_eq!(runs[0].session_id, session);
    assert_eq!(runs[0].direction, SyncDirection::SyncBack);
    assert_eq!(runs[0].result, SyncRunResult::Blocked);
    assert_eq!(runs[0].started_at, started_at);
    assert_eq!(runs[0].finished_at, Some(finished_at));
    assert_eq!(runs[0].staging_path.as_deref(), Some("/tmp/staging"));
    assert_eq!(runs[0].patch_path.as_deref(), Some("/tmp/patch.diff"));
    assert_eq!(runs[0].error_text.as_deref(), Some("host drift detected"));
}

#[test]
fn finish_destroy_repair_closes_session_and_emits_event() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("state.db");
    let conn = open_catalog(&db).expect("catalog");
    let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("created_at");
    let repair_at = Timestamp::parse_rfc3339("2026-04-15T01:00:00Z").expect("repair_at");
    let session = seed_session(
        &conn,
        SeedSession {
            name: "feat-repair",
            lifecycle_state: LifecycleState::Destroying,
            sync_state: SyncState::Pending,
            created_at,
            last_started_at: Some(created_at),
            last_stopped_at: None,
            closed_at: None,
        },
    );

    let start_calls = Cell::new(0);
    let delete_calls = Cell::new(0);
    let order = RefCell::new(Vec::new());
    let vm_name = VmName::for_session(&session);

    apply_repair_action_at_with(
        RepairExecution {
            conn: &conn,
            session_name: &session,
            vm_name: &vm_name,
            action: RepairAction::FinishDestroy,
            lifecycle_state: LifecycleState::Destroying,
        },
        |_vm_name| {
            start_calls.set(start_calls.get() + 1);
            Ok(())
        },
        |deleted_vm: &VmName| {
            order.borrow_mut().push("delete");
            delete_calls.set(delete_calls.get() + 1);
            assert_eq!(deleted_vm.as_str(), vm_name.as_str());
            Ok(())
        },
        || {
            assert_eq!(&*order.borrow(), &["delete"]);
            repair_at
        },
    )
    .expect("repair should finish destroy");

    assert_eq!(start_calls.get(), 0);
    assert_eq!(delete_calls.get(), 1);

    let session_row = find_session(&conn, &session)
        .expect("lookup session")
        .expect("session row");
    assert_eq!(session_row.lifecycle_state, LifecycleState::Closed);
    assert_eq!(session_row.last_stopped_at, Some(repair_at));
    assert_eq!(session_row.closed_at, Some(repair_at));

    let events = list_events(&conn, Some(&session)).expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "session.repaired");
    assert_eq!(events[0].message, "repair finished the destroy transition");
    assert_eq!(events[0].at, repair_at);
}

#[test]
fn restart_repair_records_completion_timestamp_after_vm_start() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("state.db");
    let conn = open_catalog(&db).expect("catalog");
    let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("created_at");
    let restarted_at = Timestamp::parse_rfc3339("2026-04-15T01:05:00Z").expect("restarted_at");
    let session = seed_session(
        &conn,
        SeedSession {
            name: "feat-restart-repair",
            lifecycle_state: LifecycleState::Starting,
            sync_state: SyncState::Pending,
            created_at,
            last_started_at: None,
            last_stopped_at: None,
            closed_at: None,
        },
    );

    let order = RefCell::new(Vec::new());
    let vm_name = VmName::for_session(&session);

    apply_repair_action_at_with(
        RepairExecution {
            conn: &conn,
            session_name: &session,
            vm_name: &vm_name,
            action: RepairAction::Restart,
            lifecycle_state: LifecycleState::Starting,
        },
        |started_vm: &VmName| {
            order.borrow_mut().push("start");
            assert_eq!(started_vm.as_str(), vm_name.as_str());
            Ok(())
        },
        |_vm_name| -> Result<(), AppError> { panic!("delete should not run") },
        || {
            assert_eq!(&*order.borrow(), &["start"]);
            restarted_at
        },
    )
    .expect("repair restart should succeed");

    let session_row = find_session(&conn, &session)
        .expect("lookup session")
        .expect("session row");
    assert_eq!(session_row.lifecycle_state, LifecycleState::Running);
    assert_eq!(session_row.last_started_at, Some(restarted_at));
    assert_eq!(session_row.last_stopped_at, None);
    assert_eq!(session_row.closed_at, None);

    let events = list_events(&conn, Some(&session)).expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "session.repaired");
    assert_eq!(events[0].message, "repair restarted the session VM");
    assert_eq!(events[0].at, restarted_at);
}

#[test]
fn manual_repair_action_returns_blocked_without_mutating_session() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("state.db");
    let conn = open_catalog(&db).expect("catalog");
    let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("created_at");
    let session = seed_session(
        &conn,
        SeedSession {
            name: "feat-manual",
            lifecycle_state: LifecycleState::Applying,
            sync_state: SyncState::Pending,
            created_at,
            last_started_at: Some(created_at),
            last_stopped_at: None,
            closed_at: None,
        },
    );

    let repair_vm = VmName::for_session(&session);
    let result = apply_repair_action_at_with(
        RepairExecution {
            conn: &conn,
            session_name: &session,
            vm_name: &repair_vm,
            action: repair_action_for_state(LifecycleState::Applying),
            lifecycle_state: LifecycleState::Applying,
        },
        |_vm_name| -> Result<(), AppError> { panic!("restart should not run") },
        |_vm_name| -> Result<(), AppError> { panic!("delete should not run") },
        || -> Timestamp { panic!("clock should not be read for manual repair actions") },
    );

    assert!(matches!(
        result,
        Err(AppError::Blocked(message)) if message == "repair requires manual action: rollback_or_resume"
    ));

    let session_row = find_session(&conn, &session)
        .expect("lookup session")
        .expect("session row");
    assert_eq!(session_row.lifecycle_state, LifecycleState::Applying);
    assert_eq!(session_row.last_stopped_at, None);
    assert_eq!(session_row.closed_at, None);

    let events = list_events(&conn, Some(&session)).expect("events");
    assert!(events.is_empty());
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
            host_git_root: Some(HostPath::new("/tmp/host")),
            host_head_oid_at_open: Some("abc123".to_owned()),
            host_head_ref_at_open: Some("refs/heads/main".to_owned()),
            host_dirty_at_open: false,
            base_ref: Some("refs/heads/main".to_owned()),
            review_branch: Some(format!("agbranch/{}", input.name)),
            session_ref_base: Some(format!("refs/agbranch/sessions/{}/base", input.name)),
            session_ref_head: Some(format!("refs/agbranch/sessions/{}/head", input.name)),
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
