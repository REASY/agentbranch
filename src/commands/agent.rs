use crate::cli::{AgentAction, AgentArgs, AttachArgs};
use crate::db::connect::open_catalog;
use crate::db::models::{AgentLaunchPreset, ProviderKind as StoredProviderKind};
use crate::db::sessions::{find_session, update_agent_metadata};
use crate::error::process::ProcessError;
use crate::error::{AppError, ValidationError};
use crate::lima::{
    client::{LimaClient, LimactlClient},
    tmux,
};
use crate::platform::detect::HostPlatform;
use crate::platform::host::HostContext;
use crate::policy::secrets::render_guest_secret_file;
use crate::provider::auth::{
    AuthPrompter, DetectedAuthSource, ImportedAuthMaterial, TerminalAuthPrompter, detect_auth,
    select_auth_imports,
};
use crate::provider::bootstrap::{GeminiAuthMode, bootstrap_files_with_gemini_auth};
use crate::provider::launch;
use crate::provider::registry::provider_spec;
use crate::types::{GuestPath, HostPath, ProviderKind, SessionName, VmName};
use crate::util::process::RealCommandRunner;
use crate::util::time::utc_now;
use std::collections::BTreeMap;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) struct SessionOwnedAgentLaunch<'a> {
    pub(crate) session_name: &'a SessionName,
    pub(crate) vm_name: &'a VmName,
    pub(crate) workspace: &'a GuestPath,
    pub(crate) host_home: &'a std::path::Path,
    pub(crate) provider: ProviderKind,
    pub(crate) shell_window_name: &'a str,
    pub(crate) agent_window_name: &'a str,
}

pub(crate) fn ensure_session_provider(
    existing: Option<StoredProviderKind>,
    requested: ProviderKind,
) -> Result<ProviderKind, ValidationError> {
    match existing {
        None => Ok(requested),
        Some(current) if current == requested => Ok(requested),
        Some(current) => Err(ValidationError::ProviderConflict {
            current: current.as_str().to_owned(),
            requested: requested.as_str().to_owned(),
        }),
    }
}

pub(crate) fn auth_prompt_enabled(json_mode: bool, stdin_tty: bool, stdout_tty: bool) -> bool {
    !json_mode && stdin_tty && stdout_tty
}

pub(crate) fn start_session_owned_agent_with(
    client: &dyn LimaClient,
    request: SessionOwnedAgentLaunch<'_>,
    host_platform: HostPlatform,
    host_env: &BTreeMap<String, String>,
    interactive_auth_prompt: bool,
    prompter: &dyn AuthPrompter,
) -> Result<Vec<ImportedAuthMaterial>, AppError> {
    ensure_provider_binary_available(client, request.vm_name, request.provider)?;
    let guest_home = crate::session::paths::guest_home_path(request.host_home);
    let detected_auth = detect_auth(
        request.provider,
        host_platform,
        request.host_home,
        host_env,
        &guest_home,
    );
    let selected_auth = select_auth_imports(
        request.provider,
        &detected_auth,
        interactive_auth_prompt,
        prompter,
    )?;
    install_provider_bootstrap_files(client, &request, selected_gemini_auth_mode(&selected_auth))?;
    let (imported_auth, guest_auth_env) =
        materialize_auth_imports(client, &request, &selected_auth)?;
    let launch_line = launch::build_agent_shell_line(
        request.session_name.as_str(),
        request.workspace,
        request.provider,
    );
    let _ = guest_auth_env;
    let mut commands = tmux::ensure_shell_window_commands(
        &crate::session::paths::tmux_socket_path(request.host_home, request.session_name),
        request.session_name.as_str(),
        request.shell_window_name,
        request.session_name.as_str(),
        &request.workspace.to_string(),
    );
    commands.extend(tmux::agent_window_launch_commands(
        &crate::session::paths::tmux_socket_path(request.host_home, request.session_name),
        request.session_name.as_str(),
        request.agent_window_name,
        &request.workspace.to_string(),
        &launch_line,
    ));
    client.bash(
        request.vm_name,
        &format!(
            "mkdir -p {} && {}",
            request.workspace,
            commands.join(" && ")
        ),
    )?;
    Ok(imported_auth)
}

