use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_the_expected_subcommands() {
    let mut cmd = Command::cargo_bin("agbranch").expect("binary should build");
    cmd.arg("--help").assert().success().stdout(
        predicate::str::contains("base")
            .and(predicate::str::contains("prepare").not())
            .and(predicate::str::contains("open"))
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
            .and(predicate::str::contains("doctor")),
    );
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
