use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};

pub const CURRENT_PROVISION_FINGERPRINT: &str = env!("AGBRANCH_PROVISION_FINGERPRINT");
pub const CURRENT_ASSET_BUNDLE_FINGERPRINT: &str = env!("AGBRANCH_ASSET_BUNDLE_FINGERPRINT");

pub const PROVISION_FINGERPRINT_SALT: &[u8] = b"agbranch-base-fingerprint-v1\0";
pub const ASSET_BUNDLE_FINGERPRINT_SALT: &[u8] = b"agbranch-asset-bundle-fingerprint-v1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintInput {
    pub path: String,
    pub bytes: Vec<u8>,
}

pub fn compute_provision_fingerprint<I>(inputs: I) -> Result<String, String>
where
    I: IntoIterator<Item = FingerprintInput>,
{
    compute_fingerprint(inputs, PROVISION_FINGERPRINT_SALT)
}

pub fn compute_asset_bundle_fingerprint<I>(inputs: I) -> Result<String, String>
where
    I: IntoIterator<Item = FingerprintInput>,
{
    compute_fingerprint(inputs, ASSET_BUNDLE_FINGERPRINT_SALT)
}

/// Source of truth for the base-provision fingerprint at runtime. Under
/// the default path we use the baked-in constant; under an override we
/// recompute from the override tree so that edits to provisioning inputs
/// actually change the fingerprint.
pub enum ProvisionFingerprintSource<'a> {
    BakedIn,
    /// Recompute from the `lima/` directory at `lima_root`. The directory
    /// must contain `safe-sync-*.yaml` and `provision/*.sh` files.
    OverrideTree {
        lima_root: &'a Path,
    },
}

/// Returns the base-provision fingerprint for the given source. This is
/// the single authority consulted by the resolver and inspector; no other
/// call site should compute the fingerprint independently.
pub fn compute_effective_provision_fingerprint(
    source: ProvisionFingerprintSource<'_>,
) -> io::Result<String> {
    match source {
        ProvisionFingerprintSource::BakedIn => Ok(CURRENT_PROVISION_FINGERPRINT.to_owned()),
        ProvisionFingerprintSource::OverrideTree { lima_root } => {
            let inputs = override_provision_inputs(lima_root)?;
            compute_provision_fingerprint(inputs)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
        }
    }
}

/// Collects the provision-fingerprint inputs from an override `lima/`
/// directory and returns them keyed on the `lima/...` relative path
/// convention `build.rs` uses, so a runtime-computed fingerprint over the
/// repo's own `lima/` tree equals the baked-in constant byte-for-byte.
fn override_provision_inputs(lima_root: &Path) -> io::Result<Vec<FingerprintInput>> {
    let mut paths: Vec<PathBuf> = vec![
        PathBuf::from("lima/safe-sync-macos.yaml"),
        PathBuf::from("lima/safe-sync-linux.yaml"),
    ];
    for path in discover_provision_scripts_relative_to_lima_root(lima_root)? {
        paths.push(path);
    }
    paths.sort_by(|left, right| {
        posix_repr(left)
            .as_bytes()
            .cmp(posix_repr(right).as_bytes())
    });

    let mut inputs = Vec::with_capacity(paths.len());
    for relative in paths {
        let bytes = read_relative_to_lima_root(lima_root, &relative)?;
        inputs.push(FingerprintInput {
            path: posix_repr(&relative),
            bytes,
        });
    }
    Ok(inputs)
}

fn read_relative_to_lima_root(lima_root: &Path, relative: &Path) -> io::Result<Vec<u8>> {
    // `relative` is rooted at `lima/…` to match build.rs. The caller's
    // `lima_root` already points at `…/lima`, so strip the `lima/` prefix
    // before joining.
    let stripped = relative
        .strip_prefix("lima")
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    std::fs::read(lima_root.join(stripped))
}