pub fn run(args: AgentArgs) -> Result<(), AppError> {
    match args.action {
        AgentAction::Start(start) => {
            let provider = ProviderKind::parse(&start.provider)
                .ok_or_else(|| AppError::Validation(ValidationError::UnsupportedProvider))?;
            let session_name_raw = start.session.resolve_owned()?;
            let session_name = SessionName::try_from(session_name_raw.as_str())?;
            let host = HostContext::detect()?;
            let conn = open_catalog(&host.state_roots.db)?;
            let session = find_session(&conn, &session_name)?.ok_or_else(|| {
                AppError::Validation(ValidationError::SessionNotFound(session_name_raw.clone()))
            })?;
            let provider = ensure_session_provider(session.provider_kind, provider)?;
            let runner = RealCommandRunner;
            let lima = LimactlClient::new(&runner);
            ensure_instance_running(&lima, &session.vm_name)?;
            let imported = start_session_owned_agent_with(
                &lima,
                SessionOwnedAgentLaunch {
                    session_name: &session_name,
                    vm_name: &session.vm_name,
                    workspace: &session.guest_workspace_path,
                    host_home: &host.home_dir,
                    provider,
                    shell_window_name: session.shell_window_name.as_deref().unwrap_or("shell"),
                    agent_window_name: session.agent_window_name.as_deref().unwrap_or("agent"),
                },
                host.platform,
                &std::env::vars().collect::<BTreeMap<_, _>>(),
                auth_prompt_enabled(
                    start.json,
                    std::io::stdin().is_terminal(),
                    std::io::stdout().is_terminal(),
                ),
                &TerminalAuthPrompter,
            )?;
            let now = utc_now();
            let imported_json = serde_json::to_string(&imported)
                .map_err(crate::error::observability::ObservabilityError::from)?;
            update_agent_metadata(
                &conn,
                &session_name,
                provider,
                &imported_json,
                AgentLaunchPreset::Unrestricted,
                &now,
            )?;
            if start.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "session": session_name,
                        "provider": provider.as_str(),
                        "status": "started",
                        "imported_provider_files": imported,
                    })
                );
                return Ok(());
            }
            crate::commands::attach::run(AttachArgs {
                session: crate::cli::SessionSelector::from_session(session_name_raw),
                shell: false,
                agent: true,
                json: false,
            })
        }
        AgentAction::Stop(stop) => {
            let args = crate::cli::KillArgs {
                session: stop.session,
                force: false,
                json: stop.json,
            };
            crate::commands::kill::run(args)
        }
    }
}

fn install_provider_bootstrap_files(
    client: &dyn LimaClient,
    request: &SessionOwnedAgentLaunch<'_>,
    gemini_auth_mode: Option<GeminiAuthMode>,
) -> Result<(), AppError> {
    for bootstrap in bootstrap_files_with_gemini_auth(
        request.provider,
        request.host_home,
        request.workspace,
        gemini_auth_mode,
    ) {
        if guest_path_exists(client, request.vm_name, &bootstrap.guest_path)? {
            continue;
        }
        let host_file = write_temp_file("agbranch-provider-bootstrap", &bootstrap.contents)?;
        let copy_result = client.copy_host_file_to_guest(
            &HostPath::new(host_file.clone()),
            request.vm_name,
            &bootstrap.guest_path,
        );
        remove_temp_file(&host_file, copy_result)?;
    }
    Ok(())
}

fn selected_gemini_auth_mode(selected_auth: &[DetectedAuthSource]) -> Option<GeminiAuthMode> {
    selected_auth.iter().find_map(|source| match source {
        DetectedAuthSource::EnvVar { name, .. }
            if matches!(name.as_str(), "GEMINI_API_KEY" | "GOOGLE_API_KEY") =>
        {
            Some(GeminiAuthMode::GeminiApiKey)
        }
        _ => None,
    })
}

fn materialize_auth_imports(
    client: &dyn LimaClient,
    request: &SessionOwnedAgentLaunch<'_>,
    selected_auth: &[DetectedAuthSource],
) -> Result<(Vec<ImportedAuthMaterial>, Option<GuestPath>), AppError> {
    if selected_auth.is_empty() {
        return Ok((Vec::new(), None));
    }

    let mut imported = Vec::new();
    let mut env = BTreeMap::new();
    for source in selected_auth {
        match source {
            DetectedAuthSource::File {
                host_path,
                guest_path,
            } => {
                client.copy_host_file_to_guest(host_path, request.vm_name, guest_path)?;
                imported.push(source.as_metadata());
            }
            DetectedAuthSource::EnvVar { name, value } => {
                env.insert(name.clone(), value.clone());
                imported.push(source.as_metadata());
            }
        }
    }

    let guest_auth_env = if env.is_empty() {
        None
    } else {
        let rendered = render_guest_secret_file(&env)?;
        let host_secret = write_temp_file("agbranch-agent-auth", &rendered)?;
        let guest_secret =
            crate::session::paths::agent_auth_env_path(request.host_home, request.session_name);
        let copy_result = client.copy_guest_secret_file(
            &HostPath::new(host_secret.clone()),
            request.vm_name,
            &guest_secret,
        );
        remove_temp_file(&host_secret, copy_result)?;
        Some(guest_secret)
    };

    Ok((imported, guest_auth_env))
}

