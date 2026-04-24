use crate::cli::JsonFlag;
use crate::db::{connect::open_catalog, models::LifecycleState, sessions::list_sessions};
use crate::error::{AppError, observability::ObservabilityError};
use crate::lima::instance::{delete_instance, list_instances, unprotect_instance};
use crate::platform::detect::HostPlatform;
use crate::platform::host::HostContext;
use crate::util::ids::prepared_base_name;
use crate::util::process::RealCommandRunner;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct GcReport {
    pub reclaimed_paths: Vec<PathBuf>,
    pub bytes_reclaimed: u64,
    pub warnings: Vec<String>,
}

pub fn collect_reclaimable_paths(staging_root: &Path) -> Vec<PathBuf> {
    if !staging_root.exists() {
        return Vec::new();
    }

    let Ok(entries) = fs::read_dir(staging_root) else {
        return Vec::new();
    };

    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

pub fn run(args: JsonFlag) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let active_sessions = active_session_names(&host)?;
    let mut warnings = Vec::new();
    let mut paths = collect_reclaimable_paths(&host.state_roots.staging)
        .into_iter()
        .filter(|path| should_reclaim_session_path(path, &active_sessions, &mut warnings))
        .collect::<Vec<_>>();
    paths.extend(collect_log_paths(
        &host.state_roots.logs,
        &active_sessions,
        &mut warnings,
    ));
    collect_obsolete_prepared_bases(host.platform, &mut warnings)?;
    let mut bytes_reclaimed = 0_u64;

    for path in &paths {
        bytes_reclaimed += path_size(path)?;
        remove_path(path)?;
    }

    let report = GcReport {
        reclaimed_paths: paths,
        bytes_reclaimed,
        warnings,
    };
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(ObservabilityError::from)?
        );
    } else if report.reclaimed_paths.is_empty() {
        println!("gc: nothing to reclaim");
    } else {
        println!(
            "gc reclaimed {} path(s), {} bytes",
            report.reclaimed_paths.len(),
            report.bytes_reclaimed
        );
        for path in &report.reclaimed_paths {
            println!("{}", path.display());
        }
        for warning in &report.warnings {
            eprintln!("warning: {warning}");
        }
    }
    Ok(())
}

fn active_session_names(host: &HostContext) -> Result<BTreeSet<String>, AppError> {
    if !host.state_roots.db.exists() {
        return Ok(BTreeSet::new());
    }
    let conn = open_catalog(&host.state_roots.db).map_err(ObservabilityError::from)?;
    let active = list_sessions(&conn)
        .map_err(ObservabilityError::from)?
        .into_iter()
        .filter(|row| row.lifecycle_state != LifecycleState::Closed)
        .map(|row| row.name.to_string())
        .collect();
    Ok(active)
}

fn should_reclaim_session_path(
    path: &Path,
    active_sessions: &BTreeSet<String>,
    warnings: &mut Vec<String>,
) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    if active_sessions.contains(name) {
        warnings.push(format!(
            "skipped active session staging/log path `{}`",
            path.display()
        ));
        return false;
    }
    true
}

fn collect_log_paths(
    logs_root: &Path,
    active_sessions: &BTreeSet<String>,
    warnings: &mut Vec<String>,
) -> Vec<PathBuf> {
    collect_reclaimable_paths(logs_root)
        .into_iter()
        .filter(|path| should_reclaim_session_path(path, active_sessions, warnings))
        .collect()
}

fn collect_obsolete_prepared_bases(
    platform: HostPlatform,
    warnings: &mut Vec<String>,
) -> Result<(), AppError> {
    let current_base = prepared_base_name(platform);
    let runner = RealCommandRunner;
    let instances = match list_instances(&runner) {
        Ok(instances) => instances,
        Err(err) => {
            warnings.push(format!("failed to inspect prepared bases: {err}"));
            return Ok(());
        }
    };

    for instance in instances {
        if !instance.name.starts_with("agbranch-base-") || instance.name == current_base.as_str() {
            continue;
        }
        let vm_name = crate::types::VmName::new(instance.name.clone());
        if let Err(err) = unprotect_instance(&runner, &vm_name) {
            warnings.push(format!(
                "failed to unprotect obsolete base `{}`: {err}",
                vm_name
            ));
            continue;
        }
        if let Err(err) = delete_instance(&runner, &vm_name) {
            warnings.push(format!(
                "failed to delete obsolete base `{}`: {err}",
                vm_name
            ));
        }
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<(), ObservabilityError> {
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|source| ObservabilityError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    } else if path.exists() {
        fs::remove_file(path).map_err(|source| ObservabilityError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn path_size(path: &Path) -> Result<u64, ObservabilityError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| ObservabilityError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }

    let mut total = 0_u64;
    for entry in fs::read_dir(path).map_err(|source| ObservabilityError::Io {
        path: path.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| ObservabilityError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        total += path_size(&entry.path())?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn gc_reports_old_staging_directories() {
        let dir = tempdir().expect("tempdir");
        let staging_root = dir.path().join("staging");
        let path = staging_root.join("feat-a");
        std::fs::create_dir_all(&path).expect("create staging dir");

        let reclaimed = collect_reclaimable_paths(&staging_root);
        assert_eq!(reclaimed, vec![path]);
    }
}
