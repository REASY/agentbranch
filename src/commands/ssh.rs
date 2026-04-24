use crate::cli::SshArgs;
use crate::error::AppError;
use crate::session::exec::{ensure_instance_running, resolve_connection, run_host_command};
use crate::types::SessionName;
use std::path::Path;

pub fn run(args: SshArgs) -> Result<(), AppError> {
    let session_name_raw = args.session.session.resolve_owned()?;
    let session_name = SessionName::try_from(session_name_raw.as_str())?;
    let connection = resolve_connection(&session_name)?;
    if args.session.json {
        println!(
            "{}",
            serde_json::json!({
                "ssh_config_file": connection.ssh_config_file,
                "host_alias": connection.host_alias,
            })
        );
        return Ok(());
    }
    ensure_instance_running(&connection)?;
    let ssh_args = build_ssh_passthrough_args(
        &connection.ssh_config_file,
        &connection.host_alias,
        args.forward_ssh_agent,
    );
    run_host_command("ssh", &ssh_args)?;
    Ok(())
}

pub(crate) fn build_ssh_passthrough_args(
    ssh_config_file: &Path,
    host_alias: &str,
    forward_ssh_agent: bool,
) -> Vec<String> {
    let mut ssh_args = vec!["-F".to_owned(), ssh_config_file.display().to_string()];
    if forward_ssh_agent {
        ssh_args.push("-A".to_owned());
    }
    ssh_args.push(host_alias.to_owned());
    ssh_args
}

#[cfg(test)]
mod tests {
    use super::build_ssh_passthrough_args;
    use std::path::Path;

    #[test]
    fn build_ssh_passthrough_args_toggles_agent_forwarding() {
        let enabled = build_ssh_passthrough_args(Path::new("/tmp/ssh-config"), "alias", true);
        assert_eq!(enabled, vec!["-F", "/tmp/ssh-config", "-A", "alias"]);

        let disabled = build_ssh_passthrough_args(Path::new("/tmp/ssh-config"), "alias", false);
        assert_eq!(disabled, vec!["-F", "/tmp/ssh-config", "alias"]);
        assert!(!disabled.iter().any(|arg| arg == "-t"));
    }
}
