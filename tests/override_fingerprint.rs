//! Integration test proving the runtime override fingerprint matches the
//! baked-in constant when computed over the repo's own `lima/` tree.
//!
//! This is the spec's key invariant for override mode: if build.rs and
//! the runtime helper use the same input discovery, override mode against
//! an unmodified repo must produce a fingerprint byte-for-byte identical
//! to the baked-in `AGBRANCH_PROVISION_FINGERPRINT`.

use agbranch::lima::fingerprint::{
    CURRENT_PROVISION_FINGERPRINT, ProvisionFingerprintSource,
    compute_effective_provision_fingerprint,
};
use std::path::PathBuf;

#[test]
fn runtime_override_fingerprint_over_repo_lima_matches_baked_in() {
    let lima_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lima");
    let computed =
        compute_effective_provision_fingerprint(ProvisionFingerprintSource::OverrideTree {
            lima_root: &lima_root,
        })
        .expect("compute");
    assert_eq!(
        computed, CURRENT_PROVISION_FINGERPRINT,
        "runtime override fingerprint over repo's own lima/ must match baked-in"
    );
}

#[test]
fn runtime_override_fingerprint_changes_when_input_changes() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let lima_root = tempdir.path().join("lima");
    std::fs::create_dir_all(lima_root.join("provision")).expect("mkdir provision");
    std::fs::write(
        lima_root.join("safe-sync-macos.yaml"),
        "images:\n  - location: macos\n",
    )
    .expect("mac template");
    std::fs::write(
        lima_root.join("safe-sync-linux.yaml"),
        "images:\n  - location: linux\n",
    )
    .expect("linux template");
    std::fs::write(lima_root.join("provision/00-system.sh"), "# v1\n").expect("provision v1");

    let first = compute_effective_provision_fingerprint(ProvisionFingerprintSource::OverrideTree {
        lima_root: &lima_root,
    })
    .expect("v1");

    std::fs::write(lima_root.join("provision/00-system.sh"), "# v2\n").expect("provision v2");
    let second =
        compute_effective_provision_fingerprint(ProvisionFingerprintSource::OverrideTree {
            lima_root: &lima_root,
        })
        .expect("v2");

    assert_ne!(first, second, "content change must flip the fingerprint");
}

#[test]
fn baked_in_source_ignores_override_path_argument() {
    let baked = compute_effective_provision_fingerprint(ProvisionFingerprintSource::BakedIn)
        .expect("baked in");
    assert_eq!(baked, CURRENT_PROVISION_FINGERPRINT);
}
