//! Enforces the CARGO_MANIFEST_DIR / OUT_DIR discipline for the embedded
//! Lima asset bundle.
//!
//! The gate scans production source (`src/**/*.rs`) only. Test helpers
//! under `tests/` are allowed to reference `env!("CARGO_MANIFEST_DIR")`
//! because they are compiled and executed from the source tree — the
//! "binary stands alone" guarantee applies to the production binary, not
//! to in-tree test harnesses.
//!
//! - `env!("CARGO_MANIFEST_DIR")` is allowed only in `build.rs`. No `src/`
//!   file may reference it; the binary must not depend on the Cargo
//!   source-tree path at runtime.
//! - `env!("OUT_DIR")` is allowed at exactly one location: the `include!`
//!   macro in `src/lima/assets.rs`, the single bridge to the generated
//!   manifest.
//!
//! Failure of either test means a regression has re-introduced a
//! forbidden `env!(...)` reference into the production source tree.

use std::path::{Path, PathBuf};

// REPO_ROOT uses CARGO_MANIFEST_DIR on purpose — this is a test helper, not
// runtime code, and the grep gate scans only src/**/*.rs (and build.rs).
// The markers below are assembled from fragments so this file can contain
// the literal text without being its own offender if the gate ever widens.
const REPO_ROOT: &str = env!("CARGO_MANIFEST_DIR");
const MANIFEST_DIR_MARKER: &str = concat!("env!(", "\"CARGO_MANIFEST_DIR\"", ")");
const OUT_DIR_MARKER: &str = concat!("env!(", "\"OUT_DIR\"", ")");
const OUT_DIR_ALLOW_PATH: &str = "src/lima/assets.rs";

#[test]
fn cargo_manifest_dir_is_only_used_in_build_rs() {
    let repo = Path::new(REPO_ROOT);
    let mut offenders = Vec::new();
    for rs in collect_source_tree_rs_files(repo) {
        let rel = rs.strip_prefix(repo).unwrap_or(&rs);
        if rel == Path::new("build.rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&rs).expect("read");
        if contents.contains(MANIFEST_DIR_MARKER) {
            offenders.push(rel.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "CARGO_MANIFEST_DIR must appear only in build.rs; offenders: {offenders:?}",
    );
}

#[test]
fn out_dir_is_only_used_in_lima_assets_rs() {
    let repo = Path::new(REPO_ROOT);
    let mut offenders = Vec::new();
    for rs in collect_source_tree_rs_files(repo) {
        let rel = rs.strip_prefix(repo).unwrap_or(&rs);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let contents = std::fs::read_to_string(&rs).expect("read");
        if contents.contains(OUT_DIR_MARKER) && rel_str != OUT_DIR_ALLOW_PATH {
            offenders.push(rel.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "OUT_DIR must appear only in {OUT_DIR_ALLOW_PATH}; offenders: {offenders:?}",
    );
}

fn collect_source_tree_rs_files(repo_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    // Production source only. Tests live outside the gate on purpose.
    walk(&repo_root.join("src"), &mut out);
    // build.rs sits at the repo root and is allow-listed per-check, but we
    // still want it in the scan so the CARGO_MANIFEST_DIR gate can confirm
    // it is the sole offender.
    let build_rs = repo_root.join("build.rs");
    if build_rs.is_file() {
        out.push(build_rs);
    }
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            walk(&path, out);
        } else if file_type.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}
