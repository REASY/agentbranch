use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_the_expected_subcommands() {
    let mut cmd = Command::cargo_bin("agbranch").expect("binary should build");
    cmd.arg("--help").assert().success().stdout(
        predicate::str::contains("base")
            .and(predicate::str::contains("\n  prepare ").not())
            .and(predicate::str::contains("open"))
            .and(predicate::str::contains("auth"))
            .and(predicate::str::contains("ps"))
            .and(predicate::str::contains("show"))
            .and(predicate::str::contains("start"))
            .and(predicate::str::contains("stop"))
            .and(predicate::str::contains("shell"))
            .and(predicate::str::contains("ssh"))
            .and(predicate::str::contains("run"))
            .and(predicate::str::contains("sync-back"))
            .and(predicate::str::contains("close"))
            .and(predicate::str::contains("gc"))
            .and(predicate::str::contains("logs"))
            .and(predicate::str::contains("watch"))
            .and(predicate::str::contains("repair"))
            .and(predicate::str::contains("retry"))
            .and(predicate::str::contains("completions"))
            .and(predicate::str::contains("doctor")),
    );
}

#[test]
fn top_level_help_describes_workflows_and_commands() {
    Command::cargo_bin("agbranch")
        .expect("binary")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Create a disposable sandbox session",
        ))
        .stdout(predicate::str::contains(
            "Create a git-native repository session",
        ))
        .stdout(predicate::str::contains("Quick start:"))
        .stdout(predicate::str::contains("agbranch base prepare"));
}

#[test]
fn launch_and_open_help_include_examples_and_resource_guidance() {
    Command::cargo_bin("agbranch")
        .expect("binary")
        .args(["launch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Examples:"))
        .stdout(predicate::str::contains("8GiB"))
        .stdout(predicate::str::contains("agbranch retry SESSION"));

    Command::cargo_bin("agbranch")
        .expect("binary")
        .args(["open", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("origin/main"))
        .stdout(predicate::str::contains(
            "checked-out host branch is never rewritten",
        ));
}

#[test]
fn ps_help_lists_all_flag() {
    let mut cmd = Command::cargo_bin("agbranch").expect("binary should build");
    cmd.args(["ps", "--help"]).assert().success().stdout(
        predicate::str::contains("-a, --all")
            .and(predicate::str::contains("ps"))
            .and(predicate::str::contains("--json"))
            .and(predicate::str::contains("--tag").not()),
    );
}
