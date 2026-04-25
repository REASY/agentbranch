use agbranch::lima::base_info::BaseMetadata;
use agbranch::lima::fingerprint::CURRENT_PROVISION_FINGERPRINT;
use assert_cmd::Command;
use predicates::prelude::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

#[test]
fn base_show_json_reports_ready_base() {
    let temp = tempdir().expect("tempdir");
    let base_dir = temp.path().join("agbranch-test-base");
    fs::create_dir_all(&base_dir).expect("base dir");
    fs::write(
        base_dir.join("agbranch-base.json"),
        serde_json::to_string(&BaseMetadata {
            schema_version: 1,
            prepared_at: "2026-04-25T04:24:06Z".to_owned(),
            provision_fingerprint: CURRENT_PROVISION_FINGERPRINT.to_owned(),
            agent_cli_versions: BTreeMap::from([("codex".to_owned(), "0.9.0".to_owned())]),
        })
        .expect("metadata"),
    )
    .expect("write metadata");
    let list_json = temp.path().join("limactl-list.json");
    fs::write(
        &list_json,
        format!(
            r#"[{{
  "name": "agbranch-test-base",
  "dir": "{}",
  "sshConfigFile": "{}/ssh.config",
  "vmType": "vz",
  "status": "Stopped",
  "protected": true,
  "disk": 107374182400
}}]"#,
            base_dir.display(),
            base_dir.display()
        ),
    )
    .expect("list json");
    let limactl = install_limactl(temp.path()).expect("limactl");

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("PATH", prepend_path(limactl.parent().expect("parent")))
        .env("LIMACTL_LIST_JSON", &list_json)
        .env("AGBRANCH_PREPARED_BASE_NAME", "agbranch-test-base")
        .args(["base", "show", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\":\"agbranch-test-base\""))
        .stdout(predicate::str::contains("\"name_source\":\"env_override\""))
        .stdout(predicate::str::contains("\"status\":\"stopped\""))
        .stdout(predicate::str::contains("\"protected\":true"))
        .stdout(predicate::str::contains("\"prepared\":true"))
        .stdout(predicate::str::contains("\"provision_stale\":false"));
}

#[test]
fn base_show_require_ready_fails_when_base_is_missing() {
    let temp = tempdir().expect("tempdir");
    let list_json = temp.path().join("limactl-list.json");
    fs::write(&list_json, "[]").expect("list json");
    let limactl = install_limactl(temp.path()).expect("limactl");

    Command::cargo_bin("agbranch")
        .expect("binary")
        .env("PATH", prepend_path(limactl.parent().expect("parent")))
        .env("LIMACTL_LIST_JSON", &list_json)
        .env("AGBRANCH_PREPARED_BASE_NAME", "agbranch-test-base")
        .args(["base", "show", "--require-ready"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains(
            "base is missing: run agbranch base prepare",
        ));
}

fn prepend_path(dir: &Path) -> String {
    let existing = std::env::var("PATH").unwrap_or_default();
    if existing.is_empty() {
        dir.display().to_string()
    } else {
        format!("{}:{existing}", dir.display())
    }
}

fn install_limactl(temp_root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let bin_dir = temp_root.join("bin");
    fs::create_dir_all(&bin_dir)?;
    let limactl = bin_dir.join("limactl");
    fs::write(
        &limactl,
        r#"#!/bin/sh
if [ "$1" = "list" ] && [ "$2" = "--json" ]; then
  cat "$LIMACTL_LIST_JSON"
  exit 0
fi
echo "unexpected limactl args: $*" >&2
exit 1
"#,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&limactl)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&limactl, perms)?;
    }
    Ok(limactl)
}