fn discover_provision_scripts_relative_to_lima_root(lima_root: &Path) -> io::Result<Vec<PathBuf>> {
    let provision_dir = lima_root.join("provision");
    let mut scripts = Vec::new();
    for entry in std::fs::read_dir(provision_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("sh") {
            continue;
        }
        scripts.push(PathBuf::from("lima/provision").join(entry.file_name()));
    }
    scripts.sort();
    Ok(scripts)
}

fn posix_repr(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn compute_fingerprint<I>(inputs: I, salt: &[u8]) -> Result<String, String>
where
    I: IntoIterator<Item = FingerprintInput>,
{
    let mut inputs = inputs.into_iter().collect::<Vec<_>>();
    inputs.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));

    let mut hasher = Sha256::new();
    hasher.update(salt);
    for input in inputs {
        if input.path.is_empty() {
            return Err("fingerprint input path must not be empty".to_owned());
        }
        let path_len = input.path.len().to_string();
        let content_len = input.bytes.len().to_string();
        hasher.update(path_len.as_bytes());
        hasher.update(b"\0");
        hasher.update(input.path.as_bytes());
        hasher.update(b"\0");
        hasher.update(content_len.as_bytes());
        hasher.update(b"\0");
        hasher.update(&input.bytes);
        hasher.update(b"\0");
    }

    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

pub fn discover_non_recursive_provision_scripts(root: &Path) -> io::Result<Vec<PathBuf>> {
    discover_non_recursive_scripts(root, "provision")
}

pub fn discover_non_recursive_guest_scripts(root: &Path) -> io::Result<Vec<PathBuf>> {
    discover_non_recursive_scripts(root, "guest")
}

