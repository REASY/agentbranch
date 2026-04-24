use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn close_accepts_positional_session() {
    let state_dir = tempdir().expect("tempdir");

    Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["close", "demo", "--discard"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session `demo` was not found"));
}

#[test]
fn attach_accepts_positional_session() {
    let state_dir = tempdir().expect("tempdir");

    Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["attach", "demo", "--agent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session `demo` was not found"));
}

#[test]
fn sync_back_accepts_positional_session() {
    let state_dir = tempdir().expect("tempdir");

    Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["sync-back", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session `demo` was not found"));
}

#[test]
fn close_still_accepts_long_session_flag() {
    let state_dir = tempdir().expect("tempdir");

    Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["close", "--session", "demo", "--discard"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session `demo` was not found"));
}

#[test]
fn test_subcommand_is_not_available_with_long_session_flag() {
    let state_dir = tempdir().expect("tempdir");

    Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["test", "rust", "--session", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand 'test'"));
}

#[test]
fn test_subcommand_is_not_available_with_positional_session() {
    let state_dir = tempdir().expect("tempdir");

    Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["test", "rust", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand 'test'"));
}

#[test]
fn compose_subcommand_is_not_available_with_long_session_flag() {
    let state_dir = tempdir().expect("tempdir");

    Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["compose", "up", "--session", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unrecognized subcommand 'compose'",
        ));
}

#[test]
fn compose_subcommand_is_not_available_with_positional_session() {
    let state_dir = tempdir().expect("tempdir");

    Command::cargo_bin("agbranch")
        .expect("binary should build")
        .env("AGBRANCH_STATE_ROOT", state_dir.path())
        .env("HOME", "/Users/alice")
        .args(["compose", "up", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unrecognized subcommand 'compose'",
        ));
}
