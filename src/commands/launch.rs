use crate::cli::LaunchArgs;
use crate::commands::agent::{
    AuthImportRequest, SessionOwnedAgentLaunch, auth_prompt_enabled,
    detect_and_resolve_auth_imports, start_session_owned_agent_with,
};
use crate::commands::base::{acquire_clone_lock_for_prepared_base, emit_prepared_base_notice};
use crate::commands::session_slot::ensure_runtime_session_slot_available;
use crate::db::connect::open_catalog;
use crate::db::launch_retries::delete_launch_retry;
use crate::db::locks::SessionLock;
use crate::db::models::{AgentLaunchPreset, LifecycleState, SessionMode};
use crate::db::ports::insert_session_ports;
use crate::db::sessions::{
    InsertSession, insert_session, update_agent_metadata, update_lifecycle_state_with_timestamps,
};
use crate::error::{AppError, ValidationError};
use crate::lima::client::{LimaClient, LimactlClient};
use crate::platform::host::HostContext;
use crate::policy::artifacts::{ArtifactPolicy, FilteredSeedTree};
use crate::ports::{PublishedPort, validate_published_ports};
use crate::session::guest_support;
use crate::session::launch_retry::{
    AGENT_STARTED, GUEST_SUPPORT_INSTALLED, PORTS_CONFIGURED, SHELL_READY, VM_CLONED, VM_STARTED,
    WORKSPACE_SEEDED, checkpoint, preserve_failure,
};
use crate::session::orchestration::{
    LockMetadataGuard, OperationTimings, SessionGuard, TimingSummary, run_step,
};
use crate::session::paths::{sandbox_workspace_path, tmux_socket_path};
use crate::types::{GuestPath, HostPath, ProviderKind, SessionName};
use crate::util::ids::{prepared_base_name, session_vm_name};
use crate::util::process::RealCommandRunner;
use crate::util::time::utc_now;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::Path;

pub fn guest_sandbox_workspace(host_home_dir: &Path, session: &SessionName) -> GuestPath {
    sandbox_workspace_path(host_home_dir, session)
}

