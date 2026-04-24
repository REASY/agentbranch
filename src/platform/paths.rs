use crate::platform::detect::HostPlatform;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateRoots {
    pub base: PathBuf,
    pub logs: PathBuf,
    pub staging: PathBuf,
    pub locks: PathBuf,
    pub db: PathBuf,
}

impl StateRoots {
    pub fn from_base(base: &Path) -> Self {
        Self {
            logs: base.join("logs"),
            staging: base.join("staging"),
            locks: base.join("locks"),
            db: base.join("state.db"),
            base: base.to_path_buf(),
        }
    }

    pub fn from_parts(
        platform: HostPlatform,
        home_dir: &Path,
        xdg_state_home: Option<&Path>,
    ) -> Self {
        let base = match platform {
            HostPlatform::Macos => home_dir.join("Library/Application Support/agbranch"),
            HostPlatform::Linux => xdg_state_home
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home_dir.join(".local/state"))
                .join("agbranch"),
        };

        Self {
            logs: base.join("logs"),
            staging: base.join("staging"),
            locks: base.join("locks"),
            db: base.join("state.db"),
            base,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_paths_use_application_support() {
        let roots = StateRoots::from_parts(HostPlatform::Macos, Path::new("/Users/alice"), None);

        assert_eq!(
            roots.base,
            Path::new("/Users/alice/Library/Application Support/agbranch")
        );
        assert_eq!(
            roots.db,
            Path::new("/Users/alice/Library/Application Support/agbranch/state.db")
        );
    }

    #[test]
    fn linux_paths_use_xdg_state_home_when_present() {
        let roots = StateRoots::from_parts(
            HostPlatform::Linux,
            Path::new("/home/alice"),
            Some(Path::new("/var/tmp/state")),
        );

        assert_eq!(roots.base, Path::new("/var/tmp/state/agbranch"));
        assert_eq!(roots.logs, Path::new("/var/tmp/state/agbranch/logs"));
    }

    #[test]
    fn custom_state_root_is_used_verbatim() {
        let roots = StateRoots::from_base(Path::new("/tmp/agbranch-smoke-state"));

        assert_eq!(roots.base, Path::new("/tmp/agbranch-smoke-state"));
        assert_eq!(roots.db, Path::new("/tmp/agbranch-smoke-state/state.db"));
        assert_eq!(roots.logs, Path::new("/tmp/agbranch-smoke-state/logs"));
    }
}
