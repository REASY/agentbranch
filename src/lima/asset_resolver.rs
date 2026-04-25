//! Resolver and extractor for the embedded Lima asset bundle.
//!
//! Resolution order on first call in a process:
//!
//! 1. `AGBRANCH_LIMA_ASSETS_DIR` — if set and the directory is complete,
//!    use it verbatim and compute a runtime provision fingerprint from
//!    its contents.
//! 2. `<state-root>/assets/<bundle-fingerprint>/` — accepted if
//!    `fingerprint.ok` matches and the tree passes the complete-tree and
//!    mtime guards.
//! 3. Extract the embedded bundle into the state-root cache.
//!
//! Subsequent calls in the same process hit a keyed cache. The resolver
//! is the sole source of the `lima/` directory path for runtime code;
//! nothing in `src/` may read `CARGO_MANIFEST_DIR`, and the source-tree
//! grep test enforces that.

use crate::error::{AppError, ValidationError};
use crate::lima::assets::EMBEDDED_LIMA_ASSETS;
use crate::lima::fingerprint::{
    CURRENT_ASSET_BUNDLE_FINGERPRINT, CURRENT_PROVISION_FINGERPRINT, ProvisionFingerprintSource,
    compute_effective_provision_fingerprint,
};
use crate::platform::host::HostContext;
use fs2::FileExt;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// Environment variable that overrides the resolved `lima/` directory.
pub const LIMA_ASSETS_DIR_ENV: &str = "AGBRANCH_LIMA_ASSETS_DIR";

/// Where the resolver produced the returned directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimaAssetOrigin {
    /// `AGBRANCH_LIMA_ASSETS_DIR` pointed at a complete tree. Populated by
    /// phase 5.
    EnvOverride,
    /// The state-root cache was used. `extracted_this_call` is true only
    /// for the single call that performed extraction on a cold cache.
    StateRootCache { extracted_this_call: bool },
}

/// The resolved `lima/` directory plus the fingerprint every consumer must
/// use for base-freshness decisions. `effective_provision_fingerprint` is
/// always populated; callers should not branch on `origin` to decide which
/// fingerprint is authoritative.
#[derive(Debug, Clone)]
pub struct LimaAssetDir {
    pub path: PathBuf,
    pub origin: LimaAssetOrigin,
    pub effective_provision_fingerprint: String,
}

/// Selects the mutating resolver for a given `HostContext`, extracting the
/// embedded bundle on first use and reusing the cache on subsequent calls.
/// Phase 5 will add override-path branching before the cache check; for now
/// the resolver only supports the cache path.
pub fn lima_asset_dir(host: &HostContext) -> Result<Arc<LimaAssetDir>, AppError> {
    let key = ResolverKey::from_host(host);
    if let Some(cached) = resolver_cache_get(&key) {
        return Ok(cached);
    }

    let resolved = match key.override_dir.as_deref() {
        Some(override_dir) => resolve_override(override_dir)?,
        None => resolve_uncached(host)?,
    };
    Ok(resolver_cache_insert(key, resolved))
}

