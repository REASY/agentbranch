use crate::error::sync::SyncError;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_PATCH_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchChangeKind {
    Add,
    Modify,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchEntryKind {
    File,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchEntry {
    pub relative_path: PathBuf,
    pub change: PatchChangeKind,
    pub kind: PatchEntryKind,
}

pub fn rewrite_patch_headers(raw_patch: &str, old_root: &Path, new_root: &Path) -> String {
    raw_patch
        .lines()
        .map(|line| rewrite_line(line, old_root, new_root))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn export_patch_from_entries(
    host_root: &Path,
    staging_root: &Path,
    entries: &[PatchEntry],
    output_path: &Path,
) -> Result<(), SyncError> {
    let scratch = temp_patch_root()?;
    let old_tree = scratch.join("old");
    let new_tree = scratch.join("new");
    fs::create_dir_all(&old_tree).map_err(|source| SyncError::Io {
        path: old_tree.clone(),
        source,
    })?;
    fs::create_dir_all(&new_tree).map_err(|source| SyncError::Io {
        path: new_tree.clone(),
        source,
    })?;

    for entry in entries {
        if !matches!(entry.change, PatchChangeKind::Add) {
            materialize_path(
                host_root,
                &old_tree,
                &entry.relative_path,
                entry.kind.clone(),
            )?;
        }
        if !matches!(entry.change, PatchChangeKind::Delete) {
            materialize_path(
                staging_root,
                &new_tree,
                &entry.relative_path,
                entry.kind.clone(),
            )?;
        }
    }

    let output = Command::new("git")
        .current_dir(&scratch)
        .arg("diff")
        .arg("--no-index")
        .arg("--binary")
        .arg("--src-prefix=a/")
        .arg("--dst-prefix=b/")
        .arg("old")
        .arg("new")
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

    let raw_patch = String::from_utf8(output.stdout).map_err(|err| SyncError::PatchExport {
        message: err.to_string(),
    })?;
    let rewritten = rewrite_patch_headers(&raw_patch, Path::new("old"), Path::new("new"));
    fs::write(output_path, format!("{rewritten}\n")).map_err(|source| SyncError::Io {
        path: output_path.to_path_buf(),
        source,
    })?;
    let _ = fs::remove_dir_all(scratch);
    Ok(())
}

fn rewrite_line(line: &str, old_root: &Path, new_root: &Path) -> String {
    if let Some(rest) = line.strip_prefix("diff --git ") {
        let mut parts = rest.split_whitespace();
        let old = parts.next().unwrap_or_default();
        let new = parts.next().unwrap_or_default();
        return format!(
            "diff --git {} {}",
            rewrite_git_path(old, "a/", &[old_root, new_root]),
            rewrite_git_path(new, "b/", &[old_root, new_root])
        );
    }
    if let Some(path) = line.strip_prefix("--- ") {
        if path == "/dev/null" {
            return line.to_owned();
        }
        return format!(
            "--- {}",
            rewrite_git_path(path, "a/", &[old_root, new_root])
        );
    }
    if let Some(path) = line.strip_prefix("+++ ") {
        if path == "/dev/null" {
            return line.to_owned();
        }
        return format!(
            "+++ {}",
            rewrite_git_path(path, "b/", &[old_root, new_root])
        );
    }
    line.to_owned()
}

fn rewrite_git_path(value: &str, prefix: &str, roots: &[&Path]) -> String {
    let Some(raw) = value.strip_prefix(prefix) else {
        return value.to_owned();
    };
    let mut stripped = Path::new(raw);
    for root in roots {
        if let Ok(candidate) = stripped.strip_prefix(root) {
            stripped = candidate;
            break;
        }
    }
    format!("{prefix}{}", stripped.display())
}

fn materialize_path(
    source_root: &Path,
    target_root: &Path,
    relative_path: &Path,
    kind: PatchEntryKind,
) -> Result<(), SyncError> {
    let source = source_root.join(relative_path);
    let destination = target_root.join(relative_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|source| SyncError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    match kind {
        PatchEntryKind::File => {
            fs::copy(&source, &destination).map_err(|source_err| SyncError::Io {
                path: destination.clone(),
                source: source_err,
            })?;
        }
        PatchEntryKind::Symlink => {
            let target = fs::read_link(&source).map_err(|source_err| SyncError::Io {
                path: source.clone(),
                source: source_err,
            })?;
            create_symlink(&target, &destination)?;
        }
    }
    Ok(())
}

fn temp_patch_root() -> Result<PathBuf, SyncError> {
    let pid = std::process::id();
    for _ in 0..1024 {
        let unique = NEXT_TEMP_PATCH_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("agbranch-patch-{pid}-{unique}"));
        match fs::create_dir(&root) {
            Ok(()) => return Ok(root),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(SyncError::Io {
                    path: root.clone(),
                    source,
                });
            }
        }
    }

    Err(SyncError::PatchExport {
        message: "failed to allocate unique temporary patch root".to_owned(),
    })
}

#[cfg(unix)]
fn create_symlink(target: &Path, destination: &Path) -> Result<(), SyncError> {
    std::os::unix::fs::symlink(target, destination).map_err(|source| SyncError::Io {
        path: destination.to_path_buf(),
        source,
    })
}

#[cfg(windows)]
fn create_symlink(target: &Path, destination: &Path) -> Result<(), SyncError> {
    std::os::windows::fs::symlink_file(target, destination).map_err(|source| SyncError::Io {
        path: destination.to_path_buf(),
        source,
    })
}
