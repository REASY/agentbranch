use crate::types::{GuestPath, ProviderKind, SessionName};
use std::path::{Path, PathBuf};

pub fn guest_home_dir(host_home_dir: &Path) -> PathBuf {
    let user = host_home_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("agbranch");
    Path::new("/home").join(format!("{user}.guest"))
}

pub fn sandbox_workspace_path(host_home_dir: &Path, session: &SessionName) -> GuestPath {
    GuestPath::new(
        guest_home_dir(host_home_dir)
            .join("sandbox")
            .join(session.as_str()),
    )
}

pub fn repo_workspace_path(host_home_dir: &Path, session: &SessionName) -> GuestPath {
    GuestPath::new(
        guest_home_dir(host_home_dir)
            .join("workspaces")
            .join(session.as_str())
            .join("repo"),
    )
}

pub fn tmux_socket_path(host_home_dir: &Path, session: &SessionName) -> GuestPath {
    GuestPath::new(
        guest_home_dir(host_home_dir)
            .join(".agbranch")
            .join("tmux")
            .join(format!("{session}.sock")),
    )
}

pub fn agent_auth_env_path(host_home_dir: &Path, session: &SessionName) -> GuestPath {
    GuestPath::new(
        guest_home_dir(host_home_dir)
            .join(".agbranch")
            .join("secrets")
            .join(session.as_str())
            .join("agent.env"),
    )
}

pub fn guest_home_path(host_home_dir: &Path) -> GuestPath {
    GuestPath::new(guest_home_dir(host_home_dir))
}

pub fn agbranch_home_path(host_home_dir: &Path) -> GuestPath {
    GuestPath::new(guest_home_dir(host_home_dir).join(".agbranch"))
}

pub fn shellenv_path(host_home_dir: &Path) -> GuestPath {
    GuestPath::new(
        agbranch_home_path(host_home_dir)
            .as_path()
            .join("shellenv.sh"),
    )
}

pub fn provider_shim_dir_path(host_home_dir: &Path) -> GuestPath {
    GuestPath::new(agbranch_home_path(host_home_dir).as_path().join("bin"))
}

pub fn claude_settings_path(host_home_dir: &Path) -> GuestPath {
    GuestPath::new(
        guest_home_dir(host_home_dir)
            .join(".claude")
            .join("settings.json"),
    )
}

pub fn claude_global_state_path(host_home_dir: &Path) -> GuestPath {
    GuestPath::new(guest_home_dir(host_home_dir).join(".claude.json"))
}

pub fn codex_config_path(host_home_dir: &Path) -> GuestPath {
    GuestPath::new(
        guest_home_dir(host_home_dir)
            .join(".codex")
            .join("config.toml"),
    )
}

pub fn gemini_settings_path(host_home_dir: &Path) -> GuestPath {
    GuestPath::new(
        guest_home_dir(host_home_dir)
            .join(".gemini")
            .join("settings.json"),
    )
}

pub fn gemini_trusted_folders_path(host_home_dir: &Path) -> GuestPath {
    GuestPath::new(
        guest_home_dir(host_home_dir)
            .join(".gemini")
            .join("trustedFolders.json"),
    )
}

pub fn provider_shim_path(host_home_dir: &Path, provider: ProviderKind) -> GuestPath {
    provider_shim_path_from_guest_home(&guest_home_dir(host_home_dir), provider)
}

pub fn provider_shim_path_from_guest_home(
    guest_home_dir: &Path,
    provider: ProviderKind,
) -> GuestPath {
    GuestPath::new(
        guest_home_dir
            .join(".agbranch")
            .join("bin")
            .join(provider.as_str()),
    )
}

#[cfg(test)]
mod tests {
    use super::provider_shim_path_from_guest_home;
    use crate::types::ProviderKind;
    use std::path::Path;

    #[test]
    fn provider_shim_path_from_guest_home_does_not_rederive_guest_suffix() {
        let path = provider_shim_path_from_guest_home(
            Path::new("/home/abalaian.guest"),
            ProviderKind::Claude,
        );
        assert_eq!(
            path.as_path(),
            Path::new("/home/abalaian.guest/.agbranch/bin/claude")
        );
    }
}
