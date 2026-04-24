use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn close_without_outcome_exits_with_validation_code() {
    Command::cargo_bin("agbranch")
        .expect("binary")
        .args(["close", "--session", "agbranch-smoke-missing-outcome"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "close requires exactly one of --sync or --discard",
        ));
}

#[test]
fn close_without_outcome_still_validates_before_host_detection() {
    Command::cargo_bin("agbranch")
        .expect("binary")
        .env_remove("HOME")
        .args(["close", "--session", "agbranch-smoke-missing-outcome"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "close requires exactly one of --sync or --discard",
        ))
        .stderr(predicate::str::contains("unsupported host platform").not());
}

#[test]
fn close_invalid_session_name_still_validates_before_host_detection() {
    let too_long = "x".repeat(49);
    Command::cargo_bin("agbranch")
        .expect("binary")
        .env_remove("HOME")
        .args([
            "close",
            "--session",
            too_long.as_str(),
            "--discard",
            "--yes",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "session name exceeds the 48-character limit",
        ))
        .stderr(predicate::str::contains("unsupported host platform").not());
}
