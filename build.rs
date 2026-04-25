use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn main() {
    let root = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .expect("CARGO_MANIFEST_DIR should be set by Cargo");
    let inputs = fingerprint_inputs(&root).expect("provision fingerprint inputs");
    let fingerprint = compute_fingerprint(inputs).expect("provision fingerprint");
    println!("cargo:rustc-env=AGBRANCH_PROVISION_FINGERPRINT={fingerprint}");
}

#[derive(Debug, Clone)]
struct FingerprintInput {
    path: String,
    bytes: Vec<u8>,
}

fn fingerprint_inputs(root: &Path) -> Result<Vec<FingerprintInput>, String> {
    let mut paths = vec![
        PathBuf::from("lima/safe-sync-macos.yaml"),
        PathBuf::from("lima/safe-sync-linux.yaml"),
    ];

    println!("cargo:rerun-if-changed=lima/safe-sync-macos.yaml");
    println!("cargo:rerun-if-changed=lima/safe-sync-linux.yaml");
    println!("cargo:rerun-if-changed=lima/provision");

    for path in discover_non_recursive_provision_scripts(root)? {
        println!("cargo:rerun-if-changed={}", to_posix(&path));
        paths.push(path);
    }
    paths.sort_by(|left, right| to_posix(left).as_bytes().cmp(to_posix(right).as_bytes()));

    paths
        .into_iter()
        .map(|path| {
            let full_path = root.join(&path);
            let bytes = std::fs::read(&full_path)
                .map_err(|err| format!("failed to read `{}`: {err}", full_path.display()))?;
            Ok(FingerprintInput {
                path: to_posix(&path),
                bytes,
            })
        })
        .collect()
}

fn discover_non_recursive_provision_scripts(root: &Path) -> Result<Vec<PathBuf>, String> {
    let provision_dir = root.join("lima").join("provision");
    let entries = std::fs::read_dir(&provision_dir)
        .map_err(|err| format!("failed to read `{}`: {err}", provision_dir.display()))?;
    let mut scripts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let file_type = entry.file_type().map_err(|err| err.to_string())?;
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("sh") {
            continue;
        }
        scripts.push(
            PathBuf::from("lima")
                .join("provision")
                .join(entry.file_name()),
        );
    }
    scripts.sort();
    Ok(scripts)
}

fn compute_fingerprint<I>(inputs: I) -> Result<String, String>
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

fn to_posix(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
