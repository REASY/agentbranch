use std::path::Path;

pub struct SshCommandSpec<'a> {
    pub ssh_config_file: &'a Path,
    pub host_alias: &'a str,
    pub session: &'a str,
    pub workdir: &'a Path,
    pub forward_agent: bool,
    pub force_tty: bool,
    pub guest_secret_file: Option<&'a Path>,
    pub command: Option<&'a [String]>,
}

pub fn build_ssh_command(spec: SshCommandSpec<'_>) -> Vec<String> {
    let mut args = vec!["-F".to_owned(), spec.ssh_config_file.display().to_string()];
    if spec.forward_agent {
        args.push("-A".to_owned());
    }
    if spec.force_tty || spec.command.is_none() {
        args.push("-t".to_owned());
    }
    args.push(spec.host_alias.to_owned());

    let mut bootstrap = format!(
        "cd {} && export AGBRANCH_SESSION={} && source \"$HOME/.agbranch/shellenv.sh\"",
        shell_escape(&spec.workdir.display().to_string()),
        shell_escape(spec.session),
    );
    if let Some(secret_file) = spec.guest_secret_file {
        let secret_file = shell_escape(&secret_file.display().to_string());
        bootstrap.push_str(&format!(
            " && if [ -f {secret_file} ]; then set -a && . {secret_file} && set +a; fi"
        ));
    }

    if let Some(command) = spec.command {
        bootstrap.push_str(" && exec ");
        bootstrap.push_str(
            &command
                .iter()
                .map(|part| shell_escape(part))
                .collect::<Vec<_>>()
                .join(" "),
        );
    } else {
        bootstrap.push_str(" && exec ${SHELL:-/bin/bash} -l");
    }

    args.push(bootstrap);
    args
}

pub fn shell_escape(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_command_bootstraps_workdir_session_and_shellenv() {
        let command = ["cargo".to_owned(), "test".to_owned()];
        let args = build_ssh_command(SshCommandSpec {
            ssh_config_file: Path::new("/tmp/ssh-config"),
            host_alias: "lima-agbranch-demo",
            session: "demo",
            workdir: Path::new("/home/user/workspaces/demo/repo"),
            forward_agent: false,
            force_tty: false,
            guest_secret_file: None,
            command: Some(&command),
        });

        assert_eq!(args[0], "-F");
        assert_eq!(args[1], "/tmp/ssh-config");
        assert_eq!(args[2], "lima-agbranch-demo");
        assert!(!args.iter().any(|arg| arg == "-t"));
        assert_eq!(
            args[3],
            "cd '/home/user/workspaces/demo/repo' && export AGBRANCH_SESSION='demo' && source \"$HOME/.agbranch/shellenv.sh\" && exec 'cargo' 'test'"
        );
    }

    #[test]
    fn ssh_command_enables_agent_forwarding_only_when_requested() {
        let args_with_forwarding = build_ssh_command(SshCommandSpec {
            ssh_config_file: Path::new("/tmp/ssh-config"),
            host_alias: "alias",
            session: "demo",
            workdir: Path::new("/work"),
            forward_agent: true,
            force_tty: false,
            guest_secret_file: None,
            command: None,
        });
        assert_eq!(
            args_with_forwarding,
            vec![
                "-F",
                "/tmp/ssh-config",
                "-A",
                "-t",
                "alias",
                "cd '/work' && export AGBRANCH_SESSION='demo' && source \"$HOME/.agbranch/shellenv.sh\" && exec ${SHELL:-/bin/bash} -l",
            ]
        );

        let args_without_forwarding = build_ssh_command(SshCommandSpec {
            ssh_config_file: Path::new("/tmp/ssh-config"),
            host_alias: "alias",
            session: "demo",
            workdir: Path::new("/work"),
            forward_agent: false,
            force_tty: false,
            guest_secret_file: None,
            command: None,
        });
        assert_eq!(
            args_without_forwarding,
            vec![
                "-F",
                "/tmp/ssh-config",
                "-t",
                "alias",
                "cd '/work' && export AGBRANCH_SESSION='demo' && source \"$HOME/.agbranch/shellenv.sh\" && exec ${SHELL:-/bin/bash} -l",
            ]
        );
    }

    #[test]
    fn ssh_command_sources_guest_secret_file_when_present() {
        let command = ["env".to_owned()];
        let args = build_ssh_command(SshCommandSpec {
            ssh_config_file: Path::new("/tmp/ssh-config"),
            host_alias: "alias",
            session: "demo",
            workdir: Path::new("/work"),
            forward_agent: false,
            force_tty: false,
            guest_secret_file: Some(Path::new("/home/user/.agbranch/secrets/demo/command.env")),
            command: Some(&command),
        });
        assert_eq!(args[0], "-F");
        assert_eq!(args[1], "/tmp/ssh-config");
        assert_eq!(args[2], "alias");
        assert!(!args.iter().any(|arg| arg == "-t"));
        assert_eq!(
            args[3],
            "cd '/work' && export AGBRANCH_SESSION='demo' && source \"$HOME/.agbranch/shellenv.sh\" && if [ -f '/home/user/.agbranch/secrets/demo/command.env' ]; then set -a && . '/home/user/.agbranch/secrets/demo/command.env' && set +a; fi && exec 'env'"
        );
    }

    #[test]
    fn ssh_command_can_force_tty_for_interactive_remote_command() {
        let command = [
            "bash".to_owned(),
            "-lc".to_owned(),
            "tmux attach-session -t demo".to_owned(),
        ];
        let args = build_ssh_command(SshCommandSpec {
            ssh_config_file: Path::new("/tmp/ssh-config"),
            host_alias: "alias",
            session: "demo",
            workdir: Path::new("/work"),
            forward_agent: false,
            force_tty: true,
            guest_secret_file: None,
            command: Some(&command),
        });

        assert_eq!(args[0], "-F");
        assert_eq!(args[1], "/tmp/ssh-config");
        assert_eq!(args[2], "-t");
        assert_eq!(args[3], "alias");
        assert_eq!(
            args[4],
            "cd '/work' && export AGBRANCH_SESSION='demo' && source \"$HOME/.agbranch/shellenv.sh\" && exec 'bash' '-lc' 'tmux attach-session -t demo'"
        );
    }
}
