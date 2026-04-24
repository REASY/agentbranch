use crate::cli::LaunchArgs;
use crate::commands::agent::{
    SessionOwnedAgentLaunch, auth_prompt_enabled, start_session_owned_agent_with,
};
use crate::commands::session_slot::ensure_runtime_session_slot_available;
use crate::db::connect::open_catalog;
use crate::db::locks::SessionLock;
use crate::db::models::{AgentLaunchPreset, LifecycleState, SessionMode};
use crate::db::sessions::{
    InsertSession, insert_session, update_agent_metadata, update_lifecycle_state_with_timestamps,
};
use crate::error::{AppError, ValidationError};
use crate::lima::{
    base,
    client::{LimaClient, LimactlClient},
    instance,
};
use crate::platform::host::HostContext;
use crate::session::guest_support;
use crate::session::orchestration::{LockMetadataGuard, SessionGuard, run_step};
use crate::session::paths::{sandbox_workspace_path, tmux_socket_path};
use crate::types::{GuestPath, HostPath, ProviderKind, SessionName};
use crate::util::ids::{prepared_base_name, session_vm_name};
use crate::util::process::RealCommandRunner;
use crate::util::time::utc_now;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::Path;
use std::time::Instant;

pub fn guest_sandbox_workspace(host_home_dir: &Path, session: &SessionName) -> GuestPath {
    sandbox_workspace_path(host_home_dir, session)
}

pub fn build_launch_record(
    host_home_dir: &Path,
    session: &SessionName,
    seed: Option<&HostPath>,
    provider: Option<ProviderKind>,
) -> InsertSession {
    InsertSession {
        name: session.clone(),
        vm_name: session_vm_name(session),
        session_mode: SessionMode::Sandbox,
        repo_sync_mode: None,
        host_context_path: None,
        guest_workspace_path: guest_sandbox_workspace(host_home_dir, session),
        seed_host_path: seed.cloned(),
        host_git_root: None,
        host_head_oid_at_open: None,
        host_head_ref_at_open: None,
        host_dirty_at_open: false,
        base_ref: None,
        review_branch: None,
        session_ref_base: None,
        session_ref_head: None,
        provider_kind: provider,
        imported_provider_files_json: "[]".to_owned(),
        guest_tmux_socket_path: Some(tmux_socket_path(host_home_dir, session)),
        shell_window_name: Some("shell".to_owned()),
        agent_window_name: Some("agent".to_owned()),
        agent_launch_preset: provider.map(|_| AgentLaunchPreset::Unrestricted),
        created_at: utc_now(),
    }
}

