use crate::session::paths::{
    claude_global_state_path, claude_settings_path, codex_config_path, gemini_settings_path,
    gemini_trusted_folders_path,
};
use crate::types::{GuestPath, ProviderKind};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBootstrapFile {
    pub guest_path: GuestPath,
    pub contents: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiAuthMode {
    GeminiApiKey,
}

pub fn bootstrap_files(
    kind: ProviderKind,
    host_home_dir: &Path,
    workspace: &GuestPath,
) -> Vec<ProviderBootstrapFile> {
    bootstrap_files_with_gemini_auth(kind, host_home_dir, workspace, None)
}

pub fn bootstrap_files_with_gemini_auth(
    kind: ProviderKind,
    host_home_dir: &Path,
    workspace: &GuestPath,
    gemini_auth_mode: Option<GeminiAuthMode>,
) -> Vec<ProviderBootstrapFile> {
    match kind {
        ProviderKind::Codex => vec![ProviderBootstrapFile {
            guest_path: codex_config_path(host_home_dir),
            contents: render_codex_config_toml(workspace, host_home_dir),
        }],
        ProviderKind::Claude => vec![
            ProviderBootstrapFile {
                guest_path: claude_settings_path(host_home_dir),
                contents: render_claude_settings_json(),
            },
            ProviderBootstrapFile {
                guest_path: claude_global_state_path(host_home_dir),
                contents: render_claude_global_state_json(workspace, host_home_dir),
            },
        ],
        ProviderKind::Gemini => vec![
            ProviderBootstrapFile {
                guest_path: gemini_settings_path(host_home_dir),
                contents: render_gemini_settings_json(host_home_dir, gemini_auth_mode),
            },
            ProviderBootstrapFile {
                guest_path: gemini_trusted_folders_path(host_home_dir),
                contents: render_gemini_trusted_folders_json(workspace),
            },
        ],
    }
}

pub fn render_claude_settings_json() -> String {
    serde_json::to_string_pretty(&json!({
        "$schema": "https://json.schemastore.org/claude-code-settings.json",
        "skipDangerousModePermissionPrompt": true,
    }))
    .expect("Claude settings bootstrap should serialize")
}

pub fn render_claude_global_state_json(workspace: &GuestPath, host_home_dir: &Path) -> String {
    let theme = detect_host_claude_theme(host_home_dir).unwrap_or_else(|| "dark".to_owned());
    serde_json::to_string_pretty(&json!({
        "theme": theme,
        "firstStartTime": "2026-01-01T00:00:00.000Z",
        "hasCompletedOnboarding": true,
        "projects": {
            workspace.to_string(): {
                "allowedTools": [],
                "mcpContextUris": [],
                "mcpServers": {},
                "enabledMcpjsonServers": [],
                "disabledMcpjsonServers": [],
                "hasTrustDialogAccepted": true,
                "projectOnboardingSeenCount": 1,
                "hasClaudeMdExternalIncludesApproved": false,
                "hasClaudeMdExternalIncludesWarningShown": false,
            }
        }
    }))
    .expect("Claude state bootstrap should serialize")
}

fn detect_host_claude_theme(host_home_dir: &Path) -> Option<String> {
    let path = host_home_dir.join(".claude.json");
    let contents = std::fs::read_to_string(path).ok()?;
    let json = serde_json::from_str::<serde_json::Value>(&contents).ok()?;
    json.get("theme")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

pub fn render_codex_config_toml(workspace: &GuestPath, host_home_dir: &Path) -> String {
    let host_config = detect_host_codex_config(host_home_dir);
    let mut rendered = String::new();

    if let Some(model) = host_config.model.as_deref() {
        rendered.push_str("model = ");
        rendered.push_str(&toml_literal(model));
        rendered.push('\n');
    }
    if let Some(base_url) = host_config.openai_base_url.as_deref() {
        rendered.push_str("openai_base_url = ");
        rendered.push_str(&toml_literal(base_url));
        rendered.push('\n');
    }
    if let Some(reasoning) = host_config.model_reasoning_effort.as_deref() {
        rendered.push_str("model_reasoning_effort = ");
        rendered.push_str(&toml_literal(reasoning));
        rendered.push('\n');
    }
    if let Some(personality) = host_config.personality.as_deref() {
        rendered.push_str("personality = ");
        rendered.push_str(&toml_literal(personality));
        rendered.push('\n');
    }

    rendered.push('\n');
    rendered.push_str(&format!(
        "[projects.{}]\ntrust_level = \"trusted\"\n",
        toml_literal(&workspace.to_string())
    ));

    if !host_config.notice_model_migrations.is_empty() {
        rendered.push_str("\n[notice.model_migrations]\n");
        for (from, to) in &host_config.notice_model_migrations {
            rendered.push_str(&toml_literal(from));
            rendered.push_str(" = ");
            rendered.push_str(&toml_literal(to));
            rendered.push('\n');
        }
    }

    rendered
}

pub fn render_gemini_settings_json(
    host_home_dir: &Path,
    auth_mode: Option<GeminiAuthMode>,
) -> String {
    let host_settings = detect_host_gemini_settings(host_home_dir);
    let mut security = serde_json::Map::new();
    security.insert(
        "folderTrust".to_owned(),
        json!({
            "enabled": true,
        }),
    );
    if let Some(auth_mode) = auth_mode {
        security.insert(
            "auth".to_owned(),
            json!({
                "selectedType": match auth_mode {
                    GeminiAuthMode::GeminiApiKey => "gemini-api-key",
                }
            }),
        );
    }

    serde_json::to_string_pretty(&json!({
        "security": security,
        "ui": {
            "theme": host_settings.theme.unwrap_or_else(|| "Google Code".to_owned()),
        },
        "general": {
            "sessionRetention": host_settings.session_retention.unwrap_or_else(default_gemini_session_retention),
        }
    }))
    .expect("Gemini settings bootstrap should serialize")
}

pub fn render_gemini_trusted_folders_json(workspace: &GuestPath) -> String {
    serde_json::to_string_pretty(&json!({
        workspace.to_string(): "TRUST_FOLDER",
    }))
    .expect("Gemini trusted folders bootstrap should serialize")
}

#[derive(Default)]
struct HostCodexConfig {
    model: Option<String>,
    openai_base_url: Option<String>,
    model_reasoning_effort: Option<String>,
    personality: Option<String>,
    notice_model_migrations: Vec<(String, String)>,
}

#[derive(Default)]
struct HostGeminiSettings {
    theme: Option<String>,
    session_retention: Option<serde_json::Value>,
}

fn detect_host_codex_config(host_home_dir: &Path) -> HostCodexConfig {
    let path = host_home_dir.join(".codex").join("config.toml");
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return HostCodexConfig::default(),
    };
    let value = match toml::from_str::<toml::Table>(&contents) {
        Ok(value) => value,
        Err(_) => return HostCodexConfig::default(),
    };

    let model = value
        .get("model")
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned);
    let openai_base_url = value
        .get("openai_base_url")
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned);
    let model_reasoning_effort = value
        .get("model_reasoning_effort")
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned);
    let personality = value
        .get("personality")
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned);
    let notice_model_migrations = value
        .get("notice")
        .and_then(toml::Value::as_table)
        .and_then(|notice| notice.get("model_migrations"))
        .and_then(toml::Value::as_table)
        .map(|table| {
            table
                .iter()
                .filter_map(|(from, to)| to.as_str().map(|to| (from.clone(), to.to_owned())))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    HostCodexConfig {
        model,
        openai_base_url,
        model_reasoning_effort,
        personality,
        notice_model_migrations,
    }
}

fn detect_host_gemini_settings(host_home_dir: &Path) -> HostGeminiSettings {
    let path = host_home_dir.join(".gemini").join("settings.json");
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return HostGeminiSettings::default(),
    };
    let json = match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(json) => json,
        Err(_) => return HostGeminiSettings::default(),
    };

    let theme = json
        .get("ui")
        .and_then(|value| value.get("theme"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let session_retention = json
        .get("general")
        .and_then(|value| value.get("sessionRetention"))
        .cloned();

    HostGeminiSettings {
        theme,
        session_retention,
    }
}

fn default_gemini_session_retention() -> serde_json::Value {
    json!({
        "enabled": true,
        "maxAge": "30d",
        "warningAcknowledged": true,
    })
}

fn toml_literal(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tempfile::tempdir;

    #[test]
    fn claude_settings_bootstrap_skips_bypass_confirmation_prompt() {
        let rendered = render_claude_settings_json();
        let json: Value = serde_json::from_str(&rendered).expect("valid json");

        assert_eq!(json["skipDangerousModePermissionPrompt"], Value::Bool(true));
    }

    #[test]
    fn claude_state_bootstrap_marks_workspace_trusted_and_onboarding_complete() {
        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");
        let home = tempdir().expect("tempdir");
        let rendered = render_claude_global_state_json(&workspace, home.path());
        let json: Value = serde_json::from_str(&rendered).expect("valid json");

        assert_eq!(json["hasCompletedOnboarding"], Value::Bool(true));
        assert_eq!(json["theme"], Value::String("dark".to_owned()));
        assert_eq!(
            json["projects"]["/home/tester.guest/workspaces/demo/repo"]["hasTrustDialogAccepted"],
            Value::Bool(true)
        );
    }

    #[test]
    fn claude_bootstrap_prefers_host_theme_when_available() {
        let home = tempdir().expect("tempdir");
        std::fs::write(home.path().join(".claude.json"), r#"{"theme":"light"}"#)
            .expect("theme file");
        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");

        let rendered = render_claude_global_state_json(&workspace, home.path());
        let json: Value = serde_json::from_str(&rendered).expect("valid json");

        assert_eq!(json["theme"], Value::String("light".to_owned()));
    }

    #[test]
    fn claude_bootstrap_files_target_guest_claude_state_locations() {
        let root = tempdir().expect("tempdir");
        let home = root.path().join("tester");
        std::fs::create_dir_all(&home).expect("home dir");
        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");
        let files = bootstrap_files(ProviderKind::Claude, &home, &workspace);

        let guest_paths = files
            .iter()
            .map(|entry| entry.guest_path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            guest_paths,
            vec![
                "/home/tester.guest/.claude/settings.json".to_owned(),
                "/home/tester.guest/.claude.json".to_owned()
            ]
        );
    }

    #[test]
    fn codex_bootstrap_carries_model_base_url_and_guest_workspace_trust() {
        let root = tempdir().expect("tempdir");
        let home = root.path().join("tester");
        let codex_dir = home.join(".codex");
        std::fs::create_dir_all(&codex_dir).expect("codex dir");
        std::fs::write(
            codex_dir.join("config.toml"),
            r#"
model = "gpt-5.4"
openai_base_url = "https://gateway.example/v1"

[notice.model_migrations]
"gpt-5.3-codex" = "gpt-5.4"
"#,
        )
        .expect("config file");

        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");
        let rendered = render_codex_config_toml(&workspace, &home);

        assert!(rendered.contains("model = "));
        assert!(rendered.contains("gpt-5.4"));
        assert!(rendered.contains("openai_base_url = "));
        assert!(rendered.contains("https://gateway.example/v1"));
        assert!(rendered.contains("[projects.\"/home/tester.guest/workspaces/demo/repo\"]"));
        assert!(rendered.contains("trust_level = \"trusted\""));
        assert!(rendered.contains("[notice.model_migrations]"));
        assert!(rendered.contains("\"gpt-5.3-codex\" = \"gpt-5.4\""));
    }

    #[test]
    fn codex_bootstrap_files_target_guest_codex_config_location() {
        let root = tempdir().expect("tempdir");
        let home = root.path().join("tester");
        let codex_dir = home.join(".codex");
        std::fs::create_dir_all(&codex_dir).expect("codex dir");
        std::fs::write(codex_dir.join("config.toml"), "model = \"gpt-5.4\"\n").expect("config");

        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");
        let files = bootstrap_files(ProviderKind::Codex, &home, &workspace);

        let guest_paths = files
            .iter()
            .map(|entry| entry.guest_path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            guest_paths,
            vec!["/home/tester.guest/.codex/config.toml".to_owned()]
        );
    }

    #[test]
    fn gemini_settings_bootstrap_enables_folder_trust_and_api_key_auth() {
        let root = tempdir().expect("tempdir");
        let home = root.path().join("tester");
        let gemini_dir = home.join(".gemini");
        std::fs::create_dir_all(&gemini_dir).expect("gemini dir");
        std::fs::write(
            gemini_dir.join("settings.json"),
            r#"{
  "ui": { "theme": "Google Code" },
  "general": {
    "sessionRetention": {
      "enabled": true,
      "maxAge": "30d",
      "warningAcknowledged": true
    }
  }
}"#,
        )
        .expect("settings");

        let rendered = render_gemini_settings_json(&home, Some(GeminiAuthMode::GeminiApiKey));
        let json: Value = serde_json::from_str(&rendered).expect("valid json");

        assert_eq!(
            json["security"]["folderTrust"]["enabled"],
            Value::Bool(true)
        );
        assert_eq!(
            json["security"]["auth"]["selectedType"],
            Value::String("gemini-api-key".to_owned())
        );
        assert_eq!(json["ui"]["theme"], Value::String("Google Code".to_owned()));
        assert_eq!(
            json["general"]["sessionRetention"]["maxAge"],
            Value::String("30d".to_owned())
        );
    }

    #[test]
    fn gemini_trusted_folders_bootstrap_marks_exact_workspace_trusted() {
        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");
        let rendered = render_gemini_trusted_folders_json(&workspace);
        let json: Value = serde_json::from_str(&rendered).expect("valid json");

        assert_eq!(
            json["/home/tester.guest/workspaces/demo/repo"],
            Value::String("TRUST_FOLDER".to_owned())
        );
    }

    #[test]
    fn gemini_bootstrap_files_target_guest_settings_and_trusted_folder_locations() {
        let root = tempdir().expect("tempdir");
        let home = root.path().join("tester");
        std::fs::create_dir_all(home.join(".gemini")).expect("gemini dir");
        let workspace = GuestPath::new("/home/tester.guest/workspaces/demo/repo");

        let files = bootstrap_files_with_gemini_auth(
            ProviderKind::Gemini,
            &home,
            &workspace,
            Some(GeminiAuthMode::GeminiApiKey),
        );

        let guest_paths = files
            .iter()
            .map(|entry| entry.guest_path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            guest_paths,
            vec![
                "/home/tester.guest/.gemini/settings.json".to_owned(),
                "/home/tester.guest/.gemini/trustedFolders.json".to_owned(),
            ]
        );
    }
}
