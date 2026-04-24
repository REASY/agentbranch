use crate::types::ProviderKind;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfigPath(PathBuf);

impl ProviderConfigPath {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct ProviderFileSpec {
    pub host_path: ProviderConfigPath,
    pub guest_relative_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ProviderSpec {
    pub files: Vec<ProviderFileSpec>,
    pub binary_name: &'static str,
    pub version_args: &'static [&'static str],
    pub unrestricted_args: &'static [&'static str],
}

pub fn provider_spec(kind: ProviderKind) -> ProviderSpec {
    match kind {
        ProviderKind::Codex => ProviderSpec {
            files: vec![ProviderFileSpec {
                host_path: ProviderConfigPath::new("~/.codex/auth.json"),
                guest_relative_path: PathBuf::from(".codex/auth.json"),
            }],
            binary_name: "codex",
            version_args: &["--version"],
            unrestricted_args: &["--dangerously-bypass-approvals-and-sandbox"],
        },
        ProviderKind::Claude => ProviderSpec {
            files: vec![],
            binary_name: "claude",
            version_args: &["--version"],
            unrestricted_args: &["--dangerously-skip-permissions"],
        },
        ProviderKind::Gemini => ProviderSpec {
            files: vec![],
            binary_name: "gemini",
            version_args: &["--version"],
            unrestricted_args: &["--approval-mode=yolo"],
        },
    }
}

pub fn supported_providers() -> [ProviderKind; 3] {
    [
        ProviderKind::Codex,
        ProviderKind::Claude,
        ProviderKind::Gemini,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_auth_allowlist_is_exact() {
        let spec = provider_spec(ProviderKind::Codex);
        let paths = spec
            .files
            .iter()
            .map(|entry| entry.host_path.as_path())
            .collect::<Vec<_>>();

        assert_eq!(paths, vec![Path::new("~/.codex/auth.json")]);

        let guest_targets = spec
            .files
            .iter()
            .map(|entry| entry.guest_relative_path.as_path())
            .collect::<Vec<_>>();
        assert_eq!(guest_targets, vec![Path::new(".codex/auth.json")]);
    }

    #[test]
    fn provider_launch_presets_are_unrestricted() {
        assert_eq!(
            provider_spec(ProviderKind::Codex).unrestricted_args,
            &["--dangerously-bypass-approvals-and-sandbox"]
        );
        assert_eq!(
            provider_spec(ProviderKind::Claude).unrestricted_args,
            &["--dangerously-skip-permissions"]
        );
        assert_eq!(
            provider_spec(ProviderKind::Gemini).unrestricted_args,
            &["--approval-mode=yolo"]
        );
    }
}
