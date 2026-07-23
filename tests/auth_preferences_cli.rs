use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::tempdir;

fn command(state_root: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("agbranch").expect("binary");
    command
        .env("AGBRANCH_STATE_ROOT", state_root)
        .env("HOME", state_root.join("home"));
    command
}

#[test]
fn auth_preferences_can_be_set_inspected_and_reset() {
    let state = tempdir().expect("state");

    command(state.path())
        .args(["auth", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            r#""provider":"codex","policy":null"#,
        ));

    command(state.path())
        .args(["auth", "set", "codex", "import", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""policy":"import""#));

    let output = command(state.path())
        .args(["auth", "list", "--json"])
        .output()
        .expect("list");
    assert!(output.status.success());
    let preferences: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(preferences[0]["provider"], "codex");
    assert_eq!(preferences[0]["policy"], "import");
    assert_eq!(preferences[1]["provider"], "claude");
    assert!(preferences[1]["policy"].is_null());

    command(state.path())
        .args(["auth", "reset", "codex", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""reset_count":1"#));
    command(state.path())
        .args(["auth", "reset", "codex"])
        .assert()
        .success()
        .stdout(predicate::str::contains("already unset"));
}

#[test]
fn auth_reset_all_clears_each_provider() {
    let state = tempdir().expect("state");
    for provider in ["codex", "claude", "gemini"] {
        command(state.path())
            .args(["auth", "set", provider, "none"])
            .assert()
            .success();
    }

    command(state.path())
        .args(["auth", "reset", "--all", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""reset_count":3"#));
    command(state.path())
        .args(["auth", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("codex      unset"))
        .stdout(predicate::str::contains("claude     unset"))
        .stdout(predicate::str::contains("gemini     unset"));
}