/// Clears the process-scoped resolver cache. Test-only so tests that swap
/// `HostContext` values can force re-resolution without constructing a
/// fresh `ResolverKey`.
#[cfg(test)]
pub fn reset_lima_asset_resolver_cache() {
    let cell = resolver_cache();
    cell.lock().expect("resolver cache mutex").clear();
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ResolverKey {
    state_root: PathBuf,
    // Populated once phase 5 reads AGBRANCH_LIMA_ASSETS_DIR. Today always
    // None because override is not yet implemented.
    override_dir: Option<PathBuf>,
}

impl ResolverKey {
    fn from_host(host: &HostContext) -> Self {
        Self {
            state_root: host.state_roots.base.clone(),
            override_dir: read_override_env(),
        }
    }
}

fn read_override_env() -> Option<PathBuf> {
    std::env::var_os(LIMA_ASSETS_DIR_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

fn resolver_cache() -> &'static Mutex<HashMap<ResolverKey, Arc<LimaAssetDir>>> {
    static CELL: OnceLock<Mutex<HashMap<ResolverKey, Arc<LimaAssetDir>>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn resolver_cache_get(key: &ResolverKey) -> Option<Arc<LimaAssetDir>> {
    resolver_cache()
        .lock()
        .expect("resolver cache mutex")
        .get(key)
        .cloned()
}

fn resolver_cache_insert(key: ResolverKey, resolved: LimaAssetDir) -> Arc<LimaAssetDir> {
    // The cached entry reports extracted_this_call: false for every
    // subsequent reader; the extracting call already consumed that signal.
    let cached_origin = match resolved.origin {
        LimaAssetOrigin::StateRootCache { .. } => LimaAssetOrigin::StateRootCache {
            extracted_this_call: false,
        },
        other => other,
    };
    let for_cache = LimaAssetDir {
        origin: cached_origin,
        ..resolved.clone()
    };
    let mut guard = resolver_cache().lock().expect("resolver cache mutex");
    let arc = Arc::new(for_cache);
    guard.entry(key).or_insert_with(|| arc.clone());
    // Return the fresh resolution (with the true extracted_this_call flag)
    // rather than the cached copy; subsequent calls hit the cached entry.
    Arc::new(resolved)
}

fn resolve_override(override_dir: &Path) -> Result<LimaAssetDir, AppError> {
    if !override_dir.is_dir() {
        return Err(AppError::Validation(
            ValidationError::LimaAssetsOverrideNotADirectory {
                path: override_dir.to_path_buf(),
            },
        ));
    }

    // Share the richer complete-tree check with the inspector so `base
    // show` and `base prepare` classify the same override tree the same
    // way: symlinks, wrong modes, wrong file types all surface by name
    // rather than collapsing into a generic "missing" bucket.
    let reasons = crate::lima::asset_inspect::scan_tree(override_dir);
    if !reasons.is_empty() {
        let rendered = reasons
            .iter()
            .map(crate::lima::asset_inspect::describe_override_invalid_reason)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(AppError::Validation(
            ValidationError::LimaAssetsOverrideIncomplete {
                path: override_dir.to_path_buf(),
                missing: rendered,
            },
        ));
    }

    let fingerprint =
        compute_effective_provision_fingerprint(ProvisionFingerprintSource::OverrideTree {
            lima_root: override_dir,
        })?;

    Ok(LimaAssetDir {
        path: override_dir.to_path_buf(),
        origin: LimaAssetOrigin::EnvOverride,
        effective_provision_fingerprint: fingerprint,
    })
}

fn resolve_uncached(host: &HostContext) -> Result<LimaAssetDir, AppError> {
    // Phase 4: cache + embedded extraction only.
    let canonical = canonical_cache_dir(host);
    let lima_dir = canonical.join("lima");

    if cache_is_acceptable(&canonical, CURRENT_ASSET_BUNDLE_FINGERPRINT) {
        return Ok(LimaAssetDir {
            path: lima_dir,
            origin: LimaAssetOrigin::StateRootCache {
                extracted_this_call: false,
            },
            effective_provision_fingerprint: CURRENT_PROVISION_FINGERPRINT.to_owned(),
        });
    }

    ensure_assets_parent_dirs(host)?;
    let lock_path = assets_lock_path(host);
    let _lock = AssetsLock::acquire_exclusive(&lock_path)?;

    // Re-check under the lock. Another process may have finished extracting.
    if cache_is_acceptable(&canonical, CURRENT_ASSET_BUNDLE_FINGERPRINT) {
        return Ok(LimaAssetDir {
            path: lima_dir,
            origin: LimaAssetOrigin::StateRootCache {
                extracted_this_call: false,
            },
            effective_provision_fingerprint: CURRENT_PROVISION_FINGERPRINT.to_owned(),
        });
    }

    extract_embedded_bundle(&host.state_roots.assets, &canonical)?;

    Ok(LimaAssetDir {
        path: lima_dir,
        origin: LimaAssetOrigin::StateRootCache {
            extracted_this_call: true,
        },
        effective_provision_fingerprint: CURRENT_PROVISION_FINGERPRINT.to_owned(),
    })
}

fn canonical_cache_dir(host: &HostContext) -> PathBuf {
    host.state_roots.assets.join(bundle_fingerprint_for_dirname(
        CURRENT_ASSET_BUNDLE_FINGERPRINT,
    ))
}

fn bundle_fingerprint_for_dirname(fingerprint: &str) -> &str {
    // Fingerprints include a colon (sha256:...), which is valid on the
    // filesystems we target. Return as-is so the cache directory matches
    // the stamped value byte-for-byte.
    fingerprint
}

fn assets_lock_path(host: &HostContext) -> PathBuf {
    host.state_roots.locks.join("assets.lock")
}

fn ensure_assets_parent_dirs(host: &HostContext) -> io::Result<()> {
    std::fs::create_dir_all(&host.state_roots.assets)?;
    std::fs::create_dir_all(&host.state_roots.locks)?;
    Ok(())
}

fn cache_is_acceptable(canonical: &Path, expected_fingerprint: &str) -> bool {
    let lima_dir = canonical.join("lima");
    if !lima_dir.is_dir() {
        return false;
    }
    let marker = canonical.join("fingerprint.ok");
    let Ok(fingerprint_contents) = std::fs::read_to_string(&marker) else {
        return false;
    };
    if fingerprint_contents.trim_end_matches('\n') != expected_fingerprint {
        return false;
    }

    if !complete_tree_check(&lima_dir) {
        return false;
    }
    if !mtime_guard_passes(&marker, &lima_dir) {
        return false;
    }
    true
}

fn complete_tree_check(lima_dir: &Path) -> bool {
    for asset in EMBEDDED_LIMA_ASSETS {
        let path = lima_dir.join(asset.relative_path);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => return false,
        };
        if !meta.file_type().is_file() {
            return false;
        }
        if !mode_bits_ok(&meta, asset.relative_path) {
            return false;
        }
    }
    true
}

#[cfg(unix)]
fn mode_bits_ok(meta: &std::fs::Metadata, relative_path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode();
    if relative_path.ends_with(".sh") {
        (mode & 0o500) == 0o500
    } else {
        (mode & 0o400) == 0o400
    }
}

#[cfg(not(unix))]
fn mode_bits_ok(_meta: &std::fs::Metadata, _relative_path: &str) -> bool {
    true
}

fn mtime_guard_passes(marker: &Path, lima_dir: &Path) -> bool {
    let Ok(marker_mtime) = mtime_of(marker) else {
        return false;
    };
    let newest = match newest_mtime(lima_dir) {
        Ok(Some(mtime)) => mtime,
        _ => return true,
    };
    // Accept if the marker is the newest file, or is within 2 seconds of
    // the newest file. A file mtime newer than the marker by more than
    // that window means a human edited the cache since extraction.
    let grace = std::time::Duration::from_secs(2);
    marker_mtime + grace >= newest
}

fn mtime_of(path: &Path) -> io::Result<SystemTime> {
    std::fs::metadata(path).and_then(|meta| meta.modified())
}

fn newest_mtime(dir: &Path) -> io::Result<Option<SystemTime>> {
    let mut newest: Option<SystemTime> = None;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let path = entry.path();
        let candidate = if ft.is_dir() {
            newest_mtime(&path)?.unwrap_or_else(SystemTime::now)
        } else {
            std::fs::metadata(&path)?.modified()?
        };
        newest = Some(match newest {
            Some(current) if current >= candidate => current,
            _ => candidate,
        });
    }
    Ok(newest)
}

fn extract_embedded_bundle(assets_root: &Path, canonical: &Path) -> io::Result<()> {
    std::fs::create_dir_all(assets_root)?;

    // Scratch directory with collision-resistant suffix. tempfile embeds
    // randomness, so PID reuse after a crash cannot collide.
    let scratch = tempfile::Builder::new()
        .prefix(&format!(
            "{}.tmp.",
            canonical
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("bundle"),
        ))
        .tempdir_in(assets_root)?;
    let scratch_path = scratch.keep();
    let scratch_lima = scratch_path.join("lima");
    std::fs::create_dir_all(&scratch_lima)?;

    for asset in EMBEDDED_LIMA_ASSETS {
        let target = scratch_lima.join(asset.relative_path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_asset_file(&target, asset.bytes, asset.relative_path)?;
    }

    // fingerprint.ok is written last so partial extractions are never
    // mistaken for valid caches.
    let marker = scratch_path.join("fingerprint.ok");
    let mut marker_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&marker)?;
    marker_file.write_all(CURRENT_ASSET_BUNDLE_FINGERPRINT.as_bytes())?;
    marker_file.flush()?;
    marker_file.sync_all()?;

    commit_scratch_into_canonical(&scratch_path, canonical)?;
    Ok(())
}

fn write_asset_file(path: &Path, bytes: &[u8], relative_path: &str) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if relative_path.ends_with(".sh") {
            0o755
        } else {
            0o644
        };
        let mut perms = file.metadata()?.permissions();
        perms.set_mode(mode);
        file.set_permissions(perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = relative_path;
    }
    Ok(())
}

