use crate::cli::SyncBackArgs;
use crate::error::AppError;
use crate::error::sync::SyncError;
use crate::types::HostPath;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct SyncBackOutcome {
    pub blocked: bool,
    pub patch_path: Option<PathBuf>,
    pub staged_path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BlockedSyncArtifacts {
    pub(crate) staged_path: Option<PathBuf>,
    pub(crate) patch_path: Option<PathBuf>,
}

pub(crate) fn prepare_blocked_sync_artifacts(
    args: &SyncBackArgs,
    host_git_root: &Path,
    current_head: &str,
    fetched_head: &str,
    bundle_path: &HostPath,
) -> Result<BlockedSyncArtifacts, SyncError> {
    let patch_path = if let Some(path) = args.export_patch.as_ref() {
        export_patch_for_ref_range(host_git_root, current_head, fetched_head, path)?;
        Some(path.clone())
    } else {
        None
    };

    Ok(BlockedSyncArtifacts {
        staged_path: Some(bundle_path.as_path().to_path_buf()),
        patch_path,
    })
}

fn export_patch_for_ref_range(
    repo_root: &Path,
    current_head: &str,
    fetched_head: &str,
    output_path: &Path,
) -> Result<(), SyncError> {
    if let Some(parent) = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| SyncError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let output = Command::new("git")
        .current_dir(repo_root)
        .arg("diff")
        .arg("--binary")
        .arg("--src-prefix=a/")
        .arg("--dst-prefix=b/")
        .arg(current_head)
        .arg(fetched_head)
        .output()
        .map_err(|source| SyncError::Io {
            path: output_path.to_path_buf(),
            source,
        })?;

    let status = output.status.code().unwrap_or(1);
    if status != 0 && status != 1 {
        return Err(SyncError::GitDiffFailed {
            status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    fs::write(output_path, output.stdout).map_err(|source| SyncError::Io {
        path: output_path.to_path_buf(),
        source,
    })?;
    Ok(())
}

pub(crate) fn format_sync_back_outcome(
    outcome: &SyncBackOutcome,
    json: bool,
) -> Result<String, AppError> {
    if json {
        serde_json::to_string_pretty(outcome)
            .map_err(crate::error::observability::ObservabilityError::from)
            .map_err(AppError::Observability)
    } else {
        Ok(format!(
            "sync-back applied from {}",
            outcome.staged_path.display()
        ))
    }
}

pub(crate) fn emit_sync_back_output(output: String) -> Result<(), AppError> {
    println!("{output}");
    Ok(())
}
