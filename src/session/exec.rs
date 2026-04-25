use crate::db::connect::open_catalog;
use crate::db::sessions::find_session;
use crate::error::{AppError, ValidationError};
use crate::lima::client::{LimaClient, LimactlClient};
use crate::lima::inspect::LimaInstanceStatus;
use crate::lima::shell::{SshCommandSpec, build_ssh_command};
use crate::platform::host::HostContext;
use crate::policy::secrets::{guest_secret_path, render_guest_secret_file};
use crate::types::{GuestPath, HostPath, SessionName, VmName};
use crate::util::process::RealCommandRunner;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct ResolvedConnection {
    pub session_name: SessionName,
    pub vm_name: VmName,
    pub host_repo_path: Option<HostPath>,
    pub guest_repo_path: GuestPath,
    pub ssh_config_file: PathBuf,
    pub host_alias: String,
}

pub struct SessionSshRequest<'a> {
    pub forward_agent: bool,
    pub force_tty: bool,
    pub guest_secret_file: Option<&'a Path>,
    pub command: Option<&'a [String]>,
}

pub fn build_session_ssh_args(
    connection: &ResolvedConnection,
    request: SessionSshRequest<'_>,
) -> Vec<String> {
    build_ssh_command(SshCommandSpec {
        ssh_config_file: &connection.ssh_config_file,
        host_alias: &connection.host_alias,
        session: connection.session_name.as_str(),
        workdir: connection.guest_repo_path.as_path(),
        forward_agent: request.forward_agent,
        force_tty: request.force_tty,
        guest_secret_file: request.guest_secret_file,
        command: request.command,
    })
}

pub fn resolve_connection(session_name: &SessionName) -> Result<ResolvedConnection, AppError> {
    let host = HostContext::detect()?;
    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    resolve_connection_with(&lima, &host, session_name)
}

pub fn resolve_connection_with(
    client: &dyn LimaClient,
    host: &HostContext,
    session_name: &SessionName,
) -> Result<ResolvedConnection, AppError> {
    let conn = open_catalog(&host.state_roots.db)?;
    let session = find_session(&conn, session_name)?.ok_or_else(|| {
        AppError::Validation(ValidationError::SessionNotFound(session_name.to_string()))
    })?;

    let instances = client.list_instances()?;
    let instance = instances
        .into_iter()
        .find(|item| item.name == session.vm_name.as_str())
        .ok_or_else(|| {
            AppError::Validation(ValidationError::SessionNotFound(session_name.to_string()))
        })?;

    let ssh_config_file = PathBuf::from(&instance.ssh_config_file);
    let host_alias = host_alias_from_config(&ssh_config_file)?;
    Ok(ResolvedConnection {
        session_name: session.name,
        vm_name: session.vm_name,
        host_repo_path: session.host_context_path.or(session.seed_host_path),
        guest_repo_path: session.guest_workspace_path,
        ssh_config_file,
        host_alias,
    })
}

pub fn ensure_instance_running(connection: &ResolvedConnection) -> Result<(), AppError> {
    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    ensure_instance_running_with(&lima, connection)
}

pub fn ensure_instance_running_with(
    client: &dyn LimaClient,
    connection: &ResolvedConnection,
) -> Result<(), AppError> {
    let instances = client.list_instances()?;
    let running = instances
        .iter()
        .find(|item| item.name == connection.vm_name.as_str())
        .map(|item| item.status == LimaInstanceStatus::Running)
        .unwrap_or(false);
    if !running {
        client.start_instance(&connection.vm_name)?;
    }
    Ok(())
}

pub fn materialize_guest_secret(
    connection: &ResolvedConnection,
    env: &BTreeMap<String, String>,
) -> Result<Option<GuestPath>, AppError> {
    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    materialize_guest_secret_with(&lima, connection, env)
}