fn commit_scratch_into_canonical(scratch: &Path, canonical: &Path) -> io::Result<()> {
    // Simple case: canonical did not exist when we started; rename wins.
    if !canonical.exists() {
        std::fs::rename(scratch, canonical)?;
        return Ok(());
    }

    // Stale case: canonical existed but failed acceptance. Unlink its
    // fingerprint first so any concurrent reader immediately sees the
    // cache as invalid, then rename-aside, rename-into-place, and remove
    // the stale directory best-effort.
    let _ = std::fs::remove_file(canonical.join("fingerprint.ok"));

    let stale = stale_sibling_path(canonical)?;
    std::fs::rename(canonical, &stale)?;
    std::fs::rename(scratch, canonical)?;
    let _ = std::fs::remove_dir_all(&stale);
    Ok(())
}

fn stale_sibling_path(canonical: &Path) -> io::Result<PathBuf> {
    let assets_root = canonical
        .parent()
        .ok_or_else(|| io::Error::other("canonical path has no parent"))?;
    let base_name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bundle");
    // tempfile::Builder gives us randomness plus guaranteed absence of a
    // colliding directory.
    let holder = tempfile::Builder::new()
        .prefix(&format!("{base_name}.stale."))
        .tempdir_in(assets_root)?;
    let path = holder.keep();
    // The holder created an empty directory. Remove it so rename-into
    // works (rename on a non-empty target fails on Linux).
    std::fs::remove_dir(&path)?;
    Ok(path)
}

