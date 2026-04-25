use crate::db::sessions::{clear_lock_metadata, delete_session, set_lock_metadata};
use crate::error::{AppError, ValidationError};
use crate::git::session_refs::delete_ref_if_exists;
use crate::lima::instance;
use crate::types::{SessionName, VmName};
use crate::util::process::RealCommandRunner;
use std::path::Path;
use std::time::Instant;

pub fn run_step<T, F>(
    session: &SessionName,
    operation: &'static str,
    step_name: &'static str,
    total_start: &Instant,
    f: F,
) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError>,
{
    let phase_start = Instant::now();
    let result = f().map_err(|err| match err {
        AppError::Blocked(_) | AppError::Interrupted => err,
        other => AppError::Validation(ValidationError::StepFailed {
            step: step_name,
            detail: other.to_string(),
        }),
    })?;
    eprintln!(
        "{operation} {session}: {step_name} (phase {}s, total {}s)",
        phase_start.elapsed().as_secs(),
        total_start.elapsed().as_secs(),
    );
    Ok(result)
}

pub struct SessionGuard<'a> {
    operation: &'static str,
    session_name: &'a SessionName,
    cleanup: Option<Box<dyn FnOnce() -> Result<(), AppError> + 'a>>,
}

impl<'a> SessionGuard<'a> {
    pub fn launch(
        runner: &'a RealCommandRunner,
        catalog: &'a rusqlite::Connection,
        session_name: &'a SessionName,
        vm_name: &'a VmName,
    ) -> Self {
        let cleanup: Box<dyn FnOnce() -> Result<(), AppError> + 'a> = Box::new(move || {
            let _ = delete_session(catalog, session_name);
            let instances = instance::list_instances(runner)?;
            if instances.iter().any(|item| item.name == vm_name.as_str()) {
                instance::delete_instance(runner, vm_name)?;
            }
            Ok(())
        });
        Self {
            operation: "launch",
            session_name,
            cleanup: Some(cleanup),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn open(
        runner: &'a RealCommandRunner,
        catalog: &'a rusqlite::Connection,
        session_name: &'a SessionName,
        vm_name: &'a VmName,
        git_root: &'a Path,
        hidden_ref_base: &'a str,
        hidden_ref_head: &'a str,
        review_branch: &'a str,
    ) -> Self {
        let cleanup: Box<dyn FnOnce() -> Result<(), AppError> + 'a> = Box::new(move || {
            delete_session(catalog, session_name)?;
            delete_ref_if_exists(runner, git_root, hidden_ref_base)?;
            delete_ref_if_exists(runner, git_root, hidden_ref_head)?;
            delete_ref_if_exists(runner, git_root, &format!("refs/heads/{review_branch}"))?;
            let instances = instance::list_instances(runner)?;
            if instances.iter().any(|item| item.name == vm_name.as_str()) {
                instance::delete_instance(runner, vm_name)?;
            }
            Ok(())
        });
        Self {
            operation: "open",
            session_name,
            cleanup: Some(cleanup),
        }
    }

    pub fn commit(mut self) {
        self.cleanup.take();
    }

    pub fn rollback(mut self, original: AppError) -> AppError {
        let Some(cleanup) = self.cleanup.take() else {
            return original;
        };
        match cleanup() {
            Ok(()) => original,
            Err(cleanup_err) => AppError::Validation(ValidationError::RollbackFailed {
                original: original.to_string(),
                cleanup: cleanup_err.to_string(),
                operation: self.operation,
            }),
        }
    }
}

impl<'a> Drop for SessionGuard<'a> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take()
            && let Err(err) = cleanup()
        {
            eprintln!(
                "{} rollback cleanup failed for session {}: {err}",
                self.operation, self.session_name
            );
        }
    }
}

pub struct LockMetadataGuard<'a> {
    catalog: &'a rusqlite::Connection,
    session_name: &'a SessionName,
    cleared: bool,
}

impl<'a> LockMetadataGuard<'a> {
    pub fn acquire(
        catalog: &'a rusqlite::Connection,
        session_name: &'a SessionName,
        pid: u32,
        operation: &'static str,
    ) -> Result<Self, AppError> {
        set_lock_metadata(catalog, session_name, pid, operation)?;
        Ok(Self {
            catalog,
            session_name,
            cleared: false,
        })
    }

    pub fn commit(mut self) -> Result<(), AppError> {
        clear_lock_metadata(self.catalog, self.session_name)?;
        self.cleared = true;
        Ok(())
    }
}

