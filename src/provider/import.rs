use crate::lima::copy;
use crate::provider::registry::provider_spec;
use crate::types::{GuestPath, HostPath, ProviderKind, VmName};
use crate::util::process::CommandRunner;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportedProviderFile {
    pub host_path: HostPath,
    pub guest_path: GuestPath,
}

pub fn plan_imported_files(
    kind: ProviderKind,
    home_dir: &Path,
    guest_home: &GuestPath,
) -> Vec<ImportedProviderFile> {
    provider_spec(kind)
        .files
        .into_iter()
        .map(|entry| {
            let host_path = expand_tilde(entry.host_path.as_path(), home_dir);
            let guest_path = guest_home.as_path().join(entry.guest_relative_path);
            (host_path, guest_path)
        })
        .filter(|(host_path, _)| {
            std::fs::metadata(host_path)
                .map(|metadata| metadata.is_file())
                .unwrap_or(false)
        })
        .map(|(host_path, guest_path)| ImportedProviderFile {
            host_path: HostPath::new(host_path),
            guest_path: GuestPath::new(guest_path),
        })
        .collect()
}

pub fn detect_host_files(kind: ProviderKind, home_dir: &Path) -> Vec<HostPath> {
    plan_imported_files(kind, home_dir, &GuestPath::new("/guest"))
        .into_iter()
        .map(|entry| entry.host_path)
        .collect()
}

pub fn import_provider_files(
    runner: &dyn CommandRunner,
    kind: ProviderKind,
    home_dir: &Path,
    instance_name: &VmName,
    guest_home: &GuestPath,
) -> Result<Vec<ImportedProviderFile>, crate::error::lima::LimaError> {
    let imports = plan_imported_files(kind, home_dir, guest_home);
    for entry in &imports {
        copy::copy_host_file_to_guest(runner, &entry.host_path, instance_name, &entry.guest_path)?;
    }
    Ok(imports)
}

fn expand_tilde(path: &Path, home_dir: &Path) -> PathBuf {
    let rendered = path.to_string_lossy();
    if rendered == "~" {
        return home_dir.to_path_buf();
    }
    if let Some(rest) = rendered.strip_prefix("~/") {
        return home_dir.join(rest);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detect_host_files_skips_directories_even_when_allowlisted_path_exists() {
        let home = tempdir().expect("tempdir");
        let codex_dir = home.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).expect("codex dir");
        std::fs::write(codex_dir.join("auth.json"), "{\"token\":\"x\"}").expect("auth file");
        std::fs::create_dir_all(codex_dir.join("config.toml")).expect("config dir");

        let files = detect_host_files(ProviderKind::Codex, home.path());
        let paths = files
            .iter()
            .map(|path| {
                path.as_path()
                    .strip_prefix(home.path())
                    .unwrap()
                    .to_path_buf()
            })
            .collect::<Vec<_>>();

        assert_eq!(paths, vec![std::path::PathBuf::from(".codex/auth.json")]);
    }

    #[test]
    fn imported_provider_files_land_in_standard_guest_home_locations() {
        let home = tempdir().expect("tempdir");
        let codex_dir = home.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).expect("codex dir");
        std::fs::write(codex_dir.join("auth.json"), "{\"token\":\"x\"}").expect("auth file");

        let imports = plan_imported_files(
            ProviderKind::Codex,
            home.path(),
            &GuestPath::new("/home/tester.guest"),
        );
        let guest_paths = imports
            .iter()
            .map(|entry| entry.guest_path.as_path().to_path_buf())
            .collect::<Vec<PathBuf>>();

        assert_eq!(
            guest_paths,
            vec![PathBuf::from("/home/tester.guest/.codex/auth.json"),]
        );
    }
}
