use crate::cli::RetryArgs;
use crate::commands::agent::{
    AuthImportRequest, SessionOwnedAgentLaunch, auth_prompt_enabled,
    detect_and_resolve_auth_imports, start_session_owned_agent_with,
};
use crate::commands::open::{configure_guest_identity, create_git_seed_clone};
use crate::commands::{find_existing_session, resolve_session_name};
use crate::db::connect::open_catalog;
use crate::db::launch_retries::{delete_launch_retry, find_launch_retry, save_launch_error};
use crate::db::locks::SessionLock;
use crate::db::models::{AgentLaunchPreset, LifecycleState, SessionMode};
use crate::db::ports::list_session_ports;
use crate::db::sessions::{update_agent_metadata, update_lifecycle_state_with_timestamps};
use crate::error::{AppError, ValidationError};
use crate::git::identity::detect_identity;
use crate::lima::client::{LimaClient, LimactlClient};
use crate::lima::instance;
use crate::platform::host::HostContext;
use crate::policy::artifacts::{ArtifactPolicy, FilteredSeedTree};
use crate::session::guest_support;
use crate::session::launch_retry::{
    AGENT_STARTED, GIT_IDENTITY_CONFIGURED, GUEST_SUPPORT_INSTALLED, PORTS_CONFIGURED, SHELL_READY,
    VM_CLONED, VM_STARTED, WORKSPACE_SEEDED, checkpoint, remaining_stages,
};
use crate::session::orchestration::{LockMetadataGuard, OperationTimings, run_step};
use crate::types::{HostPath, SessionName, VmName};
use crate::util::process::RealCommandRunner;
use crate::util::time::utc_now;
use std::collections::BTreeMap;
use std::io::IsTerminal;

const SANDBOX_STAGES: &[&str] = &[
    VM_CLONED,
    PORTS_CONFIGURED,
    VM_STARTED,
    GUEST_SUPPORT_INSTALLED,
    SHELL_READY,
    WORKSPACE_SEEDED,
    AGENT_STARTED,
];

const REPO_STAGES: &[&str] = &[
    VM_CLONED,
    PORTS_CONFIGURED,
    VM_STARTED,
    GUEST_SUPPORT_INSTALLED,
    WORKSPACE_SEEDED,
    SHELL_READY,
    GIT_IDENTITY_CONFIGURED,
    AGENT_STARTED,
];

pub fn run(args: RetryArgs) -> Result<(), AppError> {
    let (raw_session_name, session_name) = resolve_session_name(&args.session)?;
    let host = HostContext::detect()?;
    std::fs::create_dir_all(&host.state_roots.locks)?;
    let lock_path = host
        .state_roots
        .locks
        .join(format!("{}.lock", session_name));
    let _lock = SessionLock::acquire(&lock_path, std::process::id(), "retry")?;
    let catalog = open_catalog(&host.state_roots.db)?;
    let session = find_existing_session(&catalog, &session_name, &raw_session_name)?;
    let retry = find_launch_retry(&catalog, &session_name)?.ok_or_else(|| {
        ValidationError::LaunchRetryUnavailable {
            name: session_name.to_string(),
        }
    })?;
    let stages = stages_for_mode(session.session_mode);
    let remaining = remaining_stages(stages, &retry.checkpoint).ok_or_else(|| {
        ValidationError::InvalidLaunchCheckpoint {
            name: session_name.to_string(),
            checkpoint: retry.checkpoint.clone(),
        }
    })?;

    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    ensure_vm_exists(&runner, &session_name, &session.vm_name)?;
    let lima_assets = crate::lima::asset_resolver::lima_asset_dir(&host)?;
    let ports = list_session_ports(&catalog, &session_name)?;
    let timings = OperationTimings::start();
    let lock_guard =
        LockMetadataGuard::acquire(&catalog, &session_name, std::process::id(), "retry")?;
    let resumed_from = retry.checkpoint.clone();

    let result: Result<(), AppError> = (|| {
        if stage_is_pending(remaining, PORTS_CONFIGURED) {
            if !ports.is_empty() {
                run_step(&session_name, "retry", "configure-ports", &timings, || {
                    Ok(lima.configure_port_forwards(&session.vm_name, &ports)?)
                })?;
            }
            checkpoint(&catalog, &session_name, PORTS_CONFIGURED)?;
        }

        if stage_is_pending(remaining, VM_STARTED)
            || !instance_is_running(&runner, &session.vm_name)?
        {
            run_step(&session_name, "retry", "start-vm", &timings, || {
                if instance_is_running(&runner, &session.vm_name)? {
                    return Ok(());
                }
                Ok(lima.start_instance(&session.vm_name)?)
            })?;
            if stage_is_pending(remaining, VM_STARTED) {
                checkpoint(&catalog, &session_name, VM_STARTED)?;
            }
        }
        update_lifecycle_state_with_timestamps(
            &catalog,
            &session_name,
            LifecycleState::Seeding,
            &utc_now(),
            None,
            None,
            None,
        )?;

        if stage_is_pending(remaining, GUEST_SUPPORT_INSTALLED) {
            run_step(
                &session_name,
                "retry",
                "install-guest-support",
                &timings,
                || {
                    Ok(guest_support::install_guest_support_files(
                        &lima,
                        &session.vm_name,
                        &host.home_dir,
                        &lima_assets.path,
                    )?)
                },
            )?;
            checkpoint(&catalog, &session_name, GUEST_SUPPORT_INSTALLED)?;
        }

        match session.session_mode {
            SessionMode::Sandbox => retry_sandbox_stages(
                &catalog,
                &lima,
                &session_name,
                &session,
                remaining,
                &timings,
            )?,
            SessionMode::Repo => retry_repo_stages(
                &catalog,
                &runner,
                &lima,
                &host,
                &session_name,
                &session,
                remaining,
                &timings,
            )?,
        }

        if stage_is_pending(remaining, AGENT_STARTED) {
            retry_agent_stage(
                &catalog,
                &lima,
                &host,
                &session_name,
                &session,
                args.json,
                &timings,
            )?;
            checkpoint(&catalog, &session_name, AGENT_STARTED)?;
        }

        run_step(&session_name, "retry", "finalize", &timings, || {
            let now = utc_now();
            update_lifecycle_state_with_timestamps(
                &catalog,
                &session_name,
                LifecycleState::Running,
                &now,
                Some(&now),
                None,
                None,
            )?;
            delete_launch_retry(&catalog, &session_name)?;
            Ok(())
        })?;
        Ok(())
    })();

    if let Err(err) = result {
        save_launch_error(&catalog, &session_name, &err.to_string(), &utc_now())?;
        lock_guard.commit()?;
        return Err(AppError::Blocked(format!(
            "retry for session `{session_name}` failed: {err}\n\
             fix the cause and run `agbranch retry {session_name}` again"
        )));
    }

    lock_guard.commit()?;
    let summary = timings.summary();
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "session": session_name,
                "status": "running",
                "resumed_from": resumed_from,
                "timings": summary,
            })
        );
    } else {
        eprintln!(
            "retry {session_name}: resumed from {resumed_from}\n{}",
            summary.render_human("retry", &session_name)
        );
    }
    Ok(())
}

