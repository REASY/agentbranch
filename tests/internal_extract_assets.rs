//! Covers the hidden `agbranch internal extract-assets` CLI surface.

use assert_cmd::Command;
use predicates::str;
use std::fs;
use tempfile::tempdir;

fn fresh_state_root() -> tempfile::TempDir {
    tempdir().expect("state root")
}

#[test]
fn extract_assets_json_reports_extraction_then_short_circuits() {
    let state = fresh_state_root();

    // First call extracts into the fresh state root.
    let first = Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", state.path())
        .env_remove("AGBRANCH_LIMA_ASSETS_DIR")
        .args(["internal", "extract-assets", "--json"])
        .output()
        .expect("run first");
    assert!(
        first.status.success(),
        "first extract-assets failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_stdout = String::from_utf8(first.stdout).expect("utf8");
    let first_json: serde_json::Value = serde_json::from_str(first_stdout.trim()).expect("json");
    assert_eq!(first_json["origin"], "state_root_cache");
    assert_eq!(first_json["extracted_this_call"], true);
    let path = first_json["path"].as_str().expect("path field").to_owned();
    assert!(fs::metadata(&path).is_ok(), "lima dir must exist at {path}");

    // Second call must short-circuit.
    let second = Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", state.path())
        .env_remove("AGBRANCH_LIMA_ASSETS_DIR")
        .args(["internal", "extract-assets", "--json"])
        .output()
        .expect("run second");
    assert!(second.status.success());
    let second_json: serde_json::Value =
        serde_json::from_str(std::str::from_utf8(&second.stdout).unwrap().trim()).expect("json");
    assert_eq!(second_json["extracted_this_call"], false);
    assert_eq!(second_json["path"], path);
}

#[test]
fn extract_assets_human_output_distinguishes_cold_from_warm() {
    let state = fresh_state_root();

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", state.path())
        .env_remove("AGBRANCH_LIMA_ASSETS_DIR")
        .args(["internal", "extract-assets"])
        .assert()
        .success()
        .stdout(str::starts_with("extracted lima assets to "));

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("AGBRANCH_STATE_ROOT", state.path())
        .env_remove("AGBRANCH_LIMA_ASSETS_DIR")
        .args(["internal", "extract-assets"])
        .assert()
        .success()
        .stdout(str::starts_with("lima assets already present at "));
}

#[test]
fn internal_is_hidden_from_top_level_help_but_visible_via_own_help() {
    let top = Command::cargo_bin("agbranch")
        .expect("binary")
        .arg("--help")
        .assert()
        .success();
    let top_stdout = String::from_utf8(top.get_output().stdout.clone()).expect("utf8");
    assert!(
        !top_stdout.contains("internal"),
        "top-level help must not mention `internal`: {top_stdout}"
    );

    // `internal --help` should still list the subcommands so operators
    // troubleshooting a broken cache can discover the surface.
    Command::cargo_bin("agbranch")
        .expect("binary")
        .args(["internal", "--help"])
        .assert()
        .success()
        .stdout(str::contains("extract-assets"));
}
