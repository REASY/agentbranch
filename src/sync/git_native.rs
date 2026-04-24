use crate::cli::SyncBackArgs;
use crate::db::events::append_event;
use crate::db::models::{EventLevel, LifecycleState, SyncDirection, SyncRunResult, SyncState};
use crate::db::sessions::{SessionRow, update_lifecycle_state_with_timestamps, update_sync_state};
use crate::db::sync_runs::{finish_sync_run, insert_sync_run};
use crate::error::{AppError, ValidationError};
use crate::git::bundle::{
    create_guest_sync_bundle, fetch_bundle_ref, guest_head_ref, guest_repo_is_dirty,
};
use crate::git::session_refs::{
    fast_forward_review_branch, incoming_ref_name, is_ancestor, resolve_ref_oid, update_ref,
};
use crate::lima::inspect::LimaInstanceStatus;
use crate::lima::instance::{list_instances, start_instance};
use crate::platform::host::HostContext;
use crate::policy::sync_plan::{SyncBlockReason, blocked_reason_summary, detect_dirty_sync_block};
use crate::sync::artifacts::{
    BlockedSyncArtifacts, SyncBackOutcome, format_sync_back_outcome, prepare_blocked_sync_artifacts,
};
use crate::types::{HostPath, SessionName, Timestamp, VmName};
use crate::util::process::CommandRunner;
use std::fs;

pub(crate) struct GitNativeSyncExecution<'a> {
    pub(crate) args: SyncBackArgs,
    pub(crate) host: &'a HostContext,
    pub(crate) catalog: &'a rusqlite::Connection,
    pub(crate) session_name: &'a SessionName,
    pub(crate) session: &'a SessionRow,
}

enum SyncOutcome<'a> {
    Success {
        staged_path: &'a HostPath,
    },
    Blocked {
        artifacts: BlockedSyncArtifacts,
        reason_summary: String,
    },
}

fn record_sync_outcome(
    catalog: &rusqlite::Connection,
    session_name: &SessionName,
    started_at: Timestamp,
    completed_at: Timestamp,
    outcome: SyncOutcome<'_>,
) -> Result<(), AppError> {
    let (result, staged, patch, error_text, event_level, event_kind, event_message, sync_state) =
        match &outcome {
            SyncOutcome::Success { staged_path } => (
                SyncRunResult::Success,
                Some(staged_path.to_string()),
                None,
                None,
                EventLevel::Info,
                "sync_back.git_native",
                "git-native sync-back completed".to_owned(),
                SyncState::Clean,
            ),
            SyncOutcome::Blocked {
                artifacts,
                reason_summary,
            } => (
                SyncRunResult::Blocked,
                artifacts
                    .staged_path
                    .as_ref()
                    .map(|p| p.display().to_string()),
                artifacts
                    .patch_path
                    .as_ref()
                    .map(|p| p.display().to_string()),
                Some(reason_summary.clone()),
                EventLevel::Warn,
                "sync_blocked",
                reason_summary.clone(),
                SyncState::Blocked,
            ),
        };

    let sync_run_id = insert_sync_run(
        catalog,
        session_name,
        SyncDirection::SyncBack,
        result,
        started_at,
    )?;
    finish_sync_run(
        catalog,
        sync_run_id,
        result,
        completed_at,
        staged.as_deref(),
        patch.as_deref(),
        error_text.as_deref(),
    )?;
    append_event(
        catalog,
        session_name,
        event_level,
        event_kind,
        &event_message,
        completed_at,
    )?;
    update_sync_state(catalog, session_name, sync_state, &completed_at)?;
    update_lifecycle_state_with_timestamps(
        catalog,
        session_name,
        LifecycleState::Running,
        &completed_at,
        None,
        None,
        None,
    )?;
    Ok(())
}

