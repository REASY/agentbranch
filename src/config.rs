use crate::error::config::ConfigError;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    #[serde(default)]
    pub defaults: Option<RepoDefaults>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoDefaults {
    #[serde(default)]
    pub env_files: Vec<PathBuf>,
}

pub fn load_repo_config(repo_root: &Path) -> Result<RepoConfig, ConfigError> {
    let path = repo_root.join("agbranch.toml");
    if !path.exists() {
        return Ok(RepoConfig::default());
    }

    let contents = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
        path: path.clone(),
        source,
    })?;

    toml::from_str(&contents).map_err(|source| ConfigError::ParseToml { path, source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn private_network_guard_setting_is_rejected() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("agbranch.toml"),
            "[defaults]\nprivate_network_guard = false\n",
        )
        .expect("write config");

        let err =
            load_repo_config(dir.path()).expect_err("private_network_guard should be rejected");
        assert!(
            err.to_string()
                .contains("unknown field `private_network_guard`")
        );
    }

    #[test]
    fn removed_tests_section_is_rejected() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("agbranch.toml"),
            "[tests.rust]\ncommand = [\"cargo\", \"test\", \"--workspace\"]\n",
        )
        .expect("write config");

        let err = load_repo_config(dir.path()).expect_err("tests config should be rejected");
        assert!(err.to_string().contains("unknown field `tests`"));
    }
}