fn discover_non_recursive_scripts(root: &Path, subdir: &str) -> io::Result<Vec<PathBuf>> {
    let dir = root.join("lima").join(subdir);
    let mut scripts = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("sh") {
            continue;
        }
        let name = entry.file_name();
        scripts.push(PathBuf::from("lima").join(subdir).join(name));
    }
    scripts.sort();
    Ok(scripts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn fingerprint_is_stable_across_input_order() {
        let a = FingerprintInput {
            path: "lima/provision/20-docker-compose.sh".to_owned(),
            bytes: b"compose".to_vec(),
        };
        let b = FingerprintInput {
            path: "lima/safe-sync-macos.yaml".to_owned(),
            bytes: b"template".to_vec(),
        };

        let first = compute_provision_fingerprint([a.clone(), b.clone()]).expect("fingerprint");
        let second = compute_provision_fingerprint([b, a]).expect("fingerprint");

        assert_eq!(first, second);
        assert!(first.starts_with("sha256:"));
    }

    #[test]
    fn fingerprint_changes_when_path_or_contents_change() {
        let base = compute_provision_fingerprint([FingerprintInput {
            path: "lima/provision/10-agent-clis.sh".to_owned(),
            bytes: b"agent".to_vec(),
        }])
        .expect("base");
        let content_changed = compute_provision_fingerprint([FingerprintInput {
            path: "lima/provision/10-agent-clis.sh".to_owned(),
            bytes: b"agent-v2".to_vec(),
        }])
        .expect("content changed");
        let path_changed = compute_provision_fingerprint([FingerprintInput {
            path: "lima/provision/11-agent-clis.sh".to_owned(),
            bytes: b"agent".to_vec(),
        }])
        .expect("path changed");

        assert_ne!(base, content_changed);
        assert_ne!(base, path_changed);
    }

    #[test]
    fn provision_scripts_are_discovered_non_recursively() {
        let dir = tempdir().expect("tempdir");
        let provision = dir.path().join("lima").join("provision");
        std::fs::create_dir_all(provision.join("nested")).expect("mkdir");
        std::fs::write(provision.join("00-system.sh"), "system").expect("write");
        std::fs::write(provision.join("README.md"), "ignore").expect("write");
        std::fs::write(provision.join("nested").join("99-nested.sh"), "nested").expect("write");

        let scripts = discover_non_recursive_provision_scripts(dir.path()).expect("discover");

        assert_eq!(scripts, vec![PathBuf::from("lima/provision/00-system.sh")]);
    }

    #[test]
    fn guest_scripts_are_discovered_non_recursively() {
        let dir = tempdir().expect("tempdir");
        let guest = dir.path().join("lima").join("guest");
        std::fs::create_dir_all(guest.join("nested")).expect("mkdir");
        std::fs::write(guest.join("shellenv.sh"), "env").expect("write");
        std::fs::write(guest.join("README.md"), "ignore").expect("write");
        std::fs::write(guest.join("nested").join("99-nested.sh"), "nested").expect("write");

        let scripts = discover_non_recursive_guest_scripts(dir.path()).expect("discover");

        assert_eq!(scripts, vec![PathBuf::from("lima/guest/shellenv.sh")]);
    }

    #[test]
    fn provision_and_asset_bundle_salts_differ_so_fingerprints_diverge() {
        let input = FingerprintInput {
            path: "lima/safe-sync-macos.yaml".to_owned(),
            bytes: b"template".to_vec(),
        };
        let provision =
            compute_provision_fingerprint([input.clone()]).expect("provision fingerprint");
        let bundle = compute_asset_bundle_fingerprint([input]).expect("bundle fingerprint");

        assert_ne!(
            provision, bundle,
            "distinct salts must produce distinct fingerprints for identical input sets"
        );
    }

    #[test]
    fn baked_in_fingerprints_are_well_formed_sha256() {
        for value in [
            CURRENT_PROVISION_FINGERPRINT,
            CURRENT_ASSET_BUNDLE_FINGERPRINT,
        ] {
            assert!(
                value.starts_with("sha256:"),
                "value `{value}` missing prefix"
            );
            let hex = &value["sha256:".len()..];
            assert_eq!(hex.len(), 64, "sha256 hex must be 64 chars, got `{hex}`");
            assert!(
                hex.chars().all(|c| c.is_ascii_hexdigit()),
                "hex must be ascii hex digits, got `{hex}`"
            );
        }
    }

    #[test]
    fn baked_in_provision_and_bundle_fingerprints_differ() {
        assert_ne!(
            CURRENT_PROVISION_FINGERPRINT, CURRENT_ASSET_BUNDLE_FINGERPRINT,
            "build.rs must compute distinct fingerprints for the two input sets"
        );
    }

    #[test]
    fn lima_provision_and_guest_discovery_cover_disjoint_sets() {
        // Disjoint-union invariant: the asset-bundle input set is exactly
        // the base-provision set plus lima/guest/*.sh. The two discovery
        // helpers must never overlap on paths.
        let dir = tempdir().expect("tempdir");
        let lima = dir.path().join("lima");
        std::fs::create_dir_all(lima.join("provision")).expect("mkdir provision");
        std::fs::create_dir_all(lima.join("guest")).expect("mkdir guest");
        std::fs::write(lima.join("provision").join("00-system.sh"), "system").expect("write");
        std::fs::write(lima.join("provision").join("10-agent-clis.sh"), "agent").expect("write");
        std::fs::write(lima.join("guest").join("shellenv.sh"), "env").expect("write");
        std::fs::write(lima.join("guest").join("scrub-artifacts.sh"), "scrub").expect("write");

        let provision = discover_non_recursive_provision_scripts(dir.path()).expect("provision");
        let guest = discover_non_recursive_guest_scripts(dir.path()).expect("guest");

        for path in &provision {
            assert!(
                !guest.contains(path),
                "path `{}` must not appear in both sets",
                path.display()
            );
        }
        assert!(!provision.is_empty(), "provision set must be non-empty");
        assert!(!guest.is_empty(), "guest set must be non-empty");
    }
}
