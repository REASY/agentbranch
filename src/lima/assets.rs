//! Embedded Lima asset manifest.
//!
//! The contents of `lima/` are embedded at build time by `build.rs`, which
//! writes a generated Rust source file to `$OUT_DIR/lima_assets.rs`. This
//! module declares the `EmbeddedAsset` struct and `include!`s that generated
//! file. It is the only source-tree bridge to `$OUT_DIR`; the generated
//! manifest is where `include_bytes!` ultimately reaches into the Cargo
//! manifest directory, without that reference leaking into the checked-in
//! source tree.

#[derive(Debug, Clone, Copy)]
pub struct EmbeddedAsset {
    pub relative_path: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/lima_assets.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const EXPECTED_RELATIVE_PATHS: &[&str] = &[
        "guest/scrub-artifacts.sh",
        "guest/shellenv.sh",
        "guest/workspace-init.sh",
        "provision/00-system.sh",
        "provision/05-network-guard.sh",
        "provision/10-agent-clis.sh",
        "provision/20-docker-compose.sh",
        "safe-sync-linux.yaml",
        "safe-sync-macos.yaml",
    ];

    #[test]
    fn embedded_manifest_contains_every_in_scope_asset() {
        let got: BTreeSet<&str> = EMBEDDED_LIMA_ASSETS
            .iter()
            .map(|asset| asset.relative_path)
            .collect();
        let want: BTreeSet<&str> = EXPECTED_RELATIVE_PATHS.iter().copied().collect();
        assert_eq!(got, want, "embedded manifest must match the in-scope set");
    }

    #[test]
    fn embedded_manifest_count_matches_in_scope() {
        assert_eq!(EMBEDDED_LIMA_ASSETS.len(), EXPECTED_RELATIVE_PATHS.len());
    }

    #[test]
    fn every_embedded_asset_has_non_empty_bytes() {
        for asset in EMBEDDED_LIMA_ASSETS {
            assert!(
                !asset.bytes.is_empty(),
                "`{}` must not be empty",
                asset.relative_path
            );
        }
    }

    #[test]
    fn embedded_manifest_is_sorted_bytewise_by_relative_path() {
        for window in EMBEDDED_LIMA_ASSETS.windows(2) {
            assert!(
                window[0].relative_path.as_bytes() < window[1].relative_path.as_bytes(),
                "`{}` must precede `{}`",
                window[0].relative_path,
                window[1].relative_path
            );
        }
    }
}