/// File-lock primitive for `<state-root>/locks/assets.lock`. Intentionally
/// separate from `SessionLock` (which hard-wires session lifecycle
/// metadata); this one only needs exclusive/nonblocking behavior.
struct AssetsLock {
    _file: File,
}

impl AssetsLock {
    fn acquire_exclusive(path: &Path) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(Self { _file: file }),
            Err(err) if err.kind() == ErrorKind::WouldBlock => Err(AppError::Blocked(format!(
                "lima asset extraction is busy: lock `{}` is held",
                path.display(),
            ))),
            Err(err) => Err(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::detect::HostPlatform;
    use crate::platform::paths::StateRoots;
    use tempfile::tempdir;

    fn set_file_mtime(path: &Path, when: SystemTime) -> io::Result<()> {
        let file = OpenOptions::new().write(true).open(path)?;
        file.set_times(
            std::fs::FileTimes::new()
                .set_modified(when)
                .set_accessed(when),
        )
    }

    fn make_host(root: &Path) -> HostContext {
        HostContext {
            platform: HostPlatform::Macos,
            home_dir: root.to_path_buf(),
            xdg_state_home: None,
            state_roots: StateRoots::from_base(&root.join("state")),
        }
    }

    #[test]
    fn cold_cache_extracts_and_reports_extracted_this_call_true() {
        let _env = EnvTestGuard::noop();
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());

        let resolved = lima_asset_dir(&host).expect("resolve");
        assert!(matches!(
            resolved.origin,
            LimaAssetOrigin::StateRootCache {
                extracted_this_call: true
            }
        ));
        assert_eq!(
            resolved.effective_provision_fingerprint,
            CURRENT_PROVISION_FINGERPRINT
        );
        let lima_dir = resolved.path.clone();
        assert!(lima_dir.join("safe-sync-macos.yaml").is_file());
        assert!(lima_dir.join("provision/00-system.sh").is_file());
        assert!(lima_dir.join("guest/shellenv.sh").is_file());
    }

