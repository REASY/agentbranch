use crate::cli::ExportArgs;
use crate::commands::{find_existing_session, resolve_session_name};
use crate::db::connect::open_catalog;
use crate::error::{AppError, ValidationError};
use crate::lima::copy;
use crate::platform::host::HostContext;
use crate::types::{GuestPath, HostPath};
use crate::util::process::RealCommandRunner;
use std::path::Path;

pub fn validate_export_paths(
    from_guest_path: &str,
    to_host_path: &Path,
    force: bool,
) -> Result<(), AppError> {
    if !from_guest_path.starts_with("~/sandbox/") {
        return Err(AppError::Validation(
            ValidationError::ExportPathOutsideSandbox,
        ));
    }
    if to_host_path
        .components()
        .any(|component| component.as_os_str() == ".git")
    {
        return Err(AppError::Validation(
            ValidationError::ExportDestinationInsideGit,
        ));
    }
    if to_host_path.exists() && !force {
        return Err(AppError::Validation(
            ValidationError::ExportDestinationExists {
                path: to_host_path.display().to_string(),
            },
        ));
    }
    Ok(())
}

fn resolve_guest_export_path(
    from_guest_path: &str,
    guest_workspace_path: &GuestPath,
) -> Result<GuestPath, AppError> {
    let guest_home = guest_workspace_path
        .as_path()
        .parent()
        .and_then(Path::parent)
        .ok_or(ValidationError::ExportGuestHomeDeriveFailure)?;
    let relative = from_guest_path
        .strip_prefix('~')
        .expect("validated export paths should start with ~")
        .trim_start_matches('/');
    Ok(GuestPath::new(guest_home.join(relative)))
}

pub fn run(args: ExportArgs) -> Result<(), AppError> {
    let (session_name_raw, session_name) = resolve_session_name(&args.session)?;
    let host = HostContext::detect()?;
    validate_export_paths(&args.from_guest_path, &args.to_host_path, args.force)?;
    let conn = open_catalog(&host.state_roots.db)?;
    let session = find_existing_session(&conn, &session_name, &session_name_raw)?;

    if session.session_mode != crate::types::SessionMode::Sandbox {
        return Err(AppError::Validation(ValidationError::ExportRequiresSandbox));
    }

    let guest_path =
        resolve_guest_export_path(&args.from_guest_path, &session.guest_workspace_path)?;
    let host_path = HostPath::new(args.to_host_path);
    copy::copy_guest_path_to_host(
        &RealCommandRunner,
        &session.vm_name,
        &guest_path,
        &host_path,
    )?;

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "session": session_name,
                "from": guest_path,
                "to": host_path,
            })
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::GuestPath;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn export_resolves_tilde_against_session_guest_home() {
        let guest_path = resolve_guest_export_path(
            "~/sandbox/research/report.md",
            &GuestPath::new("/home/tester.guest/sandbox/research"),
        )
        .expect("guest path");

        assert_eq!(
            guest_path.as_path(),
            Path::new("/home/tester.guest/sandbox/research/report.md")
        );
    }

    #[test]
    fn export_rejects_existing_destination_without_force() {
        let tmp = tempdir().expect("tempdir");
        let dst = tmp.path().join("report.md");
        std::fs::write(&dst, "existing").expect("write");

        let err =
            validate_export_paths("~/sandbox/research/report.md", &dst, false).expect_err("reject");
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn export_rejects_git_destination() {
        let tmp = tempdir().expect("tempdir");
        let git_dir = tmp.path().join(".git");
        std::fs::create_dir_all(&git_dir).expect("git dir");
        let dst = git_dir.join("HEAD");

        let err =
            validate_export_paths("~/sandbox/research/report.md", &dst, true).expect_err("reject");
        assert!(err.to_string().contains(".git"));
    }
}