pub fn render_launch_json(
    session: &SessionName,
    vm_name: &crate::types::VmName,
    workspace: &GuestPath,
    published_ports: &[PublishedPort],
    timings: &TimingSummary,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&serde_json::json!({
        "session": session,
        "vm_name": vm_name,
        "lifecycle_state": "running",
        "guest_workspace_path": workspace,
        "published_ports": published_ports,
        "timings": timings,
    }))
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
    validate_published_ports(&args.publish).map_err(ValidationError::InvalidPublishedPorts)?;
    let seed = args.seed.as_ref().map(HostPath::new);
    let host = HostContext::detect()?;
    let lima_assets = crate::lima::asset_resolver::lima_asset_dir(&host)?;
    std::fs::create_dir_all(&host.state_roots.locks)?;
    let lock_path = host
        .state_roots
        .locks
        .join(format!("{}.lock", session_name));
    let _lock = SessionLock::acquire(&lock_path, std::process::id(), "launch")?;

    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    let timings = OperationTimings::start();
    let mut catalog = open_catalog(&host.state_roots.db)?;
    let vm_name = session_vm_name(&session_name);
    let workspace = guest_sandbox_workspace(&host.home_dir, &session_name);
    let now = utc_now();
    let record = build_launch_record(&host.home_dir, &session_name, seed.as_ref(), provider);
    ensure_runtime_session_slot_available(&catalog, &session_name, &vm_name)?;
    let host_env = std::env::vars().collect::<BTreeMap<_, _>>();
    let selected_auth = provider
        .map(|provider| {
            detect_and_resolve_auth_imports(
                &catalog,
                AuthImportRequest {
                    provider,
                    host_platform: host.platform,
                    host_home: &host.home_dir,
                    host_env: &host_env,
                    requested_mode: args.auth,
                    interactive: auth_prompt_enabled(
                        args.json,
                        std::io::stdin().is_terminal(),
                        std::io::stdout().is_terminal(),
                    ),
                },
                &crate::provider::auth::TerminalAuthPrompter,
            )
        })
        .transpose()?;

    {
        let tx = catalog
            .transaction()
            .map_err(crate::error::db::DbError::from)?;
        insert_session(&tx, &record)?;
        insert_session_ports(&tx, &session_name, &args.publish)?;
        tx.commit().map_err(crate::error::db::DbError::from)?;
    }
    let lock_guard =
        LockMetadataGuard::acquire(&catalog, &session_name, std::process::id(), "launch")?;

    let guard = SessionGuard::launch(&runner, &catalog, &session_name, &vm_name);

    let mut retryable = false;
    let result: Result<(), AppError> = (|| {
        let base_clone_lock = run_step(&session_name, "launch", "prepare-base", &timings, || {
            let mut on_notice =
                |notice| emit_prepared_base_notice(&catalog, &session_name, &notice);
            acquire_clone_lock_for_prepared_base(
                &runner,
                &host,
                "launch clone",
                "launch prepare-base",
                &mut on_notice,
            )
        })?;
        run_step(&session_name, "launch", "clone-vm", &timings, || {
            Ok(lima.clone_instance(
                &prepared_base_name(host.platform),
                &vm_name,
                args.cpus,
                args.memory.as_ref(),
                args.disk.as_ref(),
            )?)
        })?;
        drop(base_clone_lock);
        checkpoint(&catalog, &session_name, VM_CLONED)?;
        retryable = true;
        update_lifecycle_state_with_timestamps(
            &catalog,
            &session_name,
            LifecycleState::Starting,
            &utc_now(),
            None,
            None,
            None,
        )?;
        if !args.publish.is_empty() {
            run_step(&session_name, "launch", "configure-ports", &timings, || {
                Ok(lima.configure_port_forwards(&vm_name, &args.publish)?)
            })?;
        }
        checkpoint(&catalog, &session_name, PORTS_CONFIGURED)?;
        run_step(&session_name, "launch", "start-vm", &timings, || {
            Ok(lima.start_instance(&vm_name)?)
        })?;
        checkpoint(&catalog, &session_name, VM_STARTED)?;
        update_lifecycle_state_with_timestamps(
            &catalog,
            &session_name,
            LifecycleState::Seeding,
            &utc_now(),
            None,
            None,
            None,
        )?;
        run_step(
            &session_name,
            "launch",
            "install-guest-support",
            &timings,
            || {
                Ok(guest_support::install_guest_support_files(
                    &lima,
                    &vm_name,
                    &host.home_dir,
                    &lima_assets.path,
                )?)
            },
        )?;
        checkpoint(&catalog, &session_name, GUEST_SUPPORT_INSTALLED)?;
        run_step(&session_name, "launch", "ensure-shell", &timings, || {
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
        })?;
        checkpoint(&catalog, &session_name, SHELL_READY)?;
        if let Some(seed) = seed.as_ref() {
            run_step(&session_name, "launch", "seed-workspace", &timings, || {
                let policy = ArtifactPolicy::load(seed.as_path())?;
                let filtered = FilteredSeedTree::materialize(seed.as_path(), &policy)?;
                Ok(lima.copy_host_path_to_guest(
                    &HostPath::new(filtered.path()),
                    &vm_name,
                    &workspace,
                )?)
            })?;
        }
        checkpoint(&catalog, &session_name, WORKSPACE_SEEDED)?;
        if let Some(provider) = provider {
            let imported = run_step(&session_name, "launch", "launch-agent", &timings, || {
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
                    selected_auth.as_deref().unwrap_or_default(),
                )
            })?;
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
        checkpoint(&catalog, &session_name, AGENT_STARTED)?;
        run_step(&session_name, "launch", "finalize", &timings, || {
            let finished_at = utc_now();
            update_lifecycle_state_with_timestamps(
                &catalog,
                &session_name,
                LifecycleState::Running,
                &finished_at,
                Some(&finished_at),
                None,
                None,
            )?;
            delete_launch_retry(&catalog, &session_name)?;
            Ok(())
        })?;
        Ok(())
    })();

    match result {
        Ok(()) => guard.commit(),
        Err(err) if retryable => {
            let retry_err = preserve_failure(&catalog, &session_name, &err);
            guard.preserve();
            lock_guard.commit()?;
            return Err(retry_err?);
        }
        Err(err) => return Err(guard.rollback(err)),
    }

    lock_guard.commit()?;
    let timing_summary = timings.summary();

    if args.json {
        println!(
            "{}",
            render_launch_json(
                &session_name,
                &vm_name,
                &workspace,
                &args.publish,
                &timing_summary,
            )
            .map_err(|err| AppError::Validation(ValidationError::StepFailed {
                step: "finalize",
                detail: format!("failed to serialize json output: {err}"),
            }))?
        );
    } else {
        eprintln!("{}", timing_summary.render_human("launch", &session_name));
        if provider.is_some() {
            crate::commands::attach::run(crate::cli::AttachArgs {
                session: crate::cli::SessionSelector::from_session(args.session),
                shell: false,
                agent: true,
                json: false,
            })?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::SessionMode;
    use crate::session::orchestration::{PhaseTiming, TimingSummary};
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

    #[test]
    fn launch_json_contains_structured_timings() {
        let session = SessionName::try_from("research").expect("session");
        let vm_name = crate::types::VmName::new("agbranch-research");
        let workspace = GuestPath::new("/home/tester.guest/sandbox/research");
        let timings = TimingSummary {
            total_ms: 14_250,
            phases: vec![PhaseTiming {
                name: "start-vm".to_owned(),
                duration_ms: 12_000,
            }],
            slowest_phase: Some(PhaseTiming {
                name: "start-vm".to_owned(),
                duration_ms: 12_000,
            }),
        };

        let published_ports = vec!["8080:3000".parse().expect("port")];
        let rendered =
            render_launch_json(&session, &vm_name, &workspace, &published_ports, &timings)
                .expect("json");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid json");
        assert_eq!(value["session"], "research");
        assert_eq!(value["timings"]["total_ms"], 14_250);
        assert_eq!(value["timings"]["phases"][0]["duration_ms"], 12_000);
        assert_eq!(value["timings"]["slowest_phase"]["name"], "start-vm");
        assert_eq!(value["published_ports"][0]["host_port"], 8080);
    }
}
