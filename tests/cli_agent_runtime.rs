use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_agent_runtime_commands() {
    Command::cargo_bin("agbranch")
        .expect("binary")
        .arg("--help")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("launch")
                .and(predicate::str::contains("export"))
                .and(predicate::str::contains("attach"))
                .and(predicate::str::contains("agent"))
                .and(predicate::str::contains("kill")),
        );
}

#[test]
fn open_help_mentions_base_ref() {
    Command::cargo_bin("agbranch")
        .expect("binary")
        .args(["open", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--base").and(predicate::str::contains("--tag").not()));
}
