use crate::provider::registry::provider_spec;
use crate::types::GuestPath;
use crate::types::ProviderKind;

pub fn build_launch_command(kind: ProviderKind) -> Vec<String> {
    let spec = provider_spec(kind);
    let mut command = vec![spec.binary_name.to_owned()];
    command.extend(spec.unrestricted_args.iter().map(|arg| (*arg).to_owned()));
    command
}

pub fn build_session_shell_line(session: &str, workspace: &GuestPath) -> String {
    format!(
        "export AGBRANCH_SESSION={session} && source \"$HOME/.agbranch/shellenv.sh\" && cd {workspace} && exec ${{SHELL:-/bin/bash}} -l",
        session = crate::lima::shell::shell_escape(session),
        workspace = crate::lima::shell::shell_escape(&workspace.to_string()),
    )
}

pub fn build_agent_shell_line(session: &str, workspace: &GuestPath, kind: ProviderKind) -> String {
    let command = vec![provider_spec(kind).binary_name.to_owned()]
        .into_iter()
        .map(|part| crate::lima::shell::shell_escape(&part))
        .collect::<Vec<_>>()
        .join(" ");
    let line = format!(
        "export AGBRANCH_SESSION={session} && source \"$HOME/.agbranch/shellenv.sh\" && cd {workspace}",
        session = crate::lima::shell::shell_escape(session),
        workspace = crate::lima::shell::shell_escape(&workspace.to_string()),
    );
    format!("{line} && exec {command}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_command_starts_with_provider_binary_and_unrestricted_flags() {
        let command = build_launch_command(ProviderKind::Codex);
        assert_eq!(command.first().map(String::as_str), Some("codex"));
        assert_eq!(
            command,
            vec![
                "codex".to_owned(),
                "--dangerously-bypass-approvals-and-sandbox".to_owned(),
            ]
        );
    }
}
