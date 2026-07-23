use crate::db::sessions::{clear_lock_metadata, delete_session, set_lock_metadata};
use crate::error::{AppError, ValidationError};
use crate::git::session_refs::delete_ref_if_exists;
use crate::lima::instance;
use crate::types::{SessionName, VmName};
use crate::util::process::RealCommandRunner;
use serde::Serialize;
use std::cell::RefCell;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhaseTiming {
    pub name: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimingSummary {
    pub total_ms: u64,
    pub phases: Vec<PhaseTiming>,
    pub slowest_phase: Option<PhaseTiming>,
}

impl TimingSummary {
    pub fn render_human(&self, operation: &str, session: &SessionName) -> String {
        let mut lines = vec![format!(
            "{operation} {session}: completed in {}",
            format_milliseconds(self.total_ms)
        )];
        for phase in &self.phases {
            lines.push(format!(
                "  {:<24} {:>9} {:>5.1}%",
                phase.name,
                format_milliseconds(phase.duration_ms),
                percentage(phase.duration_ms, self.total_ms),
            ));
        }
        if let Some(slowest) = &self.slowest_phase {
            lines.push(format!(
                "  slowest: {} {} ({:.1}%)",
                slowest.name,
                format_milliseconds(slowest.duration_ms),
                percentage(slowest.duration_ms, self.total_ms),
            ));
        }
        lines.join("\n")
    }
}

pub struct OperationTimings {
    started: Instant,
    phases: RefCell<Vec<PhaseTiming>>,
}

impl OperationTimings {
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
            phases: RefCell::new(Vec::new()),
        }
    }

    fn record(&self, name: &'static str, duration: Duration) {
        self.phases.borrow_mut().push(PhaseTiming {
            name: name.to_owned(),
            duration_ms: duration_milliseconds(duration),
        });
    }

    pub fn summary(&self) -> TimingSummary {
        let phases = self.phases.borrow().clone();
        let slowest_phase = phases.iter().max_by_key(|phase| phase.duration_ms).cloned();
        TimingSummary {
            total_ms: duration_milliseconds(self.started.elapsed()),
            phases,
            slowest_phase,
        }
    }
}

pub fn run_step<T, F>(
    session: &SessionName,
    operation: &'static str,
    step_name: &'static str,
    timings: &OperationTimings,
    f: F,
) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError>,
{
    eprintln!(
        "{}",
        render_step_started(operation, session, step_name, timings.started.elapsed())
    );
    let phase_start = Instant::now();
    let result = f();
    let phase_duration = phase_start.elapsed();
    timings.record(step_name, phase_duration);
    let result = result.map_err(|err| match err {
        AppError::Blocked(_) | AppError::Interrupted => err,
        other => AppError::Validation(ValidationError::StepFailed {
            step: step_name,
            detail: other.to_string(),
        }),
    })?;
    eprintln!(
        "{}",
        render_step_completed(
            operation,
            session,
            step_name,
            phase_duration,
            timings.started.elapsed(),
        )
    );
    Ok(result)
}

fn render_step_started(
    operation: &str,
    session: &SessionName,
    step_name: &str,
    total: Duration,
) -> String {
    format!(
        "{operation} {session}: {step_name} (started, total {})",
        format_duration(total)
    )
}

fn render_step_completed(
    operation: &str,
    session: &SessionName,
    step_name: &str,
    phase: Duration,
    total: Duration,
) -> String {
    format!(
        "{operation} {session}: {step_name} (phase {}, total {})",
        format_duration(phase),
        format_duration(total)
    )
}

fn duration_milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn format_duration(duration: Duration) -> String {
    format_milliseconds(duration_milliseconds(duration))
}

fn format_milliseconds(milliseconds: u64) -> String {
    format!("{}.{:03}s", milliseconds / 1_000, milliseconds % 1_000)
}

fn percentage(duration_ms: u64, total_ms: u64) -> f64 {
    if total_ms == 0 {
        0.0
    } else {
        duration_ms as f64 * 100.0 / total_ms as f64
    }
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
        let timings = OperationTimings::start();
        let result: Result<i32, AppError> =
            run_step(&session, "launch", "clone-vm", &timings, || Ok(42));
        assert_eq!(result.expect("ok value"), 42);
        assert_eq!(timings.summary().phases[0].name, "clone-vm");
    }

    #[test]
    fn run_step_wraps_inner_error_as_step_failed() {
        let session = SessionName::try_from("demo").expect("session");
        let timings = OperationTimings::start();
        let result: Result<(), AppError> =
            run_step(&session, "launch", "clone-vm", &timings, || {
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
        let timings = OperationTimings::start();
        let result: Result<(), AppError> =
            run_step(&session, "launch", "clone-vm", &timings, || {
                Err(AppError::Blocked("base is busy".to_owned()))
            });

        assert!(matches!(
            result.expect_err("should fail"),
            AppError::Blocked(message) if message == "base is busy"
        ));
    }

    #[test]
    fn timing_summary_renders_milliseconds_percentages_and_slowest_phase() {
        let session = SessionName::try_from("demo").expect("session");
        let summary = TimingSummary {
            total_ms: 14_000,
            phases: vec![
                PhaseTiming {
                    name: "clone-vm".to_owned(),
                    duration_ms: 1_000,
                },
                PhaseTiming {
                    name: "start-vm".to_owned(),
                    duration_ms: 12_500,
                },
            ],
            slowest_phase: Some(PhaseTiming {
                name: "start-vm".to_owned(),
                duration_ms: 12_500,
            }),
        };

        let rendered = summary.render_human("launch", &session);
        assert!(rendered.contains("completed in 14.000s"));
        assert!(rendered.contains("clone-vm"));
        assert!(rendered.contains("7.1%"));
        assert!(rendered.contains("slowest: start-vm 12.500s (89.3%)"));
    }

    #[test]
    fn timing_summary_serializes_structured_milliseconds() {
        let summary = TimingSummary {
            total_ms: 1_250,
            phases: vec![PhaseTiming {
                name: "start-vm".to_owned(),
                duration_ms: 1_000,
            }],
            slowest_phase: Some(PhaseTiming {
                name: "start-vm".to_owned(),
                duration_ms: 1_000,
            }),
        };

        let value = serde_json::to_value(summary).expect("serialize timings");
        assert_eq!(value["total_ms"], 1_250);
        assert_eq!(value["phases"][0]["name"], "start-vm");
        assert_eq!(value["phases"][0]["duration_ms"], 1_000);
        assert_eq!(value["slowest_phase"]["name"], "start-vm");
    }

    #[test]
    fn phase_progress_has_distinct_started_and_completed_lines() {
        let session = SessionName::try_from("demo").expect("session");
        assert_eq!(
            render_step_started("launch", &session, "start-vm", Duration::from_millis(1_960),),
            "launch demo: start-vm (started, total 1.960s)"
        );
        assert_eq!(
            render_step_completed(
                "launch",
                &session,
                "start-vm",
                Duration::from_millis(17_060),
                Duration::from_millis(19_020),
            ),
            "launch demo: start-vm (phase 17.060s, total 19.020s)"
        );
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