impl<'a> Drop for LockMetadataGuard<'a> {
    fn drop(&mut self) {
        if !self.cleared
            && let Err(err) = clear_lock_metadata(self.catalog, self.session_name)
        {
            eprintln!(
                "lock metadata cleanup failed for session {}: {err}",
                self.session_name
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::open_catalog;
    use crate::db::sessions::{InsertSession, find_session, insert_session};
    use tempfile::tempdir;

    #[test]
    fn run_step_returns_ok_on_success() {
        let session = SessionName::try_from("demo").expect("session");
        let total_start = Instant::now();
        let result: Result<i32, AppError> =
            run_step(&session, "launch", "clone-vm", &total_start, || Ok(42));
        assert_eq!(result.expect("ok value"), 42);
    }

    #[test]
    fn run_step_wraps_inner_error_as_step_failed() {
        let session = SessionName::try_from("demo").expect("session");
        let total_start = Instant::now();
        let result: Result<(), AppError> =
            run_step(&session, "launch", "clone-vm", &total_start, || {
                Err(AppError::Validation(ValidationError::UnsupportedHost))
            });
        let err = result.expect_err("should fail");
        match err {
            AppError::Validation(ValidationError::StepFailed { step, detail }) => {
                assert_eq!(step, "clone-vm");
                assert!(
                    detail.contains("unsupported host"),
                    "detail should include inner message: {detail}"
                );
            }
            other => panic!("expected StepFailed, got {other:?}"),
        }
    }

    #[test]
    fn run_step_preserves_blocked_errors() {
        let session = SessionName::try_from("demo").expect("session");
        let total_start = Instant::now();
        let result: Result<(), AppError> =
            run_step(&session, "launch", "clone-vm", &total_start, || {
                Err(AppError::Blocked("base is busy".to_owned()))
            });

        assert!(matches!(
            result.expect_err("should fail"),
            AppError::Blocked(message) if message == "base is busy"
        ));
    }

    fn seed_session(conn: &rusqlite::Connection, session: &SessionName, vm: &VmName) {
        insert_session(
            conn,
            &InsertSession {
                vm_name: vm.clone(),
                ..crate::testing::test_repo_session(
                    session.as_str(),
                    crate::testing::ts("2026-04-24T00:00:00Z"),
                )
            },
        )
        .expect("insert session");
    }

    #[test]
    fn session_guard_commit_preserves_state() {
        let dir = tempdir().expect("tempdir");
        let conn = open_catalog(&dir.path().join("state.db")).expect("catalog");
        let session = SessionName::try_from("demo-commit").expect("session");
        let vm = VmName::for_session(&session);
        seed_session(&conn, &session, &vm);

        let runner = RealCommandRunner;
        {
            let guard = SessionGuard::launch(&runner, &conn, &session, &vm);
            guard.commit();
        }

        let row = find_session(&conn, &session).expect("find");
        assert!(row.is_some(), "session should survive commit");
    }

    #[test]
    fn session_guard_drop_runs_cleanup_when_not_committed() {
        let dir = tempdir().expect("tempdir");
        let conn = open_catalog(&dir.path().join("state.db")).expect("catalog");
        let session = SessionName::try_from("demo-drop").expect("session");
        let vm = VmName::for_session(&session);
        seed_session(&conn, &session, &vm);

        let runner = RealCommandRunner;
        {
            let _guard = SessionGuard::launch(&runner, &conn, &session, &vm);
            // guard dropped without commit or rollback
        }

        let row = find_session(&conn, &session).expect("find");
        assert!(row.is_none(), "session should be deleted by Drop cleanup");
    }

    #[test]
    fn session_guard_rollback_returns_original_on_clean_cleanup() {
        let dir = tempdir().expect("tempdir");
        let conn = open_catalog(&dir.path().join("state.db")).expect("catalog");
        let session = SessionName::try_from("demo-rollback-ok").expect("session");
        let vm = VmName::for_session(&session);
        seed_session(&conn, &session, &vm);

        let runner = RealCommandRunner;
        let guard = SessionGuard::launch(&runner, &conn, &session, &vm);
        let returned = guard.rollback(AppError::Interrupted);

        assert!(
            matches!(returned, AppError::Interrupted),
            "rollback should preserve original error on clean cleanup, got {returned:?}"
        );
        let row = find_session(&conn, &session).expect("find");
        assert!(
            row.is_none(),
            "session row should be deleted after rollback"
        );
    }
}
