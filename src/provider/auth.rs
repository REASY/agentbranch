use crate::error::AppError;
use crate::platform::detect::HostPlatform;
use crate::provider::import::plan_imported_files;
use crate::types::{GuestPath, HostPath, ProviderKind};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;

const CLAUDE_IMPORTABLE_ENV_KEYS: &[&str] = &[
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
    "ANTHROPIC_BEDROCK_BASE_URL",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedAuthSource {
    File {
        host_path: HostPath,
        guest_path: GuestPath,
    },
    EnvVar {
        name: String,
        value: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedAuth {
    pub sources: Vec<DetectedAuthSource>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportedAuthMaterial {
    File {
        host_path: HostPath,
        guest_path: GuestPath,
    },
    EnvVar {
        name: String,
    },
}

impl DetectedAuthSource {
    pub fn as_metadata(&self) -> ImportedAuthMaterial {
        match self {
            Self::File {
                host_path,
                guest_path,
            } => ImportedAuthMaterial::File {
                host_path: host_path.clone(),
                guest_path: guest_path.clone(),
            },
            Self::EnvVar { name, .. } => ImportedAuthMaterial::EnvVar { name: name.clone() },
        }
    }
}

pub trait AuthPrompter {
    fn confirm_import(
        &self,
        provider: ProviderKind,
        detection: &DetectedAuth,
    ) -> Result<bool, AppError>;
}

pub struct TerminalAuthPrompter;

impl AuthPrompter for TerminalAuthPrompter {
    fn confirm_import(
        &self,
        provider: ProviderKind,
        detection: &DetectedAuth,
    ) -> Result<bool, AppError> {
        let mut stderr = io::stderr().lock();
        writeln!(
            stderr,
            "detected importable {} auth for this guest session:",
            provider.as_str()
        )?;
        for source in &detection.sources {
            match source {
                DetectedAuthSource::File { host_path, .. } => {
                    writeln!(stderr, "  - file: {}", host_path)
                }
                DetectedAuthSource::EnvVar { name, .. } => writeln!(stderr, "  - env: {}", name),
            }?;
        }
        for note in &detection.notes {
            writeln!(stderr, "note: {note}")?;
        }
        write!(stderr, "import this auth into the guest session? [y/N] ")?;
        stderr.flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim().to_ascii_lowercase();
        Ok(matches!(input.as_str(), "y" | "yes"))
    }
}

pub fn detect_auth(
    kind: ProviderKind,
    platform: HostPlatform,
    home_dir: &Path,
    env: &BTreeMap<String, String>,
    guest_home: &GuestPath,
) -> DetectedAuth {
    let mut sources = match kind {
        ProviderKind::Codex => detect_codex_auth(home_dir, guest_home),
        ProviderKind::Claude => detect_claude_auth(platform, home_dir, env, guest_home),
        ProviderKind::Gemini => detect_gemini_auth(env),
    };
    sources.sources.sort_by_key(sort_key_for_source);
    sources
}

pub fn select_auth_imports(
    provider: ProviderKind,
    detection: &DetectedAuth,
    interactive: bool,
    prompter: &dyn AuthPrompter,
) -> Result<Vec<DetectedAuthSource>, AppError> {
    if detection.sources.is_empty() || !interactive {
        return Ok(Vec::new());
    }
    if prompter.confirm_import(provider, detection)? {
        Ok(detection.sources.clone())
    } else {
        Ok(Vec::new())
    }
}

fn detect_codex_auth(home_dir: &Path, guest_home: &GuestPath) -> DetectedAuth {
    DetectedAuth {
        sources: plan_imported_files(ProviderKind::Codex, home_dir, guest_home)
            .into_iter()
            .map(|file| DetectedAuthSource::File {
                host_path: file.host_path,
                guest_path: file.guest_path,
            })
            .collect(),
        notes: Vec::new(),
    }
}

fn detect_claude_auth(
    platform: HostPlatform,
    home_dir: &Path,
    env: &BTreeMap<String, String>,
    guest_home: &GuestPath,
) -> DetectedAuth {
    let mut notes = Vec::new();
    let mut sources = detect_env_auth(env, CLAUDE_IMPORTABLE_ENV_KEYS);
    merge_missing_env_sources(&mut sources, detect_claude_settings_env(home_dir));

    if matches!(platform, HostPlatform::Macos) {
        notes.push(
            "Claude browser-login auth is stored in the macOS Keychain and is not importable automatically"
                .to_owned(),
        );
    } else {
        let credentials_path = home_dir.join(".claude/.credentials.json");
        if credentials_path.is_file() {
            sources.push(DetectedAuthSource::File {
                host_path: HostPath::new(&credentials_path),
                guest_path: GuestPath::new(
                    guest_home
                        .as_path()
                        .join(".claude")
                        .join(".credentials.json"),
                ),
            });
        }
    }

    DetectedAuth { sources, notes }
}

fn detect_claude_settings_env(home_dir: &Path) -> Vec<DetectedAuthSource> {
    let settings_path = home_dir.join(".claude").join("settings.json");
    let contents = match std::fs::read_to_string(&settings_path) {
        Ok(contents) => contents,
        Err(_) => return Vec::new(),
    };
    let json = match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(json) => json,
        Err(_) => return Vec::new(),
    };
    let env_object = match json.get("env").and_then(|value| value.as_object()) {
        Some(env_object) => env_object,
        None => return Vec::new(),
    };

    CLAUDE_IMPORTABLE_ENV_KEYS
        .iter()
        .filter_map(|key| {
            env_object.get(*key).and_then(|value| {
                value.as_str().map(|rendered| DetectedAuthSource::EnvVar {
                    name: (*key).to_owned(),
                    value: rendered.to_owned(),
                })
            })
        })
        .collect()
}

fn detect_gemini_auth(env: &BTreeMap<String, String>) -> DetectedAuth {
    DetectedAuth {
        sources: detect_env_auth(
            env,
            &[
                "GEMINI_API_KEY",
                "GOOGLE_API_KEY",
                "GOOGLE_GEMINI_BASE_URL",
                "GOOGLE_APPLICATION_CREDENTIALS",
                "GOOGLE_CLOUD_PROJECT",
                "GOOGLE_CLOUD_LOCATION",
            ],
        ),
        notes: Vec::new(),
    }
}

fn detect_env_auth(env: &BTreeMap<String, String>, keys: &[&str]) -> Vec<DetectedAuthSource> {
    keys.iter()
        .filter_map(|key| {
            env.get(*key).map(|value| DetectedAuthSource::EnvVar {
                name: (*key).to_owned(),
                value: value.clone(),
            })
        })
        .collect()
}

fn merge_missing_env_sources(
    sources: &mut Vec<DetectedAuthSource>,
    additional: impl IntoIterator<Item = DetectedAuthSource>,
) {
    let existing_names = sources
        .iter()
        .filter_map(|source| match source {
            DetectedAuthSource::EnvVar { name, .. } => Some(name.clone()),
            DetectedAuthSource::File { .. } => None,
        })
        .collect::<std::collections::BTreeSet<_>>();

    sources.extend(additional.into_iter().filter(|source| match source {
        DetectedAuthSource::EnvVar { name, .. } => !existing_names.contains(name),
        DetectedAuthSource::File { .. } => true,
    }));
}

fn sort_key_for_source(source: &DetectedAuthSource) -> String {
    match source {
        DetectedAuthSource::File { host_path, .. } => format!("file:{}", host_path),
        DetectedAuthSource::EnvVar { name, .. } => format!("env:{name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn codex_auth_detection_only_includes_auth_json() {
        let home = tempdir().expect("tempdir");
        let codex_dir = home.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).expect("codex dir");
        std::fs::write(codex_dir.join("auth.json"), "{\"auth_mode\":\"chatgpt\"}")
            .expect("auth file");
        std::fs::write(codex_dir.join("config.toml"), "model = 'gpt-5.4'").expect("config file");

        let detected = detect_auth(
            ProviderKind::Codex,
            HostPlatform::Macos,
            home.path(),
            &BTreeMap::new(),
            &GuestPath::new("/home/tester.guest"),
        );

        assert_eq!(
            detected.sources,
            vec![DetectedAuthSource::File {
                host_path: HostPath::new(home.path().join(".codex/auth.json")),
                guest_path: GuestPath::new("/home/tester.guest/.codex/auth.json"),
            }]
        );
    }

    #[test]
    fn claude_on_macos_prefers_env_auth_and_ignores_host_config_blob() {
        let home = tempdir().expect("tempdir");
        std::fs::write(home.path().join(".claude.json"), "{\"projects\":{}}").expect("config file");
        let mut env = BTreeMap::new();
        env.insert("ANTHROPIC_API_KEY".to_owned(), "sk-ant-test".to_owned());

        let detected = detect_auth(
            ProviderKind::Claude,
            HostPlatform::Macos,
            home.path(),
            &env,
            &GuestPath::new("/home/tester.guest"),
        );

        assert_eq!(
            detected.sources,
            vec![DetectedAuthSource::EnvVar {
                name: "ANTHROPIC_API_KEY".to_owned(),
                value: "sk-ant-test".to_owned(),
            }]
        );
        assert!(
            detected
                .notes
                .iter()
                .any(|note| note.contains("macOS Keychain")),
            "expected an explanatory note about Claude auth on macOS: {:?}",
            detected.notes
        );
    }

    #[test]
    fn claude_on_macos_imports_auth_env_from_settings_json_only() {
        let home = tempdir().expect("tempdir");
        let claude_dir = home.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).expect("claude dir");
        std::fs::write(
            claude_dir.join("settings.json"),
            serde_json::json!({
                "env": {
                    "CLAUDE_CODE_USE_BEDROCK": "1",
                    "CLAUDE_CODE_SKIP_BEDROCK_AUTH": "1",
                    "ANTHROPIC_BEDROCK_BASE_URL": "https://gateway.example/claude",
                    "ANTHROPIC_AUTH_TOKEN": "bedrock-token",
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "should-not-import"
                }
            })
            .to_string(),
        )
        .expect("settings file");

        let detected = detect_auth(
            ProviderKind::Claude,
            HostPlatform::Macos,
            home.path(),
            &BTreeMap::new(),
            &GuestPath::new("/home/tester.guest"),
        );

        assert_eq!(
            detected.sources,
            vec![
                DetectedAuthSource::EnvVar {
                    name: "ANTHROPIC_AUTH_TOKEN".to_owned(),
                    value: "bedrock-token".to_owned(),
                },
                DetectedAuthSource::EnvVar {
                    name: "ANTHROPIC_BEDROCK_BASE_URL".to_owned(),
                    value: "https://gateway.example/claude".to_owned(),
                },
                DetectedAuthSource::EnvVar {
                    name: "CLAUDE_CODE_SKIP_BEDROCK_AUTH".to_owned(),
                    value: "1".to_owned(),
                },
                DetectedAuthSource::EnvVar {
                    name: "CLAUDE_CODE_USE_BEDROCK".to_owned(),
                    value: "1".to_owned(),
                },
            ]
        );
        assert!(
            detected
                .sources
                .iter()
                .all(|source| !matches!(source, DetectedAuthSource::EnvVar { name, .. } if name == "ANTHROPIC_DEFAULT_HAIKU_MODEL")),
            "non-auth Claude settings env should not be imported"
        );
    }

    #[test]
    fn claude_host_env_overrides_settings_json_for_same_auth_key() {
        let home = tempdir().expect("tempdir");
        let claude_dir = home.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).expect("claude dir");
        std::fs::write(
            claude_dir.join("settings.json"),
            serde_json::json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "settings-token",
                    "ANTHROPIC_BEDROCK_BASE_URL": "https://gateway.example/claude"
                }
            })
            .to_string(),
        )
        .expect("settings file");
        let mut env = BTreeMap::new();
        env.insert(
            "ANTHROPIC_AUTH_TOKEN".to_owned(),
            "process-env-token".to_owned(),
        );

        let detected = detect_auth(
            ProviderKind::Claude,
            HostPlatform::Macos,
            home.path(),
            &env,
            &GuestPath::new("/home/tester.guest"),
        );

        assert!(
            detected.sources.contains(&DetectedAuthSource::EnvVar {
                name: "ANTHROPIC_AUTH_TOKEN".to_owned(),
                value: "process-env-token".to_owned(),
            }),
            "process env value should win over settings.json"
        );
        assert!(
            !detected.sources.contains(&DetectedAuthSource::EnvVar {
                name: "ANTHROPIC_AUTH_TOKEN".to_owned(),
                value: "settings-token".to_owned(),
            }),
            "settings.json fallback should not duplicate or override process env auth"
        );
        assert!(
            detected.sources.contains(&DetectedAuthSource::EnvVar {
                name: "ANTHROPIC_BEDROCK_BASE_URL".to_owned(),
                value: "https://gateway.example/claude".to_owned(),
            }),
            "other allowed settings env should still be imported"
        );
    }

    #[test]
    fn gemini_detection_imports_env_auth_but_not_settings_json() {
        let home = tempdir().expect("tempdir");
        let gemini_dir = home.path().join(".gemini");
        std::fs::create_dir_all(&gemini_dir).expect("gemini dir");
        std::fs::write(
            gemini_dir.join("settings.json"),
            "{\"security\":{\"auth\":{}}}",
        )
        .expect("settings file");
        let mut env = BTreeMap::new();
        env.insert("GEMINI_API_KEY".to_owned(), "gem-test".to_owned());
        env.insert(
            "GOOGLE_GEMINI_BASE_URL".to_owned(),
            "https://gateway.example/gemini".to_owned(),
        );

        let detected = detect_auth(
            ProviderKind::Gemini,
            HostPlatform::Macos,
            home.path(),
            &env,
            &GuestPath::new("/home/tester.guest"),
        );

        assert_eq!(
            detected.sources,
            vec![
                DetectedAuthSource::EnvVar {
                    name: "GEMINI_API_KEY".to_owned(),
                    value: "gem-test".to_owned(),
                },
                DetectedAuthSource::EnvVar {
                    name: "GOOGLE_GEMINI_BASE_URL".to_owned(),
                    value: "https://gateway.example/gemini".to_owned(),
                }
            ]
        );
    }

    #[derive(Default)]
    struct StubPrompter {
        called: Cell<u32>,
        answer: bool,
    }

    impl AuthPrompter for StubPrompter {
        fn confirm_import(
            &self,
            _provider: ProviderKind,
            _detection: &DetectedAuth,
        ) -> Result<bool, crate::error::AppError> {
            self.called.set(self.called.get() + 1);
            Ok(self.answer)
        }
    }

    #[test]
    fn noninteractive_mode_imports_nothing_and_does_not_prompt() {
        let detection = DetectedAuth {
            sources: vec![DetectedAuthSource::EnvVar {
                name: "OPENAI_API_KEY".to_owned(),
                value: "sk-test".to_owned(),
            }],
            notes: Vec::new(),
        };
        let prompter = StubPrompter {
            answer: true,
            ..Default::default()
        };

        let selected = select_auth_imports(ProviderKind::Codex, &detection, false, &prompter)
            .expect("selection");

        assert!(selected.is_empty());
        assert_eq!(prompter.called.get(), 0);
    }

    #[test]
    fn interactive_mode_imports_detected_auth_after_confirmation() {
        let detection = DetectedAuth {
            sources: vec![DetectedAuthSource::File {
                host_path: HostPath::new(PathBuf::from("/tmp/auth.json")),
                guest_path: GuestPath::new("/home/tester.guest/.codex/auth.json"),
            }],
            notes: Vec::new(),
        };
        let prompter = StubPrompter {
            answer: true,
            ..Default::default()
        };

        let selected = select_auth_imports(ProviderKind::Codex, &detection, true, &prompter)
            .expect("selection");

        assert_eq!(selected, detection.sources);
        assert_eq!(prompter.called.get(), 1);
    }
}