pub(crate) fn run_git_native_sync_back<NowFn, EmitFn>(
    execution: GitNativeSyncExecution<'_>,
    runner: &dyn CommandRunner,
    now_fn: &mut NowFn,
    emit_outcome_fn: &mut EmitFn,
) -> Result<(), AppError>
where
    NowFn: FnMut() -> Timestamp,
    EmitFn: FnMut(String) -> Result<(), AppError>,
{
    let GitNativeSyncExecution {
        args,
        host,
        catalog,
        session_name,
        session,
    } = execution;
    let host_git_root = session.host_git_root.as_ref().ok_or_else(|| {
        AppError::Validation(ValidationError::SyncBackMissingSessionField {
            field: "host git root",
        })
    })?;
    let review_branch = session.review_branch.as_deref().ok_or_else(|| {
        AppError::Validation(ValidationError::SyncBackMissingSessionField {
            field: "review branch",
        })
    })?;
    let session_ref_head = session.session_ref_head.as_deref().ok_or_else(|| {
        AppError::Validation(ValidationError::SyncBackMissingSessionField {
            field: "session head ref",
        })
    })?;
    let incoming_ref = incoming_ref_name(session_name);

    let started_at = now_fn();
    ensure_instance_running(runner, &session.vm_name)?;
    update_lifecycle_state_with_timestamps(
        catalog,
        session_name,
        LifecycleState::Syncing,
        &started_at,
        None,
        None,
        None,
    )?;

    let dirty = guest_repo_is_dirty(runner, &session.vm_name, &session.guest_workspace_path)?;
    let reasons = detect_dirty_sync_block(dirty);
    if !reasons.is_empty() {
        return block_git_native_sync(
            args,
            catalog,
            session_name,
            started_at,
            now_fn(),
            &reasons,
            BlockedSyncArtifacts::default(),
            emit_outcome_fn,
        );
    }
    let guest_head = guest_head_ref(runner, &session.vm_name, &session.guest_workspace_path)?;
    let expected_head = format!("refs/heads/{review_branch}");
    if guest_head.as_deref() != Some(expected_head.as_str()) {
        return block_git_native_sync(
            args,
            catalog,
            session_name,
            started_at,
            now_fn(),
            &[SyncBlockReason::GuestNotOnReviewBranch],
            BlockedSyncArtifacts::default(),
            emit_outcome_fn,
        );
    }

    fs::create_dir_all(&host.state_roots.staging)?;
    let bundle_path = HostPath::new(
        host.state_roots
            .staging
            .join(format!("{}.bundle", session_name.as_str())),
    );
    create_guest_sync_bundle(
        runner,
        &session.vm_name,
        &session.guest_workspace_path,
        "HEAD",
        &bundle_path,
    )?;
    fetch_bundle_ref(runner, host_git_root, &bundle_path, "HEAD", &incoming_ref)?;

    let current_head = resolve_ref_oid(runner, host_git_root.as_path(), session_ref_head)?;
    let fetched_head = resolve_ref_oid(runner, host_git_root.as_path(), &incoming_ref)?;
    if !is_ancestor(
        runner,
        host_git_root.as_path(),
        &current_head,
        &fetched_head,
    )? {
        let artifacts = prepare_blocked_sync_artifacts(
            &args,
            host_git_root.as_path(),
            &current_head,
            &fetched_head,
            &bundle_path,
        )?;
        return block_git_native_sync(
            args,
            catalog,
            session_name,
            started_at,
            now_fn(),
            &[SyncBlockReason::SessionHeadRewritten],
            artifacts,
            emit_outcome_fn,
        );
    }

    let review_updated = fast_forward_review_branch(
        runner,
        host_git_root.as_path(),
        review_branch,
        &incoming_ref,
    )?;
    if !review_updated {
        let artifacts = prepare_blocked_sync_artifacts(
            &args,
            host_git_root.as_path(),
            &current_head,
            &fetched_head,
            &bundle_path,
        )?;
        return block_git_native_sync(
            args,
            catalog,
            session_name,
            started_at,
            now_fn(),
            &[SyncBlockReason::ReviewBranchDiverged],
            artifacts,
            emit_outcome_fn,
        );
    }
    update_ref(
        runner,
        host_git_root.as_path(),
        session_ref_head,
        &fetched_head,
    )?;

    let completed_at = now_fn();
    record_sync_outcome(
        catalog,
        session_name,
        started_at,
        completed_at,
        SyncOutcome::Success {
            staged_path: &bundle_path,
        },
    )?;

    let outcome = SyncBackOutcome {
        blocked: false,
        patch_path: None,
        staged_path: bundle_path.as_path().to_path_buf(),
    };
    emit_outcome_fn(format_sync_back_outcome(&outcome, args.json)?)
}