fn retry_sandbox_stages(
    catalog: &rusqlite::Connection,
    lima: &dyn LimaClient,
    session_name: &SessionName,
    session: &crate::db::sessions::SessionRow,
    remaining: &[&str],
    timings: &OperationTimings,
) -> Result<(), AppError> {
    if stage_is_pending(remaining, SHELL_READY) {
        run_step(session_name, "retry", "ensure-shell", timings, || {
            Ok(guest_support::ensure_workspace_and_shell(
                lima,
                &session.vm_name,
                session_name,
                session
                    .guest_tmux_socket_path
                    .as_ref()
                    .ok_or(ValidationError::SessionMissingTmuxSocket)?,
                &session.guest_workspace_path,
            )?)
        })?;
        checkpoint(catalog, session_name, SHELL_READY)?;
    }
    if stage_is_pending(remaining, WORKSPACE_SEEDED) {
        if let Some(seed) = session.seed_host_path.as_ref() {
            run_step(session_name, "retry", "seed-workspace", timings, || {
                let policy = ArtifactPolicy::load(seed.as_path())?;
                let filtered = FilteredSeedTree::materialize(seed.as_path(), &policy)?;
                Ok(lima.copy_host_path_to_guest(
                    &HostPath::new(filtered.path()),
                    &session.vm_name,
                    &session.guest_workspace_path,
                )?)
            })?;
        }
        checkpoint(catalog, session_name, WORKSPACE_SEEDED)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn retry_repo_stages(
    catalog: &rusqlite::Connection,
    runner: &RealCommandRunner,
    lima: &dyn LimaClient,
    host: &HostContext,
    session_name: &SessionName,
    session: &crate::db::sessions::SessionRow,
    remaining: &[&str],
    timings: &OperationTimings,
) -> Result<(), AppError> {
    if stage_is_pending(remaining, WORKSPACE_SEEDED) {
        let git_root = session
            .host_git_root
            .as_ref()
            .ok_or(ValidationError::OpenRequiresGitRepo)?;
        let base_ref = session
            .base_ref
            .as_deref()
            .ok_or(ValidationError::OpenRequiresGitRepo)?;
        let review_branch = session
            .review_branch
            .as_deref()
            .ok_or(ValidationError::OpenRequiresGitRepo)?;
        let seed_clone = run_step(session_name, "retry", "seed-repo-clone", timings, || {
            create_git_seed_clone(
                runner,
                &host.state_roots.staging,
                git_root,
                base_ref,
                review_branch,
            )
        })?;
        run_step(session_name, "retry", "seed-repo", timings, || {
            Ok(lima.seed_repo(
                seed_clone.path(),
                &session.vm_name,
                &session.guest_workspace_path,
            )?)
        })?;
        checkpoint(catalog, session_name, WORKSPACE_SEEDED)?;
    }
    if stage_is_pending(remaining, SHELL_READY) {
        run_step(session_name, "retry", "ensure-shell", timings, || {
            Ok(guest_support::ensure_workspace_and_shell(
                lima,
                &session.vm_name,
                session_name,
                session
                    .guest_tmux_socket_path
                    .as_ref()
                    .ok_or(ValidationError::SessionMissingTmuxSocket)?,
                &session.guest_workspace_path,
            )?)
        })?;
        checkpoint(catalog, session_name, SHELL_READY)?;
    }
    if stage_is_pending(remaining, GIT_IDENTITY_CONFIGURED) {
        let git_root = session
            .host_git_root
            .as_ref()
            .ok_or(ValidationError::OpenRequiresGitRepo)?;
        let identity = detect_identity(runner, git_root.as_path())?
            .ok_or(ValidationError::OpenRequiresGitIdentity)?;
        run_step(
            session_name,
            "retry",
            "configure-git-identity",
            timings,
            || {
                configure_guest_identity(
                    runner,
                    &session.vm_name,
                    &session.guest_workspace_path,
                    &identity,
                )
            },
        )?;
        checkpoint(catalog, session_name, GIT_IDENTITY_CONFIGURED)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn retry_agent_stage(
    catalog: &rusqlite::Connection,
    lima: &dyn LimaClient,
    host: &HostContext,
    session_name: &SessionName,
    session: &crate::db::sessions::SessionRow,
    json: bool,
    timings: &OperationTimings,
) -> Result<(), AppError> {
    let Some(provider) = session.provider_kind else {
        return Ok(());
    };
    let host_env = std::env::vars().collect::<BTreeMap<_, _>>();
    let selected_auth = detect_and_resolve_auth_imports(
        catalog,
        AuthImportRequest {
            provider,
            host_platform: host.platform,
            host_home: &host.home_dir,
            host_env: &host_env,
            requested_mode: None,
            interactive: auth_prompt_enabled(
                json,
                std::io::stdin().is_terminal(),
                std::io::stdout().is_terminal(),
            ),
        },
        &crate::provider::auth::TerminalAuthPrompter,
    )?;
    let imported = run_step(session_name, "retry", "launch-agent", timings, || {
        start_session_owned_agent_with(
            lima,
            SessionOwnedAgentLaunch {
                session_name,
                vm_name: &session.vm_name,
                workspace: &session.guest_workspace_path,
                host_home: &host.home_dir,
                provider,
                shell_window_name: session.shell_window_name.as_deref().unwrap_or("shell"),
                agent_window_name: session.agent_window_name.as_deref().unwrap_or("agent"),
            },
            &selected_auth,
        )
    })?;
    let imported_json = serde_json::to_string(&imported)
        .map_err(crate::error::observability::ObservabilityError::from)?;
    update_agent_metadata(
        catalog,
        session_name,
        provider,
        &imported_json,
        AgentLaunchPreset::Unrestricted,
        &utc_now(),
    )?;
    Ok(())
}

fn stages_for_mode(mode: SessionMode) -> &'static [&'static str] {
    match mode {
        SessionMode::Sandbox => SANDBOX_STAGES,
        SessionMode::Repo => REPO_STAGES,
    }
}

fn stage_is_pending(remaining: &[&str], stage: &str) -> bool {
    remaining.contains(&stage)
}

fn ensure_vm_exists(
    runner: &RealCommandRunner,
    session: &SessionName,
    vm: &VmName,
) -> Result<(), AppError> {
    if instance::list_instances(runner)?
        .iter()
        .any(|instance| instance.name == vm.as_str())
    {
        return Ok(());
    }
    Err(AppError::Blocked(format!(
        "cannot retry session `{session}` because VM `{vm}` no longer exists; \
         close the session and launch it again"
    )))
}

fn instance_is_running(runner: &RealCommandRunner, vm: &VmName) -> Result<bool, AppError> {
    Ok(instance::list_instances(runner)?
        .iter()
        .find(|instance| instance.name == vm.as_str())
        .is_some_and(|instance| instance.is_running()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_plan_preserves_shell_before_seed_order() {
        assert_eq!(
            remaining_stages(SANDBOX_STAGES, GUEST_SUPPORT_INSTALLED),
            Some(&[SHELL_READY, WORKSPACE_SEEDED, AGENT_STARTED][..])
        );
    }

    #[test]
    fn repo_plan_preserves_seed_before_shell_order() {
        assert_eq!(
            remaining_stages(REPO_STAGES, GUEST_SUPPORT_INSTALLED),
            Some(
                &[
                    WORKSPACE_SEEDED,
                    SHELL_READY,
                    GIT_IDENTITY_CONFIGURED,
                    AGENT_STARTED,
                ][..]
            )
        );
    }

    #[test]
    fn completed_stage_is_not_pending() {
        let remaining = remaining_stages(SANDBOX_STAGES, VM_STARTED).expect("checkpoint");
        assert!(!stage_is_pending(remaining, VM_STARTED));
        assert!(stage_is_pending(remaining, SHELL_READY));
    }
}
