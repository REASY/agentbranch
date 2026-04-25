use crate::cli::OpenArgs;
use crate::commands::agent::{
    SessionOwnedAgentLaunch, auth_prompt_enabled, start_session_owned_agent_with,
};
use crate::commands::base::{acquire_clone_lock_for_prepared_base, emit_prepared_base_notice};
use crate::commands::session_slot::ensure_runtime_session_slot_available;
use crate::db::connect::open_catalog;
use crate::db::locks::SessionLock;
use crate::db::models::{AgentLaunchPreset, RepoSyncMode, SessionMode};
use crate::db::sessions::{
    InsertSession, insert_session, update_agent_metadata, update_lifecycle_state_with_timestamps,
};
use crate::error::{AppError, ValidationError};
use crate::git::baseline::capture_repo_baseline;
use crate::git::identity::{GitIdentity, detect_identity};
use crate::git::session_refs::{
    hidden_ref_names, initialize_session_refs, resolve_base_ref, resolve_ref_oid,
    review_branch_name,
};
use crate::lima::client::{LimaClient, LimactlClient};
use crate::lima::shell;
use crate::platform::host::HostContext;
use crate::session::guest_support;
use crate::session::orchestration::{LockMetadataGuard, SessionGuard, run_step};
use crate::session::paths::{repo_workspace_path, tmux_socket_path};
use crate::session::state::transition_after_open;
use crate::types::{GuestPath, HostPath, ProviderKind, SessionName};
use crate::util::ids::{prepared_base_name, session_vm_name};
use crate::util::process::{CommandRunner, RealCommandRunner};
use crate::util::time::utc_now;
use std::collections::BTreeMap;
use std::fs;
use std::io::IsTerminal;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub fn render_open_json(
    session: &SessionName,
    vm_name: &crate::types::VmName,
    lifecycle_state: crate::db::models::LifecycleState,
    repo_host_path: &HostPath,
    repo_guest_path: &GuestPath,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&serde_json::json!({
        "session": session,
        "vm_name": vm_name,
        "lifecycle_state": crate::db::models::lifecycle_state_name(lifecycle_state),
        "repo_host_path": repo_host_path,
        "repo_guest_path": repo_guest_path,
    }))
}

fn render_open_repo_progress(session: &SessionName, repo_root: &Path, detail: &str) -> String {
    format!("open {}: {} in {}", session, detail, repo_root.display())
}