#[allow(clippy::too_many_arguments)]
fn block_git_native_sync<EmitFn>(
    args: SyncBackArgs,
    catalog: &rusqlite::Connection,
    session_name: &SessionName,
    started_at: Timestamp,
    completed_at: Timestamp,
    reasons: &[SyncBlockReason],
    artifacts: BlockedSyncArtifacts,
    emit_outcome_fn: &mut EmitFn,
) -> Result<(), AppError>
where
    EmitFn: FnMut(String) -> Result<(), AppError>,
{
    let reason_summary = blocked_reason_summary(reasons);

    record_sync_outcome(
        catalog,
        session_name,
        started_at,
        completed_at,
        SyncOutcome::Blocked {
            artifacts: artifacts.clone(),
            reason_summary: reason_summary.clone(),
        },
    )?;

    if args.json {
        let outcome = SyncBackOutcome {
            blocked: true,
            patch_path: artifacts.patch_path,
            staged_path: artifacts.staged_path.unwrap_or_default(),
        };
        emit_outcome_fn(format_sync_back_outcome(&outcome, true)?)?;
    }

    Err(AppError::Blocked(format!(
        "session `{}` is blocked: {}",
        session_name, reason_summary
    )))
}

pub(crate) fn ensure_instance_running(
    runner: &dyn CommandRunner,
    vm_name: &VmName,
) -> Result<(), AppError> {
    let instances = list_instances(runner)?;
    let running = instances
        .iter()
        .find(|item| item.name == vm_name.as_str())
        .map(|item| item.status == LimaInstanceStatus::Running)
        .unwrap_or(false);
    if !running {
        start_instance(runner, vm_name)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::open_catalog;
    use crate::db::models::{RepoSyncMode, SessionMode, Timestamp};
    use crate::db::sessions::{
        InsertSession, find_session, insert_session, update_lifecycle_state_with_timestamps,
        update_sync_state,
    };
    use crate::git::session_refs::{hidden_ref_names, review_branch_name};
    use crate::lima::shell::shell_escape;
    use crate::sync::{SyncBackExecution, SyncBackHooks, run_with_catalog_at_with};
    use crate::types::{GuestPath, HostPath};
    use crate::util::process::RealCommandRunner;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::tempdir;

    #[derive(Debug)]
    struct GitNativeLimaRunner {
        vm_name: String,
        status: RefCell<&'static str>,
        guest_repo_path: GuestPath,
        guest_repo_local: PathBuf,
        guest_bundle_local: PathBuf,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl GitNativeLimaRunner {
        fn new(
            vm_name: &VmName,
            status: &'static str,
            guest_repo_path: &GuestPath,
            guest_repo_local: &Path,
            guest_bundle_local: &Path,
        ) -> Self {
            Self {
                vm_name: vm_name.as_str().to_owned(),
                status: RefCell::new(status),
                guest_repo_path: guest_repo_path.clone(),
                guest_repo_local: guest_repo_local.to_path_buf(),
                guest_bundle_local: guest_bundle_local.to_path_buf(),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for GitNativeLimaRunner {
        fn run(
            &self,
            program: &str,
            args: &[String],
            cwd: Option<&Path>,
            env: &BTreeMap<String, String>,
        ) -> Result<crate::util::process::CommandOutput, crate::error::process::ProcessError>
        {
            if program != "limactl" {
                return RealCommandRunner.run(program, args, cwd, env);
            }

            self.calls.borrow_mut().push(args.to_vec());
            match args {
                [list, json] if list == "list" && json == "--json" => {
                    Ok(crate::util::process::CommandOutput {
                        stdout: format!(
                            r#"[{{"name":"{}","status":"{}","vmType":"vz","dir":"/tmp/{}","sshConfigFile":"/tmp/{}/ssh.config"}}]"#,
                            self.vm_name,
                            self.status.borrow(),
                            self.vm_name,
                            self.vm_name,
                        ),
                        stderr: String::new(),
                    })
                }
                [start, name] if start == "start" && name == &self.vm_name => {
                    *self.status.borrow_mut() = "Running";
                    Ok(crate::util::process::CommandOutput {
                        stdout: String::new(),
                        stderr: String::new(),
                    })
                }
                [shell, name, dash_dash, test, dash_d, path]
                    if shell == "shell"
                        && name == &self.vm_name
                        && dash_dash == "--"
                        && test == "test"
                        && dash_d == "-d"
                        && path == "/tmp/agbranch-sync.bundle" =>
                {
                    Err(crate::error::process::ProcessError::Failed {
                        program: "limactl".to_owned(),
                        status: 1,
                        stderr: String::new(),
                    })
                }
                [shell, name, dash_dash, bash, dash_lc, script]
                    if shell == "shell"
                        && name == &self.vm_name
                        && dash_dash == "--"
                        && bash == "bash"
                        && dash_lc == "-lc" =>
                {
                    let guest_repo = shell_escape(&self.guest_repo_path.to_string());
                    let local_repo =
                        shell_escape(&self.guest_repo_local.as_path().display().to_string());
                    let guest_bundle = shell_escape("/tmp/agbranch-sync.bundle");
                    let local_bundle =
                        shell_escape(&self.guest_bundle_local.as_path().display().to_string());
                    let rewritten = script
                        .replace(&guest_repo, &local_repo)
                        .replace(&guest_bundle, &local_bundle);
                    RealCommandRunner.run(
                        "bash",
                        &["-lc".to_owned(), rewritten],
                        None,
                        &BTreeMap::new(),
                    )
                }
                [copy, backend, recursive, from, to]
                    if copy == "copy"
                        && backend == "--backend=rsync"
                        && recursive == "-r"
                        && from == &format!("{}:{}", self.vm_name, "/tmp/agbranch-sync.bundle") =>
                {
                    std::fs::copy(&self.guest_bundle_local, to).map_err(|source| {
                        crate::error::process::ProcessError::Spawn {
                            program: "cp".to_owned(),
                            source,
                        }
                    })?;
                    Ok(crate::util::process::CommandOutput {
                        stdout: String::new(),
                        stderr: String::new(),
                    })
                }
                [copy, backend, from, to]
                    if copy == "copy"
                        && backend == "--backend=rsync"
                        && from == &format!("{}:{}", self.vm_name, "/tmp/agbranch-sync.bundle") =>
                {
                    std::fs::copy(&self.guest_bundle_local, to).map_err(|source| {
                        crate::error::process::ProcessError::Spawn {
                            program: "cp".to_owned(),
                            source,
                        }
                    })?;
                    Ok(crate::util::process::CommandOutput {
                        stdout: String::new(),
                        stderr: String::new(),
                    })
                }
                other => panic!("unexpected limactl invocation: {other:?}"),
            }
        }
    }

    #[test]
    fn git_native_sync_blocks_when_guest_head_is_not_review_branch() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let conn = open_catalog(&host.state_roots.db).expect("catalog");
        let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("created_at");
        let started_at = Timestamp::parse_rfc3339("2026-04-15T01:00:00Z").expect("started_at");
        let completed_at = Timestamp::parse_rfc3339("2026-04-15T01:05:00Z").expect("completed_at");
        let repo = dir.path().join("host-repo");
        let guest = dir.path().join("guest-repo");

        init_git_repo(&repo);
        fs::write(repo.join("README.md"), "host\n").expect("host readme");
        git_commit_all(&repo, "initial");

        let guest_workspace =
            GuestPath::new("/home/tester.guest/workspaces/runtime-head-mismatch/repo");
        let session = seed_runtime_repo_session(
            &conn,
            "runtime-head-mismatch",
            &repo,
            &guest_workspace,
            created_at,
        );
        let review_branch = review_branch_name(&session);
        clone_repo(&repo, &guest);
        git_checkout_branch(&guest, &review_branch);
        git_checkout_new_branch(&guest, "scratch-work");

        let vm_name = VmName::for_session(&session);
        let bundle_path = dir.path().join("guest-sync.bundle");
        let runner =
            GitNativeLimaRunner::new(&vm_name, "Running", &guest_workspace, &guest, &bundle_path);
        let emitted = RefCell::new(Vec::<String>::new());

        let mut clock = vec![started_at, completed_at].into_iter();
        let err = run_with_catalog_at_with(
            SyncBackExecution {
                args: crate::cli::SyncBackArgs {
                    session: crate::cli::SessionSelector::from_session(session.to_string()),
                    yes: true,
                    export_patch: None,
                    json: true,
                },
                host: &host,
                catalog: &conn,
                session_name: &session,
            },
            SyncBackHooks {
                runner: &runner,
                now_fn: move || clock.next().expect("clock tick"),
                emit_outcome_fn: |output| {
                    emitted.borrow_mut().push(output);
                    Ok(())
                },
            },
        )
        .expect_err("git-native sync should block when guest HEAD leaves the review branch");

        let AppError::Blocked(message) = err else {
            panic!("expected blocked error, got {err:?}");
        };
        assert!(message.contains("guest HEAD is not on the session review branch"));

        let runtime = find_session(&conn, &session)
            .expect("find runtime session")
            .expect("runtime session");
        let session_ref_head = runtime.session_ref_head.expect("session head ref");
        assert_eq!(
            head_oid_for_ref(&repo, &session_ref_head),
            head_oid(&repo),
            "blocked sync must not advance the hidden session head"
        );
        assert!(
            !bundle_path.exists(),
            "sync should not even create a bundle when the guest is on the wrong branch"
        );

        let json: serde_json::Value =
            serde_json::from_str(emitted.borrow().first().expect("json output")).expect("json");
        assert_eq!(json["blocked"], true);
    }

    #[test]
    fn git_native_sync_blocks_when_guest_head_is_detached() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let conn = open_catalog(&host.state_roots.db).expect("catalog");
        let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("created_at");
        let started_at = Timestamp::parse_rfc3339("2026-04-15T01:06:00Z").expect("started_at");
        let completed_at = Timestamp::parse_rfc3339("2026-04-15T01:07:00Z").expect("completed_at");
        let repo = dir.path().join("host-repo");
        let guest = dir.path().join("guest-repo");

        init_git_repo(&repo);
        fs::write(repo.join("README.md"), "host\n").expect("host readme");
        git_commit_all(&repo, "initial");

        let guest_workspace = GuestPath::new("/home/tester.guest/workspaces/runtime-detached/repo");
        let session = seed_runtime_repo_session(
            &conn,
            "runtime-detached",
            &repo,
            &guest_workspace,
            created_at,
        );
        let review_branch = review_branch_name(&session);
        clone_repo(&repo, &guest);
        git_checkout_branch(&guest, &review_branch);
        let review_head = head_oid(&guest);
        git_checkout_detached(&guest, &review_head);

        let vm_name = VmName::for_session(&session);
        let bundle_path = dir.path().join("guest-sync.bundle");
        let runner =
            GitNativeLimaRunner::new(&vm_name, "Running", &guest_workspace, &guest, &bundle_path);
        let emitted = RefCell::new(Vec::<String>::new());

        let mut clock = vec![started_at, completed_at].into_iter();
        let err = run_with_catalog_at_with(
            SyncBackExecution {
                args: crate::cli::SyncBackArgs {
                    session: crate::cli::SessionSelector::from_session(session.to_string()),
                    yes: true,
                    export_patch: None,
                    json: true,
                },
                host: &host,
                catalog: &conn,
                session_name: &session,
            },
            SyncBackHooks {
                runner: &runner,
                now_fn: move || clock.next().expect("clock tick"),
                emit_outcome_fn: |output| {
                    emitted.borrow_mut().push(output);
                    Ok(())
                },
            },
        )
        .expect_err("git-native sync should block when guest HEAD is detached");

        let AppError::Blocked(message) = err else {
            panic!("expected blocked error, got {err:?}");
        };
        assert!(message.contains("guest HEAD is not on the session review branch"));
        assert!(
            !bundle_path.exists(),
            "detached HEAD should block before bundle export"
        );

        let json: serde_json::Value =
            serde_json::from_str(emitted.borrow().first().expect("json output")).expect("json");
        assert_eq!(json["blocked"], true);
    }

    #[test]
    fn git_native_sync_exports_patch_when_review_branch_diverged() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let conn = open_catalog(&host.state_roots.db).expect("catalog");
        let created_at = Timestamp::parse_rfc3339("2026-04-15T00:00:00Z").expect("created_at");
        let started_at = Timestamp::parse_rfc3339("2026-04-15T01:10:00Z").expect("started_at");
        let completed_at = Timestamp::parse_rfc3339("2026-04-15T01:15:00Z").expect("completed_at");
        let repo = dir.path().join("host-repo");
        let guest = dir.path().join("guest-repo");

        init_git_repo(&repo);
        fs::write(repo.join("README.md"), "host\n").expect("host readme");
        git_commit_all(&repo, "initial");

        let guest_workspace = GuestPath::new("/home/tester.guest/workspaces/runtime-diverged/repo");
        let session = seed_runtime_repo_session(
            &conn,
            "runtime-diverged",
            &repo,
            &guest_workspace,
            created_at,
        );
        let review_branch = review_branch_name(&session);
        let session_head_before = {
            let runtime = find_session(&conn, &session)
                .expect("find runtime session")
                .expect("runtime session");
            head_oid_for_ref(
                &repo,
                runtime
                    .session_ref_head
                    .as_deref()
                    .expect("session head ref"),
            )
        };

        clone_repo(&repo, &guest);
        git_checkout_branch(&guest, &review_branch);
        fs::write(guest.join("README.md"), "guest change\n").expect("guest change");
        git_commit_all(&guest, "guest change");

        git_checkout_branch(&repo, &review_branch);
        fs::write(repo.join("README.md"), "host divergence\n").expect("host divergence");
        git_commit_all(&repo, "host divergence");
        let review_oid_before = head_oid(&repo);
        git_checkout_branch(&repo, "main");
        let patch_path = dir.path().join("blocked.patch");

        let vm_name = VmName::for_session(&session);
        let bundle_path = dir.path().join("guest-sync.bundle");
        let staged_path = host
            .state_roots
            .staging
            .join(format!("{}.bundle", session.as_str()));
        let runner =
            GitNativeLimaRunner::new(&vm_name, "Running", &guest_workspace, &guest, &bundle_path);
        let emitted = RefCell::new(Vec::<String>::new());

        let mut clock = vec![started_at, completed_at].into_iter();
        let err = run_with_catalog_at_with(
            SyncBackExecution {
                args: crate::cli::SyncBackArgs {
                    session: crate::cli::SessionSelector::from_session(session.to_string()),
                    yes: true,
                    export_patch: Some(patch_path.clone()),
                    json: true,
                },
                host: &host,
                catalog: &conn,
                session_name: &session,
            },
            SyncBackHooks {
                runner: &runner,
                now_fn: move || clock.next().expect("clock tick"),
                emit_outcome_fn: |output| {
                    emitted.borrow_mut().push(output);
                    Ok(())
                },
            },
        )
        .expect_err("git-native sync should block when the review branch diverged");

        let AppError::Blocked(message) = err else {
            panic!("expected blocked error, got {err:?}");
        };
        assert!(message.contains("review branch diverged on host"));

        let runtime = find_session(&conn, &session)
            .expect("find runtime session")
            .expect("runtime session");
        let session_ref_head = runtime.session_ref_head.expect("session head ref");
        assert_eq!(
            head_oid_for_ref(&repo, &session_ref_head),
            session_head_before,
            "blocked sync must not advance the hidden session head ref"
        );
        assert_eq!(
            head_oid_for_ref(&repo, &format!("refs/heads/{review_branch}")),
            review_oid_before,
            "blocked sync must not rewrite the diverged review branch"
        );

        let json: serde_json::Value =
            serde_json::from_str(emitted.borrow().first().expect("json output")).expect("json");
        assert_eq!(json["blocked"], true);
        assert_eq!(json["patch_path"], patch_path.to_string_lossy().as_ref());
        assert_eq!(json["staged_path"], staged_path.to_string_lossy().as_ref());
        assert!(
            patch_path.is_file(),
            "blocked sync should export the salvage patch"
        );

        let apply_check = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("apply")
            .arg("--check")
            .arg(&patch_path)
            .status()
            .expect("git apply --check");
        assert!(
            apply_check.success(),
            "blocked sync salvage patch should apply cleanly on the host repo"
        );
    }

    use crate::testing::host_context;

    fn seed_runtime_repo_session(
        conn: &rusqlite::Connection,
        name: &str,
        repo_root: &Path,
        guest_workspace_path: &GuestPath,
        created_at: Timestamp,
    ) -> SessionName {
        let session = SessionName::try_from(name).expect("session");
        let review_branch = review_branch_name(&session);
        let refs = hidden_ref_names(&session);
        let base_oid = head_oid(repo_root);

        git_update_ref(repo_root, &refs.base, &base_oid);
        git_update_ref(repo_root, &refs.head, &base_oid);
        git_update_ref(repo_root, &format!("refs/heads/{review_branch}"), &base_oid);

        insert_session(
            conn,
            &InsertSession {
                name: session.clone(),
                vm_name: VmName::for_session(&session),
                session_mode: SessionMode::Repo,
                repo_sync_mode: Some(RepoSyncMode::GitNative),
                host_context_path: Some(HostPath::new(repo_root)),
                guest_workspace_path: guest_workspace_path.clone(),
                seed_host_path: None,
                host_git_root: Some(HostPath::new(repo_root)),
                host_head_oid_at_open: Some(base_oid.clone()),
                host_head_ref_at_open: Some("refs/heads/main".to_owned()),
                host_dirty_at_open: false,
                base_ref: Some("refs/heads/main".to_owned()),
                review_branch: Some(review_branch),
                session_ref_base: Some(refs.base),
                session_ref_head: Some(refs.head),
                provider_kind: None,
                imported_provider_files_json: "[]".to_owned(),
                guest_tmux_socket_path: None,
                shell_window_name: None,
                agent_window_name: None,
                agent_launch_preset: None,
                created_at,
            },
        )
        .expect("insert session");
        update_lifecycle_state_with_timestamps(
            conn,
            &session,
            LifecycleState::Running,
            &created_at,
            Some(&created_at),
            None,
            None,
        )
        .expect("update lifecycle state");
        update_sync_state(conn, &session, SyncState::Pending, &created_at)
            .expect("update sync state");
        session
    }

    fn git_update_ref(path: &Path, reference: &str, oid: &str) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("update-ref")
            .arg(reference)
            .arg(oid)
            .status()
            .expect("git update-ref");
        assert!(status.success(), "git update-ref should succeed");
    }

    fn clone_repo(source: &Path, destination: &Path) {
        let status = Command::new("git")
            .arg("clone")
            .arg("--quiet")
            .arg(source)
            .arg(destination)
            .status()
            .expect("git clone");
        assert!(status.success(), "git clone should succeed");
    }

    fn git_checkout_branch(path: &Path, branch: &str) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("checkout")
            .arg(branch)
            .status()
            .expect("git checkout");
        assert!(status.success(), "git checkout should succeed");
    }

    fn git_checkout_new_branch(path: &Path, branch: &str) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("checkout")
            .arg("-b")
            .arg(branch)
            .status()
            .expect("git checkout -b");
        assert!(status.success(), "git checkout -b should succeed");
    }

    fn git_checkout_detached(path: &Path, reference: &str) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("checkout")
            .arg("--detach")
            .arg(reference)
            .status()
            .expect("git checkout --detach");
        assert!(status.success(), "git checkout --detach should succeed");
    }

    fn init_git_repo(path: &Path) {
        fs::create_dir_all(path).expect("repo dir");
        let status = Command::new("git")
            .arg("init")
            .arg("-b")
            .arg("main")
            .arg(path)
            .status()
            .expect("git init");
        assert!(status.success(), "git init should succeed");
    }

    fn git_commit_all(path: &Path, message: &str) {
        let add = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("add")
            .arg(".")
            .status()
            .expect("git add");
        assert!(add.success(), "git add should succeed");

        let commit = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("-c")
            .arg("user.name=agbranch-tests")
            .arg("-c")
            .arg("user.email=agbranch@example.invalid")
            .arg("commit")
            .arg("-m")
            .arg(message)
            .status()
            .expect("git commit");
        assert!(commit.success(), "git commit should succeed");
    }

    fn head_oid(path: &Path) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("rev-parse")
            .arg("HEAD")
            .output()
            .expect("git rev-parse");
        assert!(output.status.success(), "git rev-parse should succeed");
        String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_owned()
    }

    fn head_oid_for_ref(path: &Path, reference: &str) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("rev-parse")
            .arg(reference)
            .output()
            .expect("git rev-parse ref");
        assert!(output.status.success(), "git rev-parse should succeed");
        String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_owned()
    }
}
