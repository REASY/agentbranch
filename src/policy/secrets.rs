use crate::error::{AppError, ValidationError};
use crate::types::{GuestPath, SessionName};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub fn merge_env_inputs(
    inline: &[String],
    env_files: &[PathBuf],
) -> Result<BTreeMap<String, String>, AppError> {
    let mut merged = BTreeMap::new();

    for path in env_files {
        let contents = fs::read_to_string(path)?;
        for line in contents.lines().filter(|line| !line.trim().is_empty()) {
            let (key, value) = parse_env_line(line)?;
            merged.insert(key.to_owned(), value.to_owned());
        }
    }

    for entry in inline {
        let (key, value) = parse_env_line(entry)?;
        merged.insert(key.to_owned(), value.to_owned());
    }

    Ok(merged)
}

pub fn render_guest_secret_file(env: &BTreeMap<String, String>) -> Result<String, AppError> {
    let mut rendered = String::from("#!/bin/sh\nset -eu\n");
    for (key, value) in env {
        let escaped = value.replace('\'', "'\"'\"'");
        rendered.push_str(&format!("export {key}='{escaped}'\n"));
    }
    Ok(rendered)
}

pub fn guest_secret_path(repo_guest_path: &GuestPath, session: &SessionName) -> GuestPath {
    let guest_home = repo_guest_path
        .as_path()
        .ancestors()
        .nth(3)
        .expect("repo guest path should live under <guest-home>/workspaces/<session>/repo");
    GuestPath::new(
        guest_home
            .join(".agbranch")
            .join("secrets")
            .join(session.as_str())
            .join("command.env"),
    )
}

fn parse_env_line(entry: &str) -> Result<(&str, &str), AppError> {
    entry
        .split_once('=')
        .ok_or_else(|| AppError::Validation(ValidationError::InvalidEnvEntry(entry.to_owned())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn env_file_is_host_resolved_and_overrides_are_applied() {
        let file = NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), "A=1\nB=2\n").expect("write env file");

        let merged = merge_env_inputs(
            &[format!("B={}", 3), format!("C={}", 4)],
            &[file.path().to_path_buf()],
        )
        .expect("merge env");

        assert_eq!(merged.get("A").map(String::as_str), Some("1"));
        assert_eq!(merged.get("B").map(String::as_str), Some("3"));
        assert_eq!(merged.get("C").map(String::as_str), Some("4"));

        let rendered = render_guest_secret_file(&merged).expect("render env");
        assert!(rendered.contains("export A='1'"));
        assert!(rendered.contains("export B='3'"));
        assert!(rendered.contains("export C='4'"));
    }

    #[test]
    fn rendered_guest_secret_file_shell_escapes_sensitive_values() {
        let merged = std::collections::BTreeMap::from([
            ("API_KEY".to_owned(), "a value with spaces".to_owned()),
            ("QUOTE".to_owned(), "it'\"s-fine".to_owned()),
        ]);

        let rendered = render_guest_secret_file(&merged).expect("render env");
        assert!(rendered.contains("export API_KEY='a value with spaces'"));
        assert!(rendered.contains("export QUOTE='it'\"'\"'\"s-fine'"));
    }
}
