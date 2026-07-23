use crate::cli::{
    AuthAction, AuthArgs, AuthPreferencePolicy, AuthResetArgs, AuthSetArgs, JsonFlag,
};
use crate::db::connect::open_catalog;
use crate::db::preferences::{
    clear_auth_imports, forget_auth_import, remember_auth_import, remembered_auth_import,
};
use crate::error::AppError;
use crate::platform::host::HostContext;
use crate::types::ProviderKind;
use serde::Serialize;

const PROVIDERS: [ProviderKind; 3] = [
    ProviderKind::Codex,
    ProviderKind::Claude,
    ProviderKind::Gemini,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AuthPreferenceView {
    provider: &'static str,
    policy: Option<&'static str>,
}

pub fn run(args: AuthArgs) -> Result<(), AppError> {
    match args.action {
        AuthAction::List(args) => list(args),
        AuthAction::Set(args) => set(args),
        AuthAction::Reset(args) => reset(args),
    }
}

fn list(args: JsonFlag) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let catalog = open_catalog(&host.state_roots.db)?;
    let preferences = preference_views(&catalog)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string(&preferences)
                .map_err(crate::error::observability::ObservabilityError::from)?
        );
    } else {
        println!("{:<10} POLICY", "PROVIDER");
        for preference in preferences {
            println!(
                "{:<10} {}",
                preference.provider,
                preference.policy.unwrap_or("unset")
            );
        }
    }
    Ok(())
}

fn set(args: AuthSetArgs) -> Result<(), AppError> {
    let provider = ProviderKind::from(args.provider);
    let host = HostContext::detect()?;
    let catalog = open_catalog(&host.state_roots.db)?;
    remember_auth_import(&catalog, provider, args.policy.import())?;
    let policy = policy_name(args.policy);
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "provider": provider.as_str(),
                "policy": policy,
                "status": "set",
            })
        );
    } else {
        println!("{} auth preference set to {policy}", provider.as_str());
    }
    Ok(())
}

fn reset(args: AuthResetArgs) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let catalog = open_catalog(&host.state_roots.db)?;
    let (provider, reset_count) = if args.all {
        (None, clear_auth_imports(&catalog)?)
    } else {
        let provider = ProviderKind::from(args.provider.expect("clap requires provider or --all"));
        let reset = usize::from(forget_auth_import(&catalog, provider)?);
        (Some(provider), reset)
    };
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "provider": provider.map(ProviderKind::as_str),
                "reset_count": reset_count,
                "status": "reset",
            })
        );
    } else if let Some(provider) = provider {
        println!(
            "{} auth preference reset{}",
            provider.as_str(),
            if reset_count == 0 {
                " (already unset)"
            } else {
                ""
            }
        );
    } else {
        println!("reset {reset_count} auth preference(s)");
    }
    Ok(())
}

fn preference_views(catalog: &rusqlite::Connection) -> Result<Vec<AuthPreferenceView>, AppError> {
    PROVIDERS
        .into_iter()
        .map(|provider| {
            Ok(AuthPreferenceView {
                provider: provider.as_str(),
                policy: remembered_auth_import(catalog, provider)?
                    .map(|import| if import { "import" } else { "none" }),
            })
        })
        .collect()
}

fn policy_name(policy: AuthPreferencePolicy) -> &'static str {
    match policy {
        AuthPreferencePolicy::Import => "import",
        AuthPreferencePolicy::None => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::open_catalog;
    use tempfile::tempdir;

    #[test]
    fn preference_views_include_unset_providers_in_stable_order() {
        let dir = tempdir().expect("tempdir");
        let catalog = open_catalog(&dir.path().join("state.db")).expect("catalog");
        remember_auth_import(&catalog, ProviderKind::Claude, false).expect("remember");

        assert_eq!(
            preference_views(&catalog).expect("views"),
            vec![
                AuthPreferenceView {
                    provider: "codex",
                    policy: None,
                },
                AuthPreferenceView {
                    provider: "claude",
                    policy: Some("none"),
                },
                AuthPreferenceView {
                    provider: "gemini",
                    policy: None,
                },
            ]
        );
    }

    #[test]
    fn policy_names_match_launch_flags() {
        assert_eq!(policy_name(AuthPreferencePolicy::Import), "import");
        assert_eq!(policy_name(AuthPreferencePolicy::None), "none");
    }
}