pub fn run(args: OpenArgs) -> Result<(), AppError> {
    let session_name = SessionName::try_from(args.session.as_str())?;
    let provider = args.agent.as_deref().and_then(ProviderKind::parse);
    let host = HostContext::detect()?;
    std::fs::create_dir_all(&host.state_roots.locks)?;
    let lock_path = host
        .state_roots
        .locks
        .join(format!("{}.lock", session_name));
    let _lock = SessionLock::acquire(&lock_path, std::process::id(), "open")?;

    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    let open_started = Instant::now();
    let baseline = run_step(
        &session_name,
        "open",
        "capture-repo-baseline",
        &open_started,
        || Ok(capture_repo_baseline(&runner, &args.repo)?),
    )?;
    if baseline.head_oid.is_none() {
        return Err(ValidationError::OpenRequiresGitRepo.into());
    }

    let (git_root, resolved_base_ref, base_oid, review_branch, hidden_refs, identity) =
        run_step(&session_name, "open", "resolve-base", &open_started, || {
            let git_root = resolve_git_root(&runner, &args.repo)?;
            let resolved_base_ref = resolve_base_ref(
                args.base.as_deref(),
                baseline.head_ref.as_deref().unwrap_or("HEAD"),
            );
            let base_oid = resolve_ref_oid(&runner, git_root.as_path(), &resolved_base_ref)?;
            let review_branch = review_branch_name(&session_name);
            let hidden_refs = hidden_ref_names(&session_name);
            ensure_open_targets_available(
                &runner,
                git_root.as_path(),
                &hidden_refs.base,
                &hidden_refs.head,
                &review_branch,
            )?;
            let identity = detect_identity(&runner, git_root.as_path())?;
            let identity = identity.ok_or(ValidationError::OpenRequiresGitIdentity)?;
            Ok((
                git_root,
                resolved_base_ref,
                base_oid,
                review_branch,
                hidden_refs,
                identity,
            ))
        })?;

    let head_oid_at_open = Some(base_oid.clone());
    let head_ref_at_open = Some(resolved_base_ref.clone());
    let catalog = open_catalog(&host.state_roots.db)?;
    let now = utc_now();
    let vm_name = session_vm_name(&session_name);
    let guest_repo = guest_repo_path(&host.home_dir, &session_name);
    let guest_tmux_socket = tmux_socket_path(&host.home_dir, &session_name);
    let host_repo = HostPath::new(args.repo.clone());
    ensure_runtime_session_slot_available(&catalog, &session_name, &vm_name)?;

    insert_session(
        &catalog,
        &InsertSession {
            name: session_name.clone(),
            vm_name: vm_name.clone(),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(host_repo.clone()),
            guest_workspace_path: guest_repo.clone(),
            seed_host_path: None,
            host_git_root: Some(git_root.clone()),
            host_head_oid_at_open: head_oid_at_open,
            host_head_ref_at_open: head_ref_at_open,
            host_dirty_at_open: baseline.dirty,
            base_ref: Some(resolved_base_ref.clone()),
            review_branch: Some(review_branch.clone()),
            session_ref_base: Some(hidden_refs.base.clone()),
            session_ref_head: Some(hidden_refs.head.clone()),
            provider_kind: provider,
            imported_provider_files_json: "[]".to_owned(),
            guest_tmux_socket_path: Some(guest_tmux_socket.clone()),
            shell_window_name: Some("shell".to_owned()),
            agent_window_name: Some("agent".to_owned()),
            agent_launch_preset: provider.map(|_| AgentLaunchPreset::Unrestricted),
            created_at: now,
        },
    )?;
    let lock_guard =
        LockMetadataGuard::acquire(&catalog, &session_name, std::process::id(), "open")?;
    eprintln!(
        "{}",
        render_open_repo_progress(
            &session_name,
            git_root.as_path(),
            &format!(
                "create session refs {}/{{base,head}}",
                hidden_refs.base.trim_end_matches("/base")
            ),
        )
    );
    eprintln!(
        "{}",
        render_open_repo_progress(
            &session_name,
            git_root.as_path(),
            &format!("reserve review branch {review_branch}"),
        )
    );
    run_step(
        &session_name,
        "open",
        "initialize-session-refs",
        &open_started,
        || {
            Ok(initialize_session_refs(
                &runner,
                git_root.as_path(),
                &hidden_refs,
                &base_oid,
            )?)
        },
    )?;

    let guard = SessionGuard::open(
        &runner,
        &catalog,
        &session_name,
        &vm_name,
        git_root.as_path(),
        &hidden_refs.base,
        &hidden_refs.head,
        &review_branch,
    );

    let result: Result<(), AppError> = (|| {
        let seed_clone = run_step(
            &session_name,
            "open",
            "seed-repo-clone",
            &open_started,
            || {
                create_git_seed_clone(
                    &runner,
                    &host.state_roots.staging,
                    &git_root,
                    &resolved_base_ref,
                    &review_branch,
                )
            },
        )?;
        let base_clone_lock =
            run_step(&session_name, "open", "prepare-base", &open_started, || {
                let mut on_notice =
                    |notice| emit_prepared_base_notice(&catalog, &session_name, &notice);
                acquire_clone_lock_for_prepared_base(
                    &runner,
                    &host,
                    "open clone",
                    "open prepare-base",
                    &mut on_notice,
                )
            })?;
        run_step(&session_name, "open", "clone-vm", &open_started, || {
            Ok(lima.clone_instance(
                &prepared_base_name(host.platform),
                &vm_name,
                args.cpus,
                args.memory.as_ref(),
                args.disk.as_ref(),
            )?)
        })?;
        drop(base_clone_lock);
        run_step(&session_name, "open", "start-vm", &open_started, || {
            Ok(lima.start_instance(&vm_name)?)
        })?;
        run_step(
            &session_name,
            "open",
            "install-guest-support",
            &open_started,
            || {
                Ok(guest_support::install_guest_support_files(
                    &lima,
                    &vm_name,
                    &host.home_dir,
                )?)
            },
        )?;
        run_step(&session_name, "open", "seed-repo", &open_started, || {
            Ok(lima.seed_repo(seed_clone.path(), &vm_name, &guest_repo)?)
        })?;
        run_step(&session_name, "open", "ensure-shell", &open_started, || {
            Ok(guest_support::ensure_workspace_and_shell(
                &lima,
                &vm_name,
                &session_name,
                &guest_tmux_socket,
                &guest_repo,
            )?)
        })?;
        run_step(
            &session_name,
            "open",
            "configure-git-identity",
            &open_started,
            || configure_guest_identity(&runner, &vm_name, &guest_repo, &identity),
        )?;
        if let Some(provider) = provider {
            let imported = run_step(&session_name, "open", "launch-agent", &open_started, || {
                start_session_owned_agent_with(
                    &lima,
                    SessionOwnedAgentLaunch {
                        session_name: &session_name,
                        vm_name: &vm_name,
                        workspace: &guest_repo,
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

        run_step(&session_name, "open", "finalize", &open_started, || {
            Ok(update_lifecycle_state_with_timestamps(
                &catalog,
                &session_name,
                transition_after_open(),
                &now,
                Some(&now),
                None,
                None,
            )?)
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
            render_open_json(
                &session_name,
                &vm_name,
                transition_after_open(),
                &host_repo,
                &guest_repo,
            )
            .map_err(|err| AppError::Validation(ValidationError::StepFailed {
                step: "finalize",
                detail: format!("failed to serialize json output: {err}"),
            }))?
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

fn guest_repo_path(host_home_dir: &Path, session: &SessionName) -> GuestPath {
    repo_workspace_path(host_home_dir, session)
}

fn configure_guest_identity(
    runner: &RealCommandRunner,
    vm_name: &crate::types::VmName,
    repo_guest_path: &GuestPath,
    identity: &GitIdentity,
) -> Result<(), AppError> {
    runner
        .run(
            "limactl",
            &[
                "shell".to_owned(),
                vm_name.as_str().to_owned(),
                "--".to_owned(),
                "bash".to_owned(),
                "-lc".to_owned(),
                format!(
                    "git -C {repo} config user.name {name} && git -C {repo} config user.email {email} && git -C {repo} config commit.gpgsign false",
                    repo = shell::shell_escape(&repo_guest_path.to_string()),
                    name = shell::shell_escape(&identity.name),
                    email = shell::shell_escape(&identity.email),
                ),
            ],
            None,
            &BTreeMap::new(),
        )?;
    Ok(())
}

fn ensure_open_targets_available(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    hidden_base_ref: &str,
    hidden_head_ref: &str,
    review_branch: &str,
) -> Result<(), crate::error::process::ProcessError> {
    let review_ref = format!("refs/heads/{review_branch}");
    if git_ref_exists(runner, repo_root, hidden_base_ref)?
        || git_ref_exists(runner, repo_root, hidden_head_ref)?
        || git_ref_exists(runner, repo_root, &review_ref)?
    {
        return Err(crate::error::process::ProcessError::Failed {
            program: "git".to_owned(),
            status: 1,
            stderr: format!(
                "session refs or review branch already exist for `{review_branch}`; choose a different session name"
            ),
        });
    }
    Ok(())
}

fn resolve_git_root(
    runner: &dyn CommandRunner,
    repo_root: &Path,
) -> Result<HostPath, crate::error::process::ProcessError> {
    let output = runner.run(
        "git",
        &["rev-parse".to_owned(), "--show-toplevel".to_owned()],
        Some(repo_root),
        &BTreeMap::new(),
    )?;
    Ok(HostPath::new(output.stdout.trim()))
}

fn git_ref_exists(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    reference: &str,
) -> Result<bool, crate::error::process::ProcessError> {
    match runner.run(
        "git",
        &[
            "show-ref".to_owned(),
            "--verify".to_owned(),
            "--quiet".to_owned(),
            reference.to_owned(),
        ],
        Some(repo_root),
        &BTreeMap::new(),
    ) {
        Ok(_) => Ok(true),
        Err(crate::error::process::ProcessError::Failed { .. }) => Ok(false),
        Err(err) => Err(err),
    }
}

struct SeedClone {
    path: HostPath,
}

impl SeedClone {
    fn path(&self) -> &HostPath {
        &self.path
    }
}

impl Drop for SeedClone {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.path.as_path());
    }
}

fn create_git_seed_clone(
    runner: &dyn CommandRunner,
    staging_root: &Path,
    repo_root: &HostPath,
    base_ref: &str,
    review_branch: &str,
) -> Result<SeedClone, AppError> {
    fs::create_dir_all(staging_root)?;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let clone_path = HostPath::new(staging_root.join(format!("seed-clone-{unique}")));
    runner.run(
        "git",
        &[
            "clone".to_owned(),
            "--quiet".to_owned(),
            "--no-hardlinks".to_owned(),
            repo_root.to_string(),
            clone_path.to_string(),
        ],
        None,
        &BTreeMap::new(),
    )?;
    runner.run(
        "git",
        &[
            "-C".to_owned(),
            clone_path.to_string(),
            "checkout".to_owned(),
            "--quiet".to_owned(),
            base_ref.to_owned(),
        ],
        None,
        &BTreeMap::new(),
    )?;
    runner.run(
        "git",
        &[
            "-C".to_owned(),
            clone_path.to_string(),
            "checkout".to_owned(),
            "--quiet".to_owned(),
            "-B".to_owned(),
            review_branch.to_owned(),
        ],
        None,
        &BTreeMap::new(),
    )?;
    Ok(SeedClone { path: clone_path })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{guest_repo_path, render_open_repo_progress};
    use crate::db::models::LifecycleState;
    use crate::session::paths::tmux_socket_path;
    use crate::types::{SessionName, VmName};
    use std::path::{Path, PathBuf};

    #[test]
    fn derives_guest_repo_path_from_host_home() {
        let session = SessionName::try_from("agbranch-smoke-happy").expect("session");
        let guest_repo = guest_repo_path(Path::new("/Users/abalaian"), &session);

        assert_eq!(
            guest_repo.as_path(),
            Path::new("/home/abalaian.guest/workspaces/agbranch-smoke-happy/repo")
        );
    }

    #[test]
    fn derives_guest_tmux_socket_under_agbranch_home() {
        let session = SessionName::try_from("agbranch-smoke-happy").expect("session");
        let socket = tmux_socket_path(Path::new("/Users/abalaian"), &session);

        assert_eq!(
            socket.as_path(),
            Path::new("/home/abalaian.guest/.agbranch/tmux/agbranch-smoke-happy.sock")
        );
    }

    #[test]
    fn open_repo_progress_line_mentions_target_repo_and_ref_action() {
        let session = SessionName::try_from("agbranch-smoke-happy").expect("session");

        assert_eq!(
            render_open_repo_progress(
                &session,
                Path::new("/tmp/example-repo"),
                "reserve review branch agbranch/agbranch-smoke-happy",
            ),
            "open agbranch-smoke-happy: reserve review branch agbranch/agbranch-smoke-happy in /tmp/example-repo"
        );
    }

    #[test]
    fn open_json_contains_session_vm_and_lifecycle_state() {
        let rendered = render_open_json(
            &SessionName::try_from("agbranch-smoke-happy").expect("session"),
            &VmName::new("agbranch-smoke-happy"),
            LifecycleState::Running,
            &HostPath::new(PathBuf::from("/tmp/fixture")),
            &GuestPath::new(PathBuf::from(
                "/home/agbranch.guest/workspaces/agbranch-smoke-happy/repo",
            )),
        )
        .expect("open json");

        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid json");
        assert_eq!(value["session"], "agbranch-smoke-happy");
        assert_eq!(value["vm_name"], "agbranch-smoke-happy");
        assert_eq!(value["lifecycle_state"], "running");
    }
}
