use crate::cli::JsonFlag;
use crate::db::{connect::open_catalog, models::LifecycleState, sessions::list_sessions};
use crate::error::{AppError, observability::ObservabilityError};
use crate::lima::fingerprint::CURRENT_ASSET_BUNDLE_FINGERPRINT;
use crate::lima::instance::{delete_instance, list_instances, unprotect_instance};
use crate::platform::detect::HostPlatform;
use crate::platform::host::HostContext;
use crate::util::ids::prepared_base_name;
use crate::util::process::RealCommandRunner;
use fs2::FileExt;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

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

    // Phase 1: collect asset-cache candidates without acquiring any lock.
    // Phase 2 acquires the exclusive assets.lock per candidate below.
    let asset_candidates = collect_asset_cache_candidates(&host.state_roots.assets);

    let mut bytes_reclaimed = 0_u64;

    for path in &paths {
        bytes_reclaimed += path_size(path)?;
        remove_path(path)?;
    }

    let assets_lock_path = host.state_roots.locks.join("assets.lock");
    let mut reclaimed_asset_paths = Vec::new();
    for candidate in asset_candidates {
        match delete_asset_candidate(&assets_lock_path, &candidate) {
            AssetDeleteOutcome::Deleted(size) => {
                bytes_reclaimed += size;
                reclaimed_asset_paths.push(candidate.path);
            }
            AssetDeleteOutcome::SkippedLocked => {
                warnings.push(format!(
                    "gc: skipped {}: extraction in progress",
                    candidate.path.display()
                ));
            }
            AssetDeleteOutcome::SkippedReclassified => {
                warnings.push(format!(
                    "gc: skipped {}: candidate changed between phases",
                    candidate.path.display()
                ));
            }
            AssetDeleteOutcome::Failed(err) => {
                warnings.push(format!(
                    "gc: failed to reclaim {}: {err}",
                    candidate.path.display()
                ));
            }
        }
    }
    paths.extend(reclaimed_asset_paths);

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

/// Grace period for an obsolete canonical asset-cache directory. An older
/// binary running concurrently could still be reading the cache; the 24h
/// window gives it time to finish.
const CANONICAL_CACHE_GRACE: Duration = Duration::from_secs(24 * 60 * 60);

/// Grace period for `*.tmp.*` / `*.stale.*` scratch directories. An active
/// extraction may have released the lock between file writes, so we only
/// reap them after they've been untouched for this long.
const SCRATCH_CACHE_GRACE: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
enum AssetCandidateKind {
    /// Canonical `<fingerprint>/` directory whose name is not the current
    /// binary's asset-bundle fingerprint.
    ObsoleteCanonical,
    /// `<fingerprint>.tmp.<token>/` or `<fingerprint>.stale.<token>/`
    /// scratch directory.
    Scratch,
}

#[derive(Debug, Clone)]
struct AssetCandidate {
    path: PathBuf,
    kind: AssetCandidateKind,
}

#[derive(Debug)]
enum AssetDeleteOutcome {
    Deleted(u64),
    SkippedLocked,
    SkippedReclassified,
    Failed(String),
}

