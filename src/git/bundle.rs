use crate::error::AppError;
use crate::lima::copy::copy_guest_path_to_host;
use crate::types::{GuestPath, HostPath, VmName};
use crate::util::process::CommandRunner;
use std::collections::BTreeMap;

pub fn create_seed_bundle(
    runner: &dyn CommandRunner,
    repo_root: &HostPath,
    base_ref: &str,
    bundle_path: &HostPath,
) -> Result<(), AppError> {
    runner.run(
        "git",
        &[
            "-C".to_owned(),
            repo_root.to_string(),
            "bundle".to_owned(),
            "create".to_owned(),
            bundle_path.to_string(),
            base_ref.to_owned(),
        ],
        None,
        &BTreeMap::new(),
    )?;
    Ok(())
}

pub fn guest_repo_is_dirty(
    runner: &dyn CommandRunner,
    vm_name: &VmName,
    guest_repo_path: &GuestPath,
) -> Result<bool, AppError> {
    let output = runner.run(
        "limactl",
        &[
            "shell".to_owned(),
            vm_name.as_str().to_owned(),
            "--".to_owned(),
            "bash".to_owned(),
            "-lc".to_owned(),
            format!(
                "git -C {} status --porcelain",
                crate::lima::shell::shell_escape(&guest_repo_path.to_string())
            ),
        ],
        None,
        &BTreeMap::new(),
    )?;
    Ok(output.stdout.lines().next().is_some())
}

pub fn create_guest_sync_bundle(
    runner: &dyn CommandRunner,
    vm_name: &VmName,
    guest_repo_path: &GuestPath,
    revision: &str,
    host_bundle_path: &HostPath,
) -> Result<(), AppError> {
    let guest_bundle_path = GuestPath::new("/tmp/agbranch-sync.bundle");
    runner.run(
        "limactl",
        &[
            "shell".to_owned(),
            vm_name.as_str().to_owned(),
            "--".to_owned(),
            "bash".to_owned(),
            "-lc".to_owned(),
            format!(
                "rm -f {bundle} && git -C {repo} bundle create {bundle} {revision}",
                bundle = crate::lima::shell::shell_escape(&guest_bundle_path.to_string()),
                repo = crate::lima::shell::shell_escape(&guest_repo_path.to_string()),
                revision = crate::lima::shell::shell_escape(revision),
            ),
        ],
        None,
        &BTreeMap::new(),
    )?;
    copy_guest_path_to_host(runner, vm_name, &guest_bundle_path, host_bundle_path)?;
    Ok(())
}

pub fn guest_head_ref(
    runner: &dyn CommandRunner,
    vm_name: &VmName,
    guest_repo_path: &GuestPath,
) -> Result<Option<String>, AppError> {
    let output = runner.run(
        "limactl",
        &[
            "shell".to_owned(),
            vm_name.as_str().to_owned(),
            "--".to_owned(),
            "bash".to_owned(),
            "-lc".to_owned(),
            format!(
                "git -C {} symbolic-ref --quiet HEAD",
                crate::lima::shell::shell_escape(&guest_repo_path.to_string())
            ),
        ],
        None,
        &BTreeMap::new(),
    );
    match output {
        Ok(output) => Ok(Some(output.stdout.trim().to_owned())),
        Err(crate::error::process::ProcessError::Failed { status: 1, .. }) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

pub fn fetch_bundle_ref(
    runner: &dyn CommandRunner,
    repo_root: &HostPath,
    bundle_path: &HostPath,
    source_ref: &str,
    destination_ref: &str,
) -> Result<(), AppError> {
    runner.run(
        "git",
        &[
            "fetch".to_owned(),
            "--quiet".to_owned(),
            bundle_path.to_string(),
            format!("{source_ref}:{destination_ref}"),
        ],
        Some(repo_root.as_path()),
        &BTreeMap::new(),
    )?;
    Ok(())
}