pub fn run(args: LaunchArgs) -> Result<(), AppError> {
    let session_name = SessionName::try_from(args.session.as_str())?;
    let provider = args.agent.as_deref().and_then(ProviderKind::parse);
    let seed = args.seed.as_ref().map(HostPath::new);
    let host = HostContext::detect()?;
    std::fs::create_dir_all(&host.state_roots.locks)?;
    let lock_path = host
        .state_roots
        .locks
        .join(format!("{}.lock", session_name));
    let _lock = SessionLock::acquire(&lock_path, std::process::id(), "launch")?;

    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    let launch_started = Instant::now();
    let catalog = open_catalog(&host.state_roots.db)?;
    let vm_name = session_vm_name(&session_name);
    let workspace = guest_sandbox_workspace(&host.home_dir, &session_name);
    let now = utc_now();
    let record = build_launch_record(&host.home_dir, &session_name, seed.as_ref(), provider);
    ensure_runtime_session_slot_available(&catalog, &session_name, &vm_name)?;

    insert_session(&catalog, &record)?;
    let lock_guard =
        LockMetadataGuard::acquire(&catalog, &session_name, std::process::id(), "launch")?;

    let guard = SessionGuard::launch(&runner, &catalog, &session_name, &vm_name);

    let result: Result<(), AppError> = (|| {
        run_step(
            &session_name,
            "launch",
            "prepare-base",
            &launch_started,
            || Ok(ensure_prepared_base(&runner, host.platform, false)?),
        )?;
        run_step(&session_name, "launch", "clone-vm", &launch_started, || {
            Ok(lima.clone_instance(
                &prepared_base_name(host.platform),
                &vm_name,
                args.cpus,
                args.memory.as_ref(),
                args.disk.as_ref(),
            )?)
        })?;
        run_step(&session_name, "launch", "start-vm", &launch_started, || {
            Ok(lima.start_instance(&vm_name)?)
        })?;
        run_step(
            &session_name,
            "launch",
            "install-guest-support",
            &launch_started,
            || {
                Ok(guest_support::install_guest_support_files(
                    &lima,
                    &vm_name,
                    &host.home_dir,
                )?)
            },
        )?;
        run_step(
            &session_name,
            "launch",
            "ensure-shell",
            &launch_started,
            || {
                Ok(guest_support::ensure_workspace_and_shell(
                    &lima,
                    &vm_name,
                    &session_name,
                    record
                        .guest_tmux_socket_path
                        .as_ref()
                        .expect("launch record should include tmux socket"),
                    &workspace,
                )?)
            },
        )?;
        if let Some(seed) = seed.as_ref() {
            run_step(
                &session_name,
                "launch",
                "seed-workspace",
                &launch_started,
                || Ok(lima.copy_host_path_to_guest(seed, &vm_name, &workspace)?),
            )?;
        }
        update_lifecycle_state_with_timestamps(
            &catalog,
            &session_name,
            LifecycleState::Running,
            &now,
            Some(&now),
            None,
            None,
        )?;
        if let Some(provider) = provider {
            let imported = run_step(
                &session_name,
                "launch",
                "launch-agent",
                &launch_started,
                || {
                    start_session_owned_agent_with(
                        &lima,
                        SessionOwnedAgentLaunch {
                            session_name: &session_name,
                            vm_name: &vm_name,
                            workspace: &workspace,
                            host_home: &host.home_dir,
                            provider,
                            shell_window_name: "shell",
                            agent_window_name: "agent",
                        },
                        host.platform,
                        &std::env::vars().collect::<BTreeMap<_, _>>(),
                        auth_prompt_enabled(
                            args.json,
                            std::io::stdin().is_terminal(),
                            std::io::stdout().is_terminal(),
                        ),
                        &crate::provider::auth::TerminalAuthPrompter,
                    )
                },
            )?;
            let imported_json = serde_json::to_string(&imported).map_err(|err| {
                AppError::Validation(ValidationError::StepFailed {
                    step: "launch-agent",
                    detail: format!("failed to serialize agent metadata: {err}"),
                })
            })?;
            update_agent_metadata(
                &catalog,
                &session_name,
                provider,
                &imported_json,
                AgentLaunchPreset::Unrestricted,
                &now,
            )?;
        }
        run_step(&session_name, "launch", "finalize", &launch_started, || {
            Ok(())
        })?;
        Ok(())
    })();

    match result {
        Ok(()) => guard.commit(),
        Err(err) => return Err(guard.rollback(err)),
    }

    lock_guard.commit()?;

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "session": session_name,
                "vm_name": vm_name,
                "lifecycle_state": "running",
                "guest_workspace_path": workspace,
            })
        );
    } else if provider.is_some() {
        crate::commands::attach::run(crate::cli::AttachArgs {
            session: crate::cli::SessionSelector::from_session(args.session),
            shell: false,
            agent: true,
            json: false,
        })?;
    }

    Ok(())
}

fn ensure_prepared_base(
    runner: &RealCommandRunner,
    platform: crate::platform::detect::HostPlatform,
    rebuild: bool,
) -> Result<(), crate::error::lima::LimaError> {
    let base_name = crate::util::ids::prepared_base_name(platform);
    let instances = instance::list_instances(runner)?;
    if let Some(existing) = instances
        .iter()
        .find(|item| item.name == base_name.as_str())
        && !base::prepared_base_requires_rebuild(existing)
    {
        return Ok(());
    }
    let _ = base::prepare_base(runner, platform, rebuild)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::SessionMode;
    use crate::types::{HostPath, ProviderKind, SessionName};
    use std::path::Path;

    #[test]
    fn sandbox_workspace_lives_under_guest_sandbox_root() {
        let session = SessionName::try_from("research").expect("session");
        let workspace = guest_sandbox_workspace(Path::new("/Users/tester"), &session);
        assert_eq!(
            workspace.as_path(),
            Path::new("/home/tester.guest/sandbox/research")
        );
    }

    #[test]
    fn launch_record_marks_sandbox_mode() {
        let session = SessionName::try_from("research").expect("session");
        let record = build_launch_record(
            Path::new("/Users/tester"),
            &session,
            None,
            Some(ProviderKind::Codex),
        );
        assert_eq!(record.session_mode, SessionMode::Sandbox);
        assert_eq!(record.provider_kind, Some(ProviderKind::Codex));
        assert_eq!(
            record.guest_workspace_path.as_path(),
            Path::new("/home/tester.guest/sandbox/research")
        );
    }

    #[test]
    fn launch_record_keeps_seed_path_out_of_repo_metadata() {
        let session = SessionName::try_from("research-seed").expect("session");
        let seed = HostPath::new("/tmp/research-seed");
        let record = build_launch_record(Path::new("/Users/tester"), &session, Some(&seed), None);

        assert_eq!(record.session_mode, SessionMode::Sandbox);
        assert_eq!(record.seed_host_path.as_ref(), Some(&seed));
        assert_eq!(record.host_context_path, None);
    }
}
