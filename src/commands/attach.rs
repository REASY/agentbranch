use crate::cli::AttachArgs;
use crate::commands::{find_existing_session, resolve_session_name};
use crate::db::connect::open_catalog;
use crate::error::{AppError, ValidationError};
use crate::lima::client::{LimaClient, LimactlClient};
use crate::lima::shell::{SshCommandSpec, build_ssh_command};
use crate::lima::tmux;
use crate::platform::host::HostContext;
use crate::session::exec::{host_alias_from_config, run_host_command};
use crate::session::runtime::{
    GuestRuntimeProbe, RuntimeProbeTarget, WindowState, probe_guest_runtime,
};
use crate::types::{ProviderKind, SessionName};
use std::path::PathBuf;

#[derive(Clone, Copy)]
enum AttachTarget {
    Shell,
    Agent,
}

impl AttachTarget {
    fn label(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Agent => "agent",
        }
    }
}

pub fn run(args: AttachArgs) -> Result<(), AppError> {
    let (session_name_raw, session_name) = resolve_session_name(&args.session)?;
    if args.shell == args.agent {
        return Err(AppError::Validation(
            ValidationError::AttachRequiresExactlyOne,
        ));
    }

    let host = HostContext::detect()?;
    let conn = open_catalog(&host.state_roots.db)?;
    let session = find_existing_session(&conn, &session_name, &session_name_raw)?;
    let target = if args.shell {
        AttachTarget::Shell
    } else {
        AttachTarget::Agent
    };
    let tmux_socket = session
        .guest_tmux_socket_path
        .as_ref()
        .ok_or(ValidationError::SessionMissingTmuxSocket)?;

    let runner = crate::util::process::RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    let instances = lima.list_instances()?;
    let instance = instances
        .into_iter()
        .find(|item| item.name == session.vm_name.as_str())
        .ok_or_else(|| {
            AppError::Validation(ValidationError::SessionNotFound(session_name_raw.clone()))
        })?;

    if !args.json {
        let runtime = probe_guest_runtime(
            &lima,
            RuntimeProbeTarget {
                session_name: session_name.as_str(),
                vm_name: &session.vm_name,
                provider_kind: session.provider_kind,
                guest_tmux_socket_path: Some(tmux_socket),
                shell_window_name: session.shell_window_name.as_deref(),
                agent_window_name: session.agent_window_name.as_deref(),
            },
        );
        validate_attach_target(&session_name, session.provider_kind, target, &runtime)?;
    }

    let ssh_config_file = PathBuf::from(&instance.ssh_config_file);
    let host_alias = host_alias_from_config(&ssh_config_file)?;
    let command = if matches!(target, AttachTarget::Shell) {
        tmux::attach_shell_command(
            tmux_socket,
            session_name.as_str(),
            session.shell_window_name.as_deref().unwrap_or("shell"),
            session_name.as_str(),
            &session.guest_workspace_path,
        )
    } else {
        tmux::attach_window_command(
            tmux_socket,
            session_name.as_str(),
            session.agent_window_name.as_deref().unwrap_or("agent"),
        )
    };
    let attach_command = ["bash".to_owned(), "-lc".to_owned(), command];
    let ssh_args = build_ssh_command(SshCommandSpec {
        ssh_config_file: &ssh_config_file,
        host_alias: &host_alias,
        session: session_name.as_str(),
        workdir: session.guest_workspace_path.as_path(),
        forward_agent: false,
        force_tty: true,
        guest_secret_file: None,
        command: Some(&attach_command),
    });

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "session": session_name,
                "target": target.label(),
                "ssh_config_file": ssh_config_file,
                "host_alias": host_alias,
            })
        );
        return Ok(());
    }

    run_host_command("ssh", &ssh_args)
}