fn guest_path_exists(
    client: &dyn LimaClient,
    vm_name: &VmName,
    guest_path: &GuestPath,
) -> Result<bool, AppError> {
    let command = format!(
        "test -e {}",
        crate::lima::shell::shell_escape(&guest_path.to_string())
    );
    match client.bash(vm_name, &command) {
        Ok(_) => Ok(true),
        Err(crate::error::lima::LimaError::Process(ProcessError::Failed { status: 1, .. })) => {
            Ok(false)
        }
        Err(err) => Err(err.into()),
    }
}

fn write_temp_file(prefix: &str, rendered: &str) -> Result<PathBuf, AppError> {
    let unique = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}.tmp", std::process::id()));
    fs::write(&path, rendered)?;
    Ok(path)
}

fn remove_temp_file(
    host_file: &Path,
    copy_result: Result<(), impl Into<AppError>>,
) -> Result<(), AppError> {
    let cleanup_result = fs::remove_file(host_file);
    match (copy_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(copy_err), Ok(())) => Err(copy_err.into()),
        (Ok(()), Err(cleanup_err)) => Err(cleanup_err.into()),
        (Err(copy_err), Err(cleanup_err)) => {
            let copy_err: AppError = copy_err.into();
            Err(ValidationError::AgentBootstrapChainedFailure {
                primary: copy_err.to_string(),
                cleanup: cleanup_err.to_string(),
            }
            .into())
        }
    }
}

fn ensure_instance_running(client: &dyn LimaClient, vm_name: &VmName) -> Result<(), AppError> {
    let instances = client.list_instances()?;
    let running = instances
        .iter()
        .find(|item| item.name == vm_name.as_str())
        .map(|item| item.status == crate::lima::inspect::LimaInstanceStatus::Running)
        .unwrap_or(false);
    if !running {
        client.start_instance(vm_name)?;
    }
    Ok(())
}