pub fn materialize_guest_secret_with(
    client: &dyn LimaClient,
    connection: &ResolvedConnection,
    env: &BTreeMap<String, String>,
) -> Result<Option<GuestPath>, AppError> {
    if env.is_empty() {
        return Ok(None);
    }
    let rendered = render_guest_secret_file(env)?;
    let host_secret = temp_secret_file(&rendered)?;
    let guest_secret = guest_secret_path(&connection.guest_repo_path, &connection.session_name);
    let copy_result = client
        .copy_guest_secret_file(
            &HostPath::new(host_secret.clone()),
            &connection.vm_name,
            &guest_secret,
        )
        .map_err(Into::into);
    finalize_temp_secret_file(&host_secret, copy_result)?;
    Ok(Some(guest_secret))
}

pub fn run_host_command(program: &str, args: &[String]) -> Result<(), AppError> {
    let status = Command::new(program).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::Validation(ValidationError::GuestCommandFailed {
            program: program.to_string(),
            status: status.code().unwrap_or(1),
        }))
    }
}

pub fn host_alias_from_config(path: &Path) -> Result<String, AppError> {
    let config = fs::read_to_string(path)?;
    for line in config.lines() {
        if let Some(alias) = line.strip_prefix("Host ") {
            let alias = alias.trim();
            if !alias.is_empty() && alias != "*" {
                return Ok(alias.to_owned());
            }
        }
    }
    Err(AppError::Validation(ValidationError::SshResolutionFailed))
}

fn temp_secret_file(rendered: &str) -> Result<PathBuf, AppError> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("agbranch-secret-{unique}.env"));
    fs::write(&path, rendered)?;
    Ok(path)
}