    #[test]
    fn warm_cache_reports_extracted_this_call_false() {
        let _env = EnvTestGuard::noop();
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());

        let _first = lima_asset_dir(&host).expect("first");
        // Clear the in-process cache so the second call re-reads disk and
        // proves the on-disk cache acceptance path short-circuits.
        reset_lima_asset_resolver_cache();
        let second = lima_asset_dir(&host).expect("second");
        assert!(matches!(
            second.origin,
            LimaAssetOrigin::StateRootCache {
                extracted_this_call: false
            }
        ));
    }

    #[test]
    fn cache_with_missing_asset_triggers_reextraction() {
        let _env = EnvTestGuard::noop();
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());
        let first = lima_asset_dir(&host).expect("first");
        let lima_dir = first.path.clone();
        std::fs::remove_file(lima_dir.join("guest/shellenv.sh")).expect("unlink");

        reset_lima_asset_resolver_cache();
        let second = lima_asset_dir(&host).expect("second");
        assert!(matches!(
            second.origin,
            LimaAssetOrigin::StateRootCache {
                extracted_this_call: true
            }
        ));
        assert!(second.path.join("guest/shellenv.sh").is_file());
    }

    #[test]
    fn cache_with_stale_fingerprint_triggers_reextraction() {
        let _env = EnvTestGuard::noop();
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());
        let first = lima_asset_dir(&host).expect("first");
        let canonical = first.path.parent().expect("canonical").to_path_buf();
        std::fs::write(canonical.join("fingerprint.ok"), "sha256:bogus").expect("poison");

        reset_lima_asset_resolver_cache();
        let second = lima_asset_dir(&host).expect("second");
        assert!(matches!(
            second.origin,
            LimaAssetOrigin::StateRootCache {
                extracted_this_call: true
            }
        ));
        let marker = canonical.join("fingerprint.ok");
        assert_eq!(
            std::fs::read_to_string(&marker)
                .expect("fingerprint.ok")
                .trim_end_matches('\n'),
            CURRENT_ASSET_BUNDLE_FINGERPRINT
        );
    }

    #[test]
    fn cache_with_newer_file_than_marker_triggers_reextraction() {
        let _env = EnvTestGuard::noop();
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());
        let first = lima_asset_dir(&host).expect("first");
        let shellenv = first.path.join("guest/shellenv.sh");

        // Bump the file mtime past the 2s grace window.
        let future = SystemTime::now() + std::time::Duration::from_secs(60);
        set_file_mtime(&shellenv, future).expect("set mtime");

        reset_lima_asset_resolver_cache();
        let second = lima_asset_dir(&host).expect("second");
        assert!(matches!(
            second.origin,
            LimaAssetOrigin::StateRootCache {
                extracted_this_call: true
            }
        ));
    }

    #[test]
    fn different_state_roots_are_resolved_independently() {
        let _env = EnvTestGuard::noop();
        let dir_a = tempdir().expect("tempdir a");
        let dir_b = tempdir().expect("tempdir b");
        let host_a = make_host(dir_a.path());
        let host_b = make_host(dir_b.path());

        let a = lima_asset_dir(&host_a).expect("a");
        let b = lima_asset_dir(&host_b).expect("b");
        assert_ne!(a.path, b.path);
    }

    #[cfg(unix)]
    #[test]
    fn extracted_shell_scripts_are_executable_and_yaml_is_readable() {
        use std::os::unix::fs::PermissionsExt;

        let _env = EnvTestGuard::noop();
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());
        let resolved = lima_asset_dir(&host).expect("resolve");
        let lima_dir = &resolved.path;

        let sh_mode = std::fs::metadata(lima_dir.join("provision/00-system.sh"))
            .expect("sh metadata")
            .permissions()
            .mode();
        assert_eq!(sh_mode & 0o500, 0o500, "got {sh_mode:o}");

        let yaml_mode = std::fs::metadata(lima_dir.join("safe-sync-macos.yaml"))
            .expect("yaml metadata")
            .permissions()
            .mode();
        assert_eq!(yaml_mode & 0o400, 0o400, "got {yaml_mode:o}");
    }

    #[cfg(unix)]
    #[test]
    fn complete_tree_check_rejects_sh_without_execute_bit() {
        use std::os::unix::fs::PermissionsExt;

        let _env = EnvTestGuard::noop();
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());
        let first = lima_asset_dir(&host).expect("first");
        let lima_dir = first.path.clone();

        let sh = lima_dir.join("provision/00-system.sh");
        let mut perms = std::fs::metadata(&sh).expect("sh meta").permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&sh, perms).expect("chmod");

        reset_lima_asset_resolver_cache();
        let second = lima_asset_dir(&host).expect("second");
        assert!(matches!(
            second.origin,
            LimaAssetOrigin::StateRootCache {
                extracted_this_call: true
            }
        ));
    }

    /// Serializes every resolver test, whether or not it touches the
    /// override env var. Without this, parallel tests clobber each other:
    /// one test sets `AGBRANCH_LIMA_ASSETS_DIR`, another sees it mid-run,
    /// and the whole suite goes red.
    fn env_test_mutex() -> &'static Mutex<()> {
        static CELL: OnceLock<Mutex<()>> = OnceLock::new();
        CELL.get_or_init(|| Mutex::new(()))
    }

    struct EnvTestGuard {
        _guard: std::sync::MutexGuard<'static, ()>,
        prior: Option<std::ffi::OsString>,
        touched: bool,
    }

    impl EnvTestGuard {
        /// Guard that leaves the override env alone — suitable for tests
        /// that exercise the cache path.
        fn noop() -> Self {
            // SAFETY: no env manipulation while `_guard` is held; see
            // `set` for the mutation path.
            let guard = env_test_mutex().lock().expect("env mutex");
            let prior = std::env::var_os(LIMA_ASSETS_DIR_ENV);
            // Clear any stray value from a prior test that may have
            // panicked before its own drop ran.
            unsafe {
                std::env::remove_var(LIMA_ASSETS_DIR_ENV);
            }
            reset_lima_asset_resolver_cache();
            Self {
                _guard: guard,
                prior,
                touched: true,
            }
        }

        fn set(value: &Path) -> Self {
            let guard = env_test_mutex().lock().expect("env mutex");
            let prior = std::env::var_os(LIMA_ASSETS_DIR_ENV);
            unsafe {
                std::env::set_var(LIMA_ASSETS_DIR_ENV, value);
            }
            reset_lima_asset_resolver_cache();
            Self {
                _guard: guard,
                prior,
                touched: true,
            }
        }
    }

    impl Drop for EnvTestGuard {
        fn drop(&mut self) {
            if !self.touched {
                return;
            }
            unsafe {
                match self.prior.take() {
                    Some(prev) => std::env::set_var(LIMA_ASSETS_DIR_ENV, prev),
                    None => std::env::remove_var(LIMA_ASSETS_DIR_ENV),
                }
            }
            reset_lima_asset_resolver_cache();
        }
    }

    fn make_override_lima_tree(dir: &Path) -> PathBuf {
        let lima = dir.join("lima");
        std::fs::create_dir_all(lima.join("provision")).expect("mkdir provision");
        std::fs::create_dir_all(lima.join("guest")).expect("mkdir guest");
        for asset in EMBEDDED_LIMA_ASSETS {
            let target = lima.join(asset.relative_path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(&target, asset.bytes).expect("write asset");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if asset.relative_path.ends_with(".sh") {
                    0o755
                } else {
                    0o644
                };
                let mut perms = std::fs::metadata(&target).unwrap().permissions();
                perms.set_mode(mode);
                std::fs::set_permissions(&target, perms).expect("chmod");
            }
        }
        lima
    }

    #[test]
    fn env_override_returns_env_override_origin_with_override_fingerprint() {
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());
        let lima = make_override_lima_tree(dir.path());
        let _guard = EnvTestGuard::set(&lima);

        let resolved = lima_asset_dir(&host).expect("override resolved");
        assert_eq!(resolved.origin, LimaAssetOrigin::EnvOverride);
        assert_eq!(resolved.path, lima);
        // The override tree is a byte-for-byte copy of the embedded bundle,
        // so the runtime fingerprint must equal the baked-in constant.
        assert_eq!(
            resolved.effective_provision_fingerprint,
            CURRENT_PROVISION_FINGERPRINT
        );
    }

    #[test]
    fn env_override_missing_directory_surfaces_validation_error() {
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());
        let nonexistent = dir.path().join("does-not-exist");
        let _guard = EnvTestGuard::set(&nonexistent);

        let err = lima_asset_dir(&host).expect_err("nonexistent must fail");
        assert!(matches!(
            err,
            AppError::Validation(ValidationError::LimaAssetsOverrideNotADirectory { .. })
        ));
    }

    #[test]
    fn env_override_incomplete_tree_reports_missing_paths() {
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());
        let lima = make_override_lima_tree(dir.path());
        std::fs::remove_file(lima.join("guest/shellenv.sh")).expect("unlink");
        let _guard = EnvTestGuard::set(&lima);

        let err = lima_asset_dir(&host).expect_err("incomplete must fail");
        let AppError::Validation(ValidationError::LimaAssetsOverrideIncomplete { missing, .. }) =
            err
        else {
            panic!("expected LimaAssetsOverrideIncomplete, got {err:?}");
        };
        assert!(missing.contains("guest/shellenv.sh"), "got: {missing}");
    }

    #[test]
    fn env_override_content_change_changes_effective_fingerprint() {
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());
        let lima = make_override_lima_tree(dir.path());
        let _guard = EnvTestGuard::set(&lima);

        let first = lima_asset_dir(&host).expect("first");
        let baseline = first.effective_provision_fingerprint.clone();
        drop(first);

        // Modify one provision script so the override fingerprint diverges.
        std::fs::write(
            lima.join("provision/00-system.sh"),
            b"#!/bin/sh\n# override edit\n",
        )
        .expect("edit");
        reset_lima_asset_resolver_cache();

        let second = lima_asset_dir(&host).expect("second");
        assert_ne!(second.effective_provision_fingerprint, baseline);
    }

    #[cfg(unix)]
    #[test]
    fn complete_tree_check_accepts_sh_at_0700() {
        use std::os::unix::fs::PermissionsExt;

        let _env = EnvTestGuard::noop();
        let dir = tempdir().expect("tempdir");
        let host = make_host(dir.path());
        let first = lima_asset_dir(&host).expect("first");
        let lima_dir = first.path.clone();

        // Touch the marker after the chmod so the mtime guard passes.
        let sh = lima_dir.join("provision/00-system.sh");
        let mut perms = std::fs::metadata(&sh).expect("sh meta").permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&sh, perms).expect("chmod");
        let marker = first
            .path
            .parent()
            .expect("canonical")
            .join("fingerprint.ok");
        set_file_mtime(
            &marker,
            SystemTime::now() + std::time::Duration::from_secs(5),
        )
        .expect("touch marker");

        reset_lima_asset_resolver_cache();
        let second = lima_asset_dir(&host).expect("second");
        assert!(matches!(
            second.origin,
            LimaAssetOrigin::StateRootCache {
                extracted_this_call: false
            }
        ));
    }
}
