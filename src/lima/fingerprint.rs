use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};

pub const CURRENT_PROVISION_FINGERPRINT: &str = env!("AGBRANCH_PROVISION_FINGERPRINT");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintInput {
    pub path: String,
    pub bytes: Vec<u8>,
}

pub fn compute_provision_fingerprint<I>(inputs: I) -> Result<String, String>
where
    I: IntoIterator<Item = FingerprintInput>,
{
    let mut inputs = inputs.into_iter().collect::<Vec<_>>();
    inputs.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));

    let mut hasher = Sha256::new();
    hasher.update(b"agbranch-base-fingerprint-v1\0");
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
    let provision_dir = root.join("lima").join("provision");
    let mut scripts = Vec::new();
    for entry in std::fs::read_dir(provision_dir)? {
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
        scripts.push(PathBuf::from("lima").join("provision").join(name));
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
}