fn validate_attach_target(
    session_name: &SessionName,
    provider_kind: Option<ProviderKind>,
    target: AttachTarget,
    runtime: &GuestRuntimeProbe,
) -> Result<(), AppError> {
    let err = match target {
        AttachTarget::Shell => match runtime.shell_window_state {
            WindowState::Missing => Some(ValidationError::AttachTargetUnavailable {
                session: session_name.to_string(),
                target: "shell",
                reason: format!(
                    "tmux window `{}` is missing; run `agbranch start --session {}`",
                    runtime.shell_window_name, session_name
                ),
            }),
            WindowState::Unknown => Some(ValidationError::AttachTargetUnavailable {
                session: session_name.to_string(),
                target: "shell",
                reason: format!(
                    "guest tmux runtime could not be probed; check `agbranch show --session {}` or rerun `agbranch start --session {}`",
                    session_name, session_name
                ),
            }),
            WindowState::Live | WindowState::Dead => None,
        },
        AttachTarget::Agent => match runtime.agent_window_state {
            WindowState::Missing => Some(ValidationError::AttachTargetUnavailable {
                session: session_name.to_string(),
                target: "agent",
                reason: missing_agent_reason(
                    session_name,
                    provider_kind,
                    &runtime.agent_window_name,
                    runtime.shell_window_state,
                ),
            }),
            WindowState::Unknown => Some(ValidationError::AttachTargetUnavailable {
                session: session_name.to_string(),
                target: "agent",
                reason: format!(
                    "guest tmux runtime could not be probed; check `agbranch show --session {}` or rerun `agbranch start --session {}`",
                    session_name, session_name
                ),
            }),
            WindowState::Live | WindowState::Dead => None,
        },
    };
    match err {
        Some(err) => Err(AppError::Validation(err)),
        None => Ok(()),
    }
}

fn missing_agent_reason(
    session_name: &SessionName,
    provider_kind: Option<ProviderKind>,
    agent_window_name: &str,
    shell_window_state: WindowState,
) -> String {
    let restart_hint = match provider_kind {
        Some(provider) => format!(
            "run `agbranch agent start --session {} --provider {}`",
            session_name,
            provider.as_str()
        ),
        None => format!(
            "run `agbranch agent start --session {} --provider <codex|claude|gemini>`",
            session_name
        ),
    };

    if shell_window_state == WindowState::Live {
        format!(
            "tmux window `{}` is missing; after stop/start only the shell window was restored, so {}",
            agent_window_name, restart_hint
        )
    } else {
        format!(
            "tmux window `{}` is missing; {}",
            agent_window_name, restart_hint
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{AttachTarget, missing_agent_reason, validate_attach_target};
    use crate::error::{AppError, ValidationError};
    use crate::session::runtime::{GuestRuntimeProbe, WindowState};
    use crate::types::{ProviderKind, SessionName};

    fn runtime(
        shell_window_state: WindowState,
        agent_window_state: WindowState,
    ) -> GuestRuntimeProbe {
        GuestRuntimeProbe {
            agent: "shell-only".to_owned(),
            shell_window_name: "shell".to_owned(),
            shell_window_state,
            agent_window_name: "agent".to_owned(),
            agent_window_state,
        }
    }

    #[test]
    fn missing_agent_reason_explains_restart_only_restores_shell() {
        let session = SessionName::try_from("ops-attach").expect("session");
        let reason = missing_agent_reason(
            &session,
            Some(ProviderKind::Codex),
            "agent",
            WindowState::Live,
        );
        assert!(reason.contains("only the shell window was restored"));
        assert!(reason.contains("agbranch agent start --session ops-attach --provider codex"));
    }

    #[test]
    fn attach_agent_blocks_when_agent_window_is_missing() {
        let session = SessionName::try_from("ops-attach").expect("session");
        let err = validate_attach_target(
            &session,
            Some(ProviderKind::Codex),
            AttachTarget::Agent,
            &runtime(WindowState::Live, WindowState::Missing),
        )
        .expect_err("missing agent window should block attach");
        match err {
            AppError::Validation(ValidationError::AttachTargetUnavailable {
                session,
                target,
                reason,
            }) => {
                assert_eq!(session, "ops-attach");
                assert_eq!(target, "agent");
                assert!(reason.contains("only the shell window was restored"));
            }
            other => panic!("expected attach target error, got {other:?}"),
        }
    }

    #[test]
    fn attach_agent_allows_existing_dead_window() {
        let session = SessionName::try_from("ops-attach").expect("session");
        validate_attach_target(
            &session,
            Some(ProviderKind::Codex),
            AttachTarget::Agent,
            &runtime(WindowState::Live, WindowState::Dead),
        )
        .expect("dead windows remain attachable");
    }
}
