//! End-to-end proof that the `agbranch` release binary stands alone once
//! the source tree is gone.
//!
//! The test copies the binary to a temp directory and runs a small
//! sequence of subcommands against a fresh state root. It is expensive,
//! so it's gated behind the `AGBRANCH_RUN_STANDALONE_E2E=1` env var. CI
//! and nightly smoke can opt in; routine `cargo test` skips it to stay
//! fast.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn standalone_enabled() -> bool {
    std::env::var("AGBRANCH_RUN_STANDALONE_E2E")
        .map(|value| !value.is_empty() && value != "0")
        .unwrap_or(false)
}

fn release_binary_path() -> Option<PathBuf> {
    // Prefer the release binary if the caller has already built it.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let release = PathBuf::from(manifest_dir)
        .join("target")
        .join("release")
        .join("agbranch");
    release.is_file().then_some(release)
}

fn inode_of(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path)
        .unwrap_or_else(|err| panic!("stat {}: {err}", path.display()))
        .ino()
}

fn sha256_of(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    format!("{:x}", Sha256::digest(&bytes))
}

fn run_binary(binary: &Path, state_root: &Path, args: &[&str]) -> (String, String, i32) {
    let output = Command::new(binary)
        .env("AGBRANCH_STATE_ROOT", state_root)
        // Guarantee the test runs in the default (non-override) path so
        // the resolver goes through the cache + extraction branch.
        .env_remove("AGBRANCH_LIMA_ASSETS_DIR")
        .args(args)
        .output()
        .expect("run binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or_default(),
    )
}

#[test]
fn standalone_binary_extracts_and_reuses_cache() {
    if !standalone_enabled() {
        eprintln!("skipping standalone E2E (set AGBRANCH_RUN_STANDALONE_E2E=1 to run)");
        return;
    }
    let Some(release) = release_binary_path() else {
        eprintln!("skipping standalone E2E: target/release/agbranch not built");
        return;
    };

    // Copy the binary to a fresh temp directory and confirm it runs from
    // there without the source tree present (the binary is truly
    // position-independent by the spec's definition).
    let bin_host = tempdir().expect("bin host");
    let dest = bin_host.path().join("agbranch");
    std::fs::copy(&release, &dest).expect("copy binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms).expect("chmod");
    }

    let state = tempdir().expect("state root");
    let state_root = state.path().to_path_buf();

    // 1) Force extraction via the hidden `internal extract-assets` helper.
    let (stdout, stderr, status) = run_binary(
        &dest,
        &state_root,
        &["internal", "extract-assets", "--json"],
    );
    assert_eq!(status, 0, "extract-assets exit: stderr={stderr}");
    let first: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|err| panic!("parse {stdout}: {err}"));
    assert_eq!(first["origin"], "state_root_cache");
    assert_eq!(first["extracted_this_call"], true);
    let lima_path = PathBuf::from(
        first["path"]
            .as_str()
            .expect("path in extract-assets output"),
    );
    assert!(lima_path.join("safe-sync-macos.yaml").is_file());
    assert!(lima_path.join("provision/00-system.sh").is_file());
    assert!(lima_path.join("guest/shellenv.sh").is_file());

    let canonical = lima_path.parent().expect("canonical").to_path_buf();
    let marker = canonical.join("fingerprint.ok");
    let marker_inode_before = inode_of(&marker);
    let marker_sha_before = sha256_of(&marker);
    let canonical_inode_before = inode_of(&canonical);

    // 2) `base show --json` must report Cache and must not re-extract. The
    //    inspector's contract is read-only; Cache implies the acceptance
    //    checks passed, and the fingerprint.ok inode/sha must be
    //    byte-for-byte unchanged (a re-extraction would rename-into-place
    //    and give the marker a new inode).
    let (stdout, stderr, status) = run_binary(&dest, &state_root, &["base", "show", "--json"]);
    assert_eq!(status, 0, "base show exit: stderr={stderr}");
    let show: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|err| panic!("parse {stdout}: {err}"));
    assert_eq!(show["lima_assets"]["source"], "cache");
    assert_eq!(
        show["lima_assets"]["cache_fingerprint"],
        show["lima_assets"]["bundle_fingerprint"]
    );
    assert_eq!(inode_of(&marker), marker_inode_before);
    assert_eq!(sha256_of(&marker), marker_sha_before);
    assert_eq!(inode_of(&canonical), canonical_inode_before);

    // 3) A second extract-assets call must short-circuit.
    let (stdout, stderr, status) = run_binary(
        &dest,
        &state_root,
        &["internal", "extract-assets", "--json"],
    );
    assert_eq!(status, 0, "second extract-assets exit: stderr={stderr}");
    let second: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|err| panic!("parse {stdout}: {err}"));
    assert_eq!(second["extracted_this_call"], false);
}