fn finalize_temp_secret_file(
    host_secret: &Path,
    copy_result: Result<(), AppError>,
) -> Result<(), AppError> {
    let cleanup_result = fs::remove_file(host_secret).map_err(|err| {
        AppError::Validation(ValidationError::TempSecretCleanupFailed {
            path: host_secret.to_path_buf(),
            source: err,
        })
    });

    match (copy_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(copy_err), Ok(())) => Err(copy_err),
        (Ok(()), Err(cleanup_err)) => Err(cleanup_err),
        (Err(copy_err), Err(cleanup_err)) => Err(AppError::Validation(
            ValidationError::AgentBootstrapChainedFailure {
                primary: copy_err.to_string(),
                cleanup: cleanup_err.to_string(),
            },
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ResolvedConnection, SessionSshRequest, build_session_ssh_args, finalize_temp_secret_file,
        materialize_guest_secret_with, resolve_connection_with,
    };
    use crate::db::connect::open_catalog;
    use crate::db::sessions::{InsertSession, insert_session};
    use crate::error::AppError;
    use crate::error::lima::LimaError;
    use crate::lima::client::LimaClient;
    use crate::lima::inspect::{LimaConfig, LimaInstance, LimaInstanceStatus};
    use crate::platform::detect::HostPlatform;
    use crate::platform::host::HostContext;
    use crate::platform::paths::StateRoots;
    use crate::types::{
        DiskSize, GuestPath, HostPath, MemorySize, SessionMode, SessionName, Timestamp, VmName,
    };
    use crate::util::process::CommandOutput;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn fixture_connection() -> ResolvedConnection {
        ResolvedConnection {
            session_name: SessionName::try_from("demo").expect("valid session"),
            vm_name: VmName::new("agbranch-demo"),
            host_repo_path: Some(HostPath::new("/host/repo")),
            guest_repo_path: GuestPath::new("/home/demo/workspaces/demo/repo"),
            ssh_config_file: PathBuf::from("/tmp/ssh-config"),
            host_alias: "lima-agbranch-demo".to_owned(),
        }
    }

    #[derive(Default)]
    struct StubLimaClient {
        copied_guest_targets: RefCell<Vec<String>>,
        instances: Vec<LimaInstance>,
    }

    impl LimaClient for StubLimaClient {
        fn list_instances(&self) -> Result<Vec<LimaInstance>, LimaError> {
            Ok(self.instances.clone())
        }

        fn clone_instance(
            &self,
            _source: &VmName,
            _target: &VmName,
            _cpus: Option<u16>,
            _memory: Option<&MemorySize>,
            _disk: Option<&DiskSize>,
        ) -> Result<(), LimaError> {
            unreachable!("clone_instance is not used by session exec unit tests");
        }

        fn start_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("start_instance is not used by session exec unit tests");
        }

        fn stop_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("stop_instance is not used by session exec unit tests");
        }

        fn delete_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("delete_instance is not used by session exec unit tests");
        }

        fn bash(&self, _vm: &VmName, _command: &str) -> Result<CommandOutput, LimaError> {
            unreachable!("bash is not used by session exec unit tests");
        }

        fn copy_host_path_to_guest(
            &self,
            _host_path: &HostPath,
            _instance_name: &VmName,
            _guest_path: &GuestPath,
        ) -> Result<(), LimaError> {
            unreachable!("copy_host_path_to_guest is not used by session exec unit tests");
        }

        fn seed_repo(
            &self,
            _filtered_seed_root: &HostPath,
            _instance_name: &VmName,
            _guest_repo: &GuestPath,
        ) -> Result<(), LimaError> {
            unreachable!("seed_repo is not used by session exec unit tests");
        }

        fn copy_host_file_to_guest(
            &self,
            _host_file: &HostPath,
            _instance_name: &VmName,
            _guest_file: &GuestPath,
        ) -> Result<(), LimaError> {
            unreachable!("copy_host_file_to_guest is not used by session exec unit tests");
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

    #[test]
    fn build_session_ssh_args_supports_interactive_shell_bootstrap() {
        let connection = fixture_connection();
        let args = build_session_ssh_args(
            &connection,
            SessionSshRequest {
                forward_agent: true,
                force_tty: true,
                guest_secret_file: Some(Path::new("/home/demo/.agbranch/secrets/demo/command.env")),
                command: None,
            },
        );

        assert_eq!(args[0], "-F");
        assert_eq!(args[1], "/tmp/ssh-config");
        assert_eq!(args[2], "-A");
        assert_eq!(args[3], "-t");
        assert_eq!(args[4], "lima-agbranch-demo");
        assert_eq!(
            args[5],
            "cd '/home/demo/workspaces/demo/repo' && export AGBRANCH_SESSION='demo' && source \"$HOME/.agbranch/shellenv.sh\" && if [ -f '/home/demo/.agbranch/secrets/demo/command.env' ]; then set -a && . '/home/demo/.agbranch/secrets/demo/command.env' && set +a; fi && exec ${SHELL:-/bin/bash} -l"
        );
    }

    #[test]
    fn build_session_ssh_args_supports_noninteractive_exec() {
        let connection = fixture_connection();
        let command = vec![
            "cargo".to_owned(),
            "test".to_owned(),
            "--workspace".to_owned(),
        ];
        let args = build_session_ssh_args(
            &connection,
            SessionSshRequest {
                forward_agent: false,
                force_tty: false,
                guest_secret_file: None,
                command: Some(&command),
            },
        );

        assert_eq!(args[0], "-F");
        assert_eq!(args[1], "/tmp/ssh-config");
        assert_eq!(args[2], "lima-agbranch-demo");
        assert!(!args.iter().any(|arg| arg == "-t"));
        assert_eq!(
            args[3],
            "cd '/home/demo/workspaces/demo/repo' && export AGBRANCH_SESSION='demo' && source \"$HOME/.agbranch/shellenv.sh\" && exec 'cargo' 'test' '--workspace'"
        );
    }

    #[test]
    fn materialize_guest_secret_short_circuits_when_env_is_empty() {
        let connection = fixture_connection();
        let env = BTreeMap::new();

        let secret = materialize_guest_secret_with(&StubLimaClient::default(), &connection, &env)
            .expect("materialize");

        assert!(secret.is_none());
    }

    #[test]
    fn finalize_temp_secret_file_propagates_copy_error_after_successful_cleanup() {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        let path = file.path().to_path_buf();
        drop(file);
        std::fs::write(&path, "secret").expect("write");

        let result = finalize_temp_secret_file(
            &path,
            Err(AppError::Process(
                crate::error::process::ProcessError::Failed {
                    program: "limactl".to_owned(),
                    status: 1,
                    stderr: "copy failed".to_owned(),
                },
            )),
        );

        assert!(result.is_err());
        assert!(!path.exists());
    }

    #[test]
    fn finalize_temp_secret_file_errors_when_cleanup_fails_after_copy_success() {
        let missing = PathBuf::from("/tmp/agbranch-secret-missing-for-cleanup.env");
        let _ = std::fs::remove_file(&missing);

        let result = finalize_temp_secret_file(&missing, Ok(()));

        let error = result.expect_err("cleanup should fail");
        assert!(
            error
                .to_string()
                .contains("failed to remove temporary secret file")
        );
    }

    #[test]
    fn resolve_connection_supports_sandbox_runtime_sessions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let host = HostContext {
            platform: HostPlatform::current().expect("supported platform"),
            home_dir: dir.path().join("home"),
            xdg_state_home: None,
            state_roots: StateRoots::from_base(&dir.path().join("state-root")),
        };
        std::fs::create_dir_all(&host.home_dir).expect("home dir");

        let ssh_config = dir.path().join("sandbox.ssh.config");
        std::fs::write(&ssh_config, "Host lima-sandbox\n").expect("ssh config");

        let conn = open_catalog(&host.state_roots.db).expect("catalog");
        let session = SessionName::try_from("sandbox-demo").expect("session");
        let seed = HostPath::new("/tmp/sandbox-seed");
        insert_session(
            &conn,
            &InsertSession {
                name: session.clone(),
                vm_name: VmName::new("agbranch-sandbox-demo"),
                session_mode: SessionMode::Sandbox,
                repo_sync_mode: None,
                host_context_path: None,
                guest_workspace_path: GuestPath::new("/home/tester.guest/sandbox/sandbox-demo"),
                seed_host_path: Some(seed.clone()),
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
                guest_tmux_socket_path: Some(GuestPath::new(
                    "/home/tester.guest/.agbranch/tmux/sandbox-demo.sock",
                )),
                shell_window_name: Some("shell".to_owned()),
                agent_window_name: Some("agent".to_owned()),
                agent_launch_preset: None,
                created_at: Timestamp::parse_rfc3339("2026-04-22T00:00:00Z").expect("timestamp"),
            },
        )
        .expect("insert runtime session");

        let resolved = resolve_connection_with(
            &StubLimaClient {
                instances: vec![LimaInstance {
                    name: "agbranch-sandbox-demo".to_owned(),
                    instance_dir: "/tmp/agbranch-sandbox-demo".to_owned(),
                    ssh_config_file: ssh_config.display().to_string(),
                    vm_type: "vz".to_owned(),
                    raw_status: "Running".to_owned(),
                    arch: None,
                    cpus: None,
                    memory: None,
                    disk: None,
                    protected: false,
                    ssh_local_port: None,
                    ssh_address: None,
                    config: LimaConfig::default(),
                    status: LimaInstanceStatus::Running,
                }],
                ..Default::default()
            },
            &host,
            &session,
        )
        .expect("resolve sandbox connection");

        assert_eq!(resolved.host_repo_path.as_ref(), Some(&seed));
        assert_eq!(
            resolved.guest_repo_path.as_path(),
            Path::new("/home/tester.guest/sandbox/sandbox-demo")
        );
        assert_eq!(resolved.host_alias, "lima-sandbox");
    }
}