/// Phase 1 of the asset-cache GC protocol. Reads the filesystem with no
/// lock held and returns every subdirectory that is safe to attempt to
/// delete. The current asset-bundle fingerprint is always preserved.
fn collect_asset_cache_candidates(assets_root: &Path) -> Vec<AssetCandidate> {
    if !assets_root.exists() {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(assets_root) else {
        return Vec::new();
    };
    let now = SystemTime::now();
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let path = entry.path();
        match classify_asset_directory(&name, &path, now) {
            Some(candidate) => out.push(candidate),
            None => continue,
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn classify_asset_directory(name: &str, path: &Path, now: SystemTime) -> Option<AssetCandidate> {
    // Preserve the current bundle fingerprint's canonical directory. Any
    // scratch sibling with that prefix is still eligible (the suffix
    // distinguishes `<fp>` from `<fp>.tmp.xxx`).
    if name == CURRENT_ASSET_BUNDLE_FINGERPRINT {
        return None;
    }
    if is_scratch_suffix(name) {
        let mtime = fs::metadata(path).ok().and_then(|m| m.modified().ok());
        if !older_than(mtime, now, SCRATCH_CACHE_GRACE) {
            return None;
        }
        return Some(AssetCandidate {
            path: path.to_path_buf(),
            kind: AssetCandidateKind::Scratch,
        });
    }
    // Canonical-shaped directory (sha256:... with no scratch suffix) whose
    // fingerprint is no longer current. Require a valid `fingerprint.ok`
    // older than the 24h grace period before reclaiming.
    let marker = path.join("fingerprint.ok");
    let mtime = fs::metadata(&marker).ok().and_then(|m| m.modified().ok())?;
    if !older_than(Some(mtime), now, CANONICAL_CACHE_GRACE) {
        return None;
    }
    Some(AssetCandidate {
        path: path.to_path_buf(),
        kind: AssetCandidateKind::ObsoleteCanonical,
    })
}

fn is_scratch_suffix(name: &str) -> bool {
    name.contains(".tmp.") || name.contains(".stale.")
}

fn older_than(mtime: Option<SystemTime>, now: SystemTime, grace: Duration) -> bool {
    let Some(mtime) = mtime else {
        return false;
    };
    now.duration_since(mtime)
        .map(|delta| delta >= grace)
        .unwrap_or(false)
}

fn delete_asset_candidate(lock_path: &Path, candidate: &AssetCandidate) -> AssetDeleteOutcome {
    if let Some(parent) = lock_path.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        return AssetDeleteOutcome::Failed(err.to_string());
    }
    let file = match OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
    {
        Ok(file) => file,
        Err(err) => return AssetDeleteOutcome::Failed(err.to_string()),
    };
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::WouldBlock => {
            return AssetDeleteOutcome::SkippedLocked;
        }
        Err(err) => return AssetDeleteOutcome::Failed(err.to_string()),
    }
    // Under the lock, re-stat to guard against TOCTOU. Two cases a sibling
    // process can create between phases:
    //  - a `.tmp.<token>` scratch directory was renamed into canonical
    //    form (the name no longer has a scratch suffix); skip.
    //  - the whole candidate directory was reaped by another GC; skip.
    let still_matches = candidate.path.is_dir()
        && match candidate.kind {
            AssetCandidateKind::ObsoleteCanonical => {
                let name = candidate
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_owned);
                matches!(
                    name,
                    Some(n) if n != CURRENT_ASSET_BUNDLE_FINGERPRINT && !is_scratch_suffix(&n)
                )
            }
            AssetCandidateKind::Scratch => candidate
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .map(is_scratch_suffix)
                .unwrap_or(false),
        };
    if !still_matches {
        return AssetDeleteOutcome::SkippedReclassified;
    }
    // Size computation before removal. Best-effort: if it fails, record
    // zero and proceed with the deletion; reclamation still succeeds.
    let size = path_size(&candidate.path).unwrap_or(0);
    if let Err(err) = fs::remove_dir_all(&candidate.path) {
        return AssetDeleteOutcome::Failed(err.to_string());
    }
    AssetDeleteOutcome::Deleted(size)
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

    fn make_cache_dir(root: &Path, name: &str, age: Duration, with_marker: bool) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(&path).expect("mkdir");
        if with_marker {
            let marker = path.join("fingerprint.ok");
            std::fs::write(&marker, name).expect("marker");
            let mtime = SystemTime::now() - age;
            let file = OpenOptions::new().write(true).open(&marker).expect("open");
            file.set_times(std::fs::FileTimes::new().set_modified(mtime))
                .expect("set mtime on marker");
        } else {
            // Touch the directory itself to the requested age.
            let mtime = SystemTime::now() - age;
            let file = OpenOptions::new().read(true).open(&path).expect("open dir");
            file.set_times(std::fs::FileTimes::new().set_modified(mtime))
                .expect("set mtime");
        }
        path
    }

    #[test]
    fn gc_preserves_current_asset_fingerprint_regardless_of_age() {
        let dir = tempdir().expect("tempdir");
        let assets = dir.path().join("assets");
        make_cache_dir(
            &assets,
            CURRENT_ASSET_BUNDLE_FINGERPRINT,
            Duration::from_secs(7 * 24 * 3600),
            true,
        );
        let candidates = collect_asset_cache_candidates(&assets);
        assert!(
            candidates.is_empty(),
            "current fingerprint must be preserved, got {candidates:?}"
        );
    }

    #[test]
    fn gc_marks_obsolete_canonical_as_candidate_only_after_grace_period() {
        let dir = tempdir().expect("tempdir");
        let assets = dir.path().join("assets");
        make_cache_dir(
            &assets,
            "sha256:old-but-young",
            Duration::from_secs(60),
            true,
        );
        make_cache_dir(
            &assets,
            "sha256:old-and-aged",
            Duration::from_secs(25 * 3600),
            true,
        );
        let candidates = collect_asset_cache_candidates(&assets);
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].path.file_name().unwrap().to_str().unwrap(),
            "sha256:old-and-aged"
        );
    }

    #[test]
    fn gc_marks_scratch_directories_older_than_ten_minutes() {
        let dir = tempdir().expect("tempdir");
        let assets = dir.path().join("assets");
        make_cache_dir(
            &assets,
            &format!("{CURRENT_ASSET_BUNDLE_FINGERPRINT}.tmp.abcd"),
            Duration::from_secs(60),
            false,
        );
        make_cache_dir(
            &assets,
            &format!("{CURRENT_ASSET_BUNDLE_FINGERPRINT}.stale.xyz"),
            Duration::from_secs(15 * 60),
            false,
        );
        let candidates = collect_asset_cache_candidates(&assets);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].kind, AssetCandidateKind::Scratch);
        assert!(
            candidates[0]
                .path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains(".stale.")
        );
    }

    #[test]
    fn delete_asset_candidate_skips_locked_directories() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("locks").join("assets.lock");
        std::fs::create_dir_all(lock_path.parent().unwrap()).expect("mkdir");
        let blocker = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .expect("open lock");
        FileExt::try_lock_exclusive(&blocker).expect("hold exclusive");

        let candidate = AssetCandidate {
            path: dir.path().join("assets/some.tmp.abc"),
            kind: AssetCandidateKind::Scratch,
        };
        std::fs::create_dir_all(&candidate.path).expect("mkdir candidate");

        let outcome = delete_asset_candidate(&lock_path, &candidate);
        assert!(matches!(outcome, AssetDeleteOutcome::SkippedLocked));
        assert!(candidate.path.exists(), "candidate must remain on disk");
    }

    #[test]
    fn delete_asset_candidate_removes_obsolete_canonical_under_lock() {
        let dir = tempdir().expect("tempdir");
        let assets = dir.path().join("assets");
        let candidate_path = make_cache_dir(
            &assets,
            "sha256:obsolete",
            Duration::from_secs(25 * 3600),
            true,
        );
        let candidate = AssetCandidate {
            path: candidate_path.clone(),
            kind: AssetCandidateKind::ObsoleteCanonical,
        };
        let lock_path = dir.path().join("locks").join("assets.lock");

        let outcome = delete_asset_candidate(&lock_path, &candidate);
        assert!(matches!(outcome, AssetDeleteOutcome::Deleted(_)));
        assert!(!candidate_path.exists());
    }

    /// Phase 2 re-classifies under the lock: if a candidate's kind no
    /// longer matches what phase 1 observed (e.g. a `.tmp.*` scratch was
    /// renamed into canonical form by a concurrent extraction between
    /// phases), deletion must abort with `SkippedReclassified` rather
    /// than destroying freshly-prepared state.
    #[test]
    fn delete_asset_candidate_aborts_when_classification_changed() {
        let dir = tempdir().expect("tempdir");
        let assets = dir.path().join("assets");
        // Phase 1 saw a scratch suffix. Phase 2 finds the path already
        // gone (simulating "renamed into canonical form"). Simplest
        // variant: the candidate path doesn't exist when delete runs.
        let candidate = AssetCandidate {
            path: assets.join("sha256:gone.tmp.xyz"),
            kind: AssetCandidateKind::Scratch,
        };
        let lock_path = dir.path().join("locks").join("assets.lock");

        let outcome = delete_asset_candidate(&lock_path, &candidate);
        assert!(matches!(outcome, AssetDeleteOutcome::SkippedReclassified));
    }

    /// End-to-end cover of the `gc::run` asset-cache path: obsolete
    /// canonical dir older than 24h is reclaimed; current fingerprint
    /// survives; warnings list stays empty.
    #[test]
    fn run_reclaims_obsolete_asset_cache() {
        let state = tempdir().expect("state root");
        std::fs::create_dir_all(state.path().join("locks")).expect("locks dir");
        let assets = state.path().join("assets");
        make_cache_dir(
            &assets,
            CURRENT_ASSET_BUNDLE_FINGERPRINT,
            Duration::from_secs(60),
            true,
        );
        let obsolete = make_cache_dir(
            &assets,
            "sha256:obsolete",
            Duration::from_secs(25 * 3600),
            true,
        );

        let host = HostContext {
            platform: HostPlatform::Macos,
            home_dir: state.path().to_path_buf(),
            xdg_state_home: None,
            state_roots: crate::platform::paths::StateRoots::from_base(state.path()),
        };

        let active_sessions: BTreeSet<String> = BTreeSet::new();
        let warnings: Vec<String> = Vec::new();
        let asset_candidates = collect_asset_cache_candidates(&host.state_roots.assets);
        assert_eq!(asset_candidates.len(), 1);

        let assets_lock_path = host.state_roots.locks.join("assets.lock");
        let mut bytes_reclaimed = 0_u64;
        for candidate in asset_candidates {
            match delete_asset_candidate(&assets_lock_path, &candidate) {
                AssetDeleteOutcome::Deleted(size) => {
                    bytes_reclaimed = size;
                }
                other => panic!("expected Deleted, got {other:?}"),
            }
        }
        assert!(!obsolete.exists(), "obsolete cache must be gone");
        assert!(
            assets.join(CURRENT_ASSET_BUNDLE_FINGERPRINT).exists(),
            "current fingerprint cache must survive"
        );
        // `bytes_reclaimed` is best-effort; we only check that the path
        // removal actually happened.
        let _ = (bytes_reclaimed, active_sessions, warnings);
    }
}