fn ensure_provider_binary_available(
    client: &dyn LimaClient,
    vm_name: &VmName,
    provider: ProviderKind,
) -> Result<(), AppError> {
    let spec = provider_spec(provider);
    let check = format!(
        "command -v {} >/dev/null 2>&1",
        crate::lima::shell::shell_escape(spec.binary_name)
    );
    client.bash(vm_name, &check).map(|_| ()).map_err(|_| {
        ValidationError::ProviderCliMissing {
            name: spec.binary_name.to_owned(),
        }
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::{
        SessionOwnedAgentLaunch, auth_prompt_enabled, ensure_session_provider,
        start_session_owned_agent_with,
    };
    use crate::db::models::ProviderKind as StoredProviderKind;
    use crate::error::lima::LimaError;
    use crate::error::process::ProcessError;
    use crate::lima::client::LimaClient;
    use crate::lima::inspect::LimaInstance;
    use crate::platform::detect::HostPlatform;
    use crate::provider::auth::{AuthPrompter, DetectedAuth};
    use crate::session::paths::{claude_global_state_path, claude_settings_path};
    use crate::types::{
        DiskSize, GuestPath, HostPath, MemorySize, ProviderKind, SessionName, VmName,
    };
    use crate::util::process::CommandOutput;
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet};
    use tempfile::tempdir;

    #[derive(Default)]
    struct RecordingClient {
        bash_commands: RefCell<Vec<String>>,
        copied_guest_targets: RefCell<Vec<String>>,
        fail_provider_check: RefCell<bool>,
        existing_guest_paths: RefCell<BTreeSet<String>>,
    }

    impl LimaClient for RecordingClient {
        fn list_instances(&self) -> Result<Vec<LimaInstance>, LimaError> {
            unreachable!("list_instances is not used by agent unit tests");
        }

        fn clone_instance(
            &self,
            _source: &VmName,
            _target: &VmName,
            _cpus: Option<u16>,
            _memory: Option<&MemorySize>,
            _disk: Option<&DiskSize>,
        ) -> Result<(), LimaError> {
            unreachable!("clone_instance is not used by agent unit tests");
        }

        fn start_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("start_instance is not used by agent unit tests");
        }

        fn stop_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("stop_instance is not used by agent unit tests");
        }

        fn delete_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("delete_instance is not used by agent unit tests");
        }

        fn bash(&self, _vm: &VmName, command: &str) -> Result<CommandOutput, LimaError> {
            self.bash_commands.borrow_mut().push(command.to_owned());
            if let Some(path) = command.strip_prefix("test -e ") {
                let trimmed = path.trim_matches('\'');
                if self.existing_guest_paths.borrow().contains(trimmed) {
                    return Ok(CommandOutput {
                        stdout: String::new(),
                        stderr: String::new(),
                    });
                }
                return Err(LimaError::Process(ProcessError::Failed {
                    program: "limactl".to_owned(),
                    status: 1,
                    stderr: "missing".to_owned(),
                }));
            }
            if *self.fail_provider_check.borrow()
                && command.contains("command -v")
                && command.contains("claude")
            {
                return Err(LimaError::Process(ProcessError::Failed {
                    program: "limactl".to_owned(),
                    status: 1,
                    stderr: "claude not found".to_owned(),
                }));
            }
            Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }

        fn copy_host_path_to_guest(
            &self,
            _host_path: &HostPath,
            _instance_name: &VmName,
            guest_path: &GuestPath,
        ) -> Result<(), LimaError> {
            self.copied_guest_targets
                .borrow_mut()
                .push(guest_path.to_string());
            Ok(())
        }

        fn seed_repo(
            &self,
            _filtered_seed_root: &HostPath,
            _instance_name: &VmName,
            guest_repo: &GuestPath,
        ) -> Result<(), LimaError> {
            self.copied_guest_targets
                .borrow_mut()
                .push(guest_repo.to_string());
            Ok(())
        }

        fn copy_host_file_to_guest(
            &self,
            _host_file: &HostPath,
            _instance_name: &VmName,
            guest_file: &GuestPath,
        ) -> Result<(), LimaError> {
            self.copied_guest_targets
                .borrow_mut()
                .push(guest_file.to_string());
            Ok(())
        }

        fn copy_guest_secret_file(
            &self,
            _host_secret_file: &HostPath,
            _instance_name: &VmName,
            guest_secret_file: &GuestPath,
        ) -> Result<(), LimaError> {
            self.copied_guest_targets
                .borrow_mut()
                .push(guest_secret_file.to_string());
            Ok(())
        }
    }

    #[derive(Default)]
    struct StubPrompter {
        called: Cell<u32>,
        answer: bool,
    }

    impl AuthPrompter for StubPrompter {
        fn confirm_import(
            &self,
            _provider: ProviderKind,
            _detection: &DetectedAuth,
        ) -> Result<bool, crate::error::AppError> {
            self.called.set(self.called.get() + 1);
            Ok(self.answer)
        }
    }

    #[test]
    fn start_refuses_provider_change_within_a_session() {
        let err = ensure_session_provider(Some(StoredProviderKind::Codex), ProviderKind::Claude)
            .expect_err("provider changes should be refused");
        assert!(
            err.to_string()
                .contains("session already belongs to provider `codex`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn start_in_noninteractive_mode_does_not_import_provider_auth() {
        let client = RecordingClient::default();
        let host_home = tempdir().expect("host home");
        let codex_dir = host_home.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).expect("codex dir");
        std::fs::write(codex_dir.join("auth.json"), "{\"auth_mode\":\"chatgpt\"}")
            .expect("provider auth");
        let session = SessionName::try_from("demo").expect("session");
        let vm_name = VmName::new("agbranch-demo");
        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");

        let imported = start_session_owned_agent_with(
            &client,
            SessionOwnedAgentLaunch {
                session_name: &session,
                vm_name: &vm_name,
                workspace: &workspace,
                host_home: host_home.path(),
                provider: ProviderKind::Codex,
                shell_window_name: "shell",
                agent_window_name: "agent",
            },
            HostPlatform::Macos,
            &BTreeMap::new(),
            false,
            &StubPrompter {
                answer: true,
                ..Default::default()
            },
        )
        .expect("agent start should succeed");

        assert!(imported.is_empty(), "provider auth should not be imported");
        assert!(
            !client
                .copied_guest_targets
                .borrow()
                .iter()
                .any(|target| target.contains(".codex/auth.json")),
            "noninteractive startup should not copy host auth into the guest by default"
        );
    }

    #[test]
    fn start_in_interactive_mode_imports_selected_auth_before_launch() {
        let client = RecordingClient::default();
        let host_home = tempdir().expect("host home");
        let mut env = BTreeMap::new();
        env.insert("ANTHROPIC_API_KEY".to_owned(), "sk-ant-test".to_owned());
        let session = SessionName::try_from("demo").expect("session");
        let vm_name = VmName::new("agbranch-demo");
        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");
        let prompter = StubPrompter {
            answer: true,
            ..Default::default()
        };

        let imported = start_session_owned_agent_with(
            &client,
            SessionOwnedAgentLaunch {
                session_name: &session,
                vm_name: &vm_name,
                workspace: &workspace,
                host_home: host_home.path(),
                provider: ProviderKind::Claude,
                shell_window_name: "shell",
                agent_window_name: "agent",
            },
            HostPlatform::Macos,
            &env,
            true,
            &prompter,
        )
        .expect("agent start should succeed");

        assert_eq!(prompter.called.get(), 1);
        assert!(
            !imported.is_empty(),
            "interactive startup should import confirmed auth material"
        );
        assert!(
            client
                .copied_guest_targets
                .borrow()
                .iter()
                .any(|target| target.contains("agent.env")),
            "interactive startup should stage an agent auth env file in the guest"
        );
    }

    #[test]
    fn start_claude_bootstraps_guest_state_before_launch() {
        let client = RecordingClient::default();
        let host_home = tempdir().expect("host home");
        let session = SessionName::try_from("demo").expect("session");
        let vm_name = VmName::new("agbranch-demo");
        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");

        start_session_owned_agent_with(
            &client,
            SessionOwnedAgentLaunch {
                session_name: &session,
                vm_name: &vm_name,
                workspace: &workspace,
                host_home: host_home.path(),
                provider: ProviderKind::Claude,
                shell_window_name: "shell",
                agent_window_name: "agent",
            },
            HostPlatform::Macos,
            &BTreeMap::new(),
            false,
            &StubPrompter::default(),
        )
        .expect("agent start should succeed");

        let copied_targets = client.copied_guest_targets.borrow();
        assert!(
            copied_targets
                .iter()
                .any(|target| target.ends_with(&claude_settings_path(host_home.path()).to_string())),
            "expected guest settings.json bootstrap copy, got {copied_targets:?}"
        );
        assert!(
            copied_targets
                .iter()
                .any(|target| target
                    .ends_with(&claude_global_state_path(host_home.path()).to_string())),
            "expected guest .claude.json bootstrap copy, got {copied_targets:?}"
        );
    }

    #[test]
    fn start_skips_claude_bootstrap_when_guest_state_already_exists() {
        let client = RecordingClient::default();
        let host_home = tempdir().expect("host home");
        client
            .existing_guest_paths
            .borrow_mut()
            .insert(claude_settings_path(host_home.path()).to_string());
        client
            .existing_guest_paths
            .borrow_mut()
            .insert(claude_global_state_path(host_home.path()).to_string());
        let session = SessionName::try_from("demo").expect("session");
        let vm_name = VmName::new("agbranch-demo");
        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");

        start_session_owned_agent_with(
            &client,
            SessionOwnedAgentLaunch {
                session_name: &session,
                vm_name: &vm_name,
                workspace: &workspace,
                host_home: host_home.path(),
                provider: ProviderKind::Claude,
                shell_window_name: "shell",
                agent_window_name: "agent",
            },
            HostPlatform::Macos,
            &BTreeMap::new(),
            false,
            &StubPrompter::default(),
        )
        .expect("agent start should succeed");

        let copied_targets = client.copied_guest_targets.borrow();
        assert!(
            copied_targets
                .iter()
                .all(|target| !target.contains(".claude")),
            "existing guest Claude state should not be overwritten: {copied_targets:?}"
        );
    }

    #[test]
    fn auth_prompt_enabled_respects_json_mode() {
        assert!(auth_prompt_enabled(false, true, true));
        assert!(!auth_prompt_enabled(true, true, true));
        assert!(!auth_prompt_enabled(false, false, true));
        assert!(!auth_prompt_enabled(false, true, false));
    }

    #[test]
    fn start_fails_clearly_when_provider_binary_is_missing_in_guest() {
        let client = RecordingClient::default();
        *client.fail_provider_check.borrow_mut() = true;
        let host_home = tempdir().expect("host home");
        let session = SessionName::try_from("demo").expect("session");
        let vm_name = VmName::new("agbranch-demo");
        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");

        let err = start_session_owned_agent_with(
            &client,
            SessionOwnedAgentLaunch {
                session_name: &session,
                vm_name: &vm_name,
                workspace: &workspace,
                host_home: host_home.path(),
                provider: ProviderKind::Claude,
                shell_window_name: "shell",
                agent_window_name: "agent",
            },
            HostPlatform::Macos,
            &BTreeMap::new(),
            false,
            &StubPrompter {
                called: Cell::new(0),
                answer: true,
            },
        )
        .expect_err("missing provider binary should block startup");

        assert!(
            err.to_string()
                .contains("provider CLI `claude` is unavailable in the prepared base"),
            "unexpected message: {err}"
        );
    }
}
