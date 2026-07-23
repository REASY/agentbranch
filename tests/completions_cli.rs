use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn generates_bash_completion_with_commands_and_options() {
    Command::cargo_bin("agbranch")
        .expect("binary")
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_agbranch()"))
        .stdout(predicate::str::contains("sync-back"))
        .stdout(predicate::str::contains("--publish"));
}

#[test]
fn generates_native_markers_for_other_shells() {
    for (shell, marker) in [
        ("zsh", "#compdef agbranch"),
        ("fish", "complete -c agbranch"),
        ("elvish", "edit:completion:arg-completer[agbranch]"),
        ("powershell", "Register-ArgumentCompleter"),
    ] {
        Command::cargo_bin("agbranch")
            .expect("binary")
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains(marker));
    }
}
