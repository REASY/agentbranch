use crate::cli::JsonFlag;
use crate::db::sessions::list_sessions;
use crate::error::{AppError, ValidationError};
use crate::lima::base_info::{BaseSummary, ReadinessIssue, summarize_expected_base};
use crate::lima::fingerprint::CURRENT_PROVISION_FINGERPRINT;
use crate::lima::instance::list_instances;
use crate::platform::host::{DoctorChecks, HostPrereqs, collect_host_prereqs};
use crate::provider::import::detect_host_files;
use crate::types::ProviderKind;
use crate::util::process::RealCommandRunner;
use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeMap;
use std::time::Duration;

pub fn run_for_test(prereqs: HostPrereqs) -> Result<DoctorChecks, AppError> {
    Ok(DoctorChecks::from_prereqs(prereqs))
}

pub fn render_json(
    ok: bool,
    platform: &str,
    lima_version: Option<&str>,
    state_root: &str,
    provider_config_paths: &BTreeMap<String, Vec<String>>,
    prepared_base: &BaseSummary,
    messages: &[String],
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&serde_json::json!({
        "ok": ok,
        "platform": platform,
        "lima_version": lima_version,
        "state_root": state_root,
        "provider_config_paths": provider_config_paths,
        "prepared_base": prepared_base,
        "messages": messages,
    }))
}

pub fn run(args: JsonFlag) -> Result<(), AppError> {
    let runner = RealCommandRunner;
    let prereqs = collect_host_prereqs(&runner)?;
    let mut report = DoctorChecks::from_prereqs(prereqs.clone());

    let host = crate::platform::host::HostContext::detect()?;
    let provider_config_paths = supported_provider_config_paths(&host.home_dir);
    let conn = match inspect_catalog(&host.state_roots.db) {
        Ok(conn) => conn,
        Err(err) => {
            report.ok = false;
            report.messages.push(format!("catalog unavailable: {err}"));
            None
        }
    };

    let mut prepared_base =
        summarize_expected_base(host.platform, &[], CURRENT_PROVISION_FINGERPRINT);
    if prereqs.lima_available {
        match list_instances(&runner) {
            Ok(instances) => {
                prepared_base = summarize_expected_base(
                    host.platform,
                    &instances,
                    CURRENT_PROVISION_FINGERPRINT,
                );
                if let Some(advisory) = prepared_base_advisory(&prepared_base) {
                    report.messages.push(advisory.to_owned());
                }
                let session_vm_names = if let Some(conn) = conn.as_ref() {
                    match list_sessions(conn) {
                        Ok(rows) => rows
                            .into_iter()
                            .map(|row| row.vm_name.to_string())
                            .collect::<std::collections::BTreeSet<_>>(),
                        Err(err) => {
                            report.ok = false;
                            report
                                .messages
                                .push(format!("failed to read sessions: {err}"));
                            std::collections::BTreeSet::new()
                        }
                    }
                } else {
                    std::collections::BTreeSet::new()
                };
                let prepared_base = crate::util::ids::prepared_base_name(host.platform);
                let orphaned = instances
                    .iter()
                    .filter(|instance| {
                        instance.name.starts_with("agbranch-")
                            && !instance.name.starts_with("agbranch-base-")
                            && instance.name != prepared_base.as_str()
                            && !session_vm_names.contains(&instance.name)
                    })
                    .map(|instance| instance.name.clone())
                    .collect::<Vec<_>>();
                if !orphaned.is_empty() {
                    report.ok = false;
                    report
                        .messages
                        .push(format!("orphaned Lima instances: {}", orphaned.join(", ")));
                }
            }
            Err(err) => {
                report.ok = false;
                report
                    .messages
                    .push(format!("failed to inspect Lima instances: {err}"));
            }
        }
    }

    if args.json {
        println!(
            "{}",
            render_json(
                report.ok,
                prereqs.platform.as_str(),
                prereqs
                    .lima_version
                    .as_ref()
                    .map(semver::Version::to_string)
                    .as_deref(),
                host.state_roots.base.to_string_lossy().as_ref(),
                &provider_config_paths,
                &prepared_base,
                &report.messages,
            )
            .map_err(crate::error::observability::ObservabilityError::Json)?
        );
        return if report.ok {
            Ok(())
        } else {
            Err(AppError::Validation(ValidationError::DoctorReportIssues {
                messages: report.messages.join("; "),
            }))
        };
    }

    let state_root = host.state_roots.base.display();
    if report.ok {
        if report.messages.is_empty() {
            println!("doctor: ok");
        } else {
            for message in &report.messages {
                println!("{message}");
            }
        }
        println!("state root: {state_root}");
        Ok(())
    } else {
        for message in &report.messages {
            eprintln!("{message}");
        }
        eprintln!("state root: {state_root}");
        Err(AppError::Validation(ValidationError::DoctorReportIssues {
            messages: report.messages.join("; "),
        }))
    }
}

fn prepared_base_advisory(summary: &BaseSummary) -> Option<&'static str> {
    match summary.readiness_issue() {
        Some(ReadinessIssue::Missing) => Some("prepared base missing: run agbranch base prepare"),
        Some(ReadinessIssue::MetadataMissing) => {
            Some("prepared base metadata missing: run agbranch base prepare")
        }
        Some(ReadinessIssue::Stale) => {
            Some("prepared base stale: run agbranch base prepare --rebuild")
        }
        Some(ReadinessIssue::Unprotected) => {
            Some("prepared base unprotected: run agbranch base prepare")
        }
        None => None,
    }
}

fn supported_provider_config_paths(home_dir: &std::path::Path) -> BTreeMap<String, Vec<String>> {
    [
        ProviderKind::Codex,
        ProviderKind::Claude,
        ProviderKind::Gemini,
    ]
    .into_iter()
    .map(|kind| {
        (
            kind.as_str().to_owned(),
            detect_host_files(kind, home_dir)
                .into_iter()
                .map(|path| path.to_string())
                .collect::<Vec<_>>(),
        )
    })
    .collect()
}

fn inspect_catalog(path: &std::path::Path) -> Result<Option<Connection>, rusqlite::Error> {
    if !path.exists() {
        return Ok(None);
    }

    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        "#,
    )?;
    Ok(Some(conn))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lima::base_info::BaseSummary;
    use std::collections::BTreeMap;

    #[test]
    fn doctor_json_contains_ok_and_messages() {
        let mut provider_configs = BTreeMap::new();
        provider_configs.insert(
            "codex".to_owned(),
            vec!["/Users/tester/.codex/auth.json".to_owned()],
        );
        let prepared_base = BaseSummary {
            name: "agbranch-base-macos".to_owned(),
            name_source: "default".to_owned(),
            status: "missing".to_owned(),
            location: None,
            protected: false,
            prepared: false,
            size_bytes: None,
            disk_bytes: None,
            created_at: None,
            prepared_at: None,
            provision_fingerprint: "sha256:current".to_owned(),
            prepared_provision_fingerprint: None,
            provision_stale: None,
            agent_cli_versions: BTreeMap::new(),
        };
        let rendered = render_json(
            true,
            "macos",
            Some("2.1.1"),
            "/tmp/agbranch-smoke-state",
            &provider_configs,
            &prepared_base,
            &[String::from("ready")],
        )
        .expect("render json");
        assert!(rendered.contains("\"ok\":true"));
        assert!(rendered.contains("\"platform\":\"macos\""));
        assert!(rendered.contains("\"lima_version\":\"2.1.1\""));
        assert!(rendered.contains("\"state_root\":\"/tmp/agbranch-smoke-state\""));
        assert!(rendered.contains(
            "\"provider_config_paths\":{\"codex\":[\"/Users/tester/.codex/auth.json\"]}"
        ));
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid json");
        assert_eq!(value["prepared_base"]["name"], "agbranch-base-macos");
        assert!(rendered.contains("\"messages\":[\"ready\"]"));
    }

    #[test]
    fn doctor_json_contains_required_machine_fields() {
        let prepared_base = BaseSummary {
            name: "agbranch-base-macos".to_owned(),
            name_source: "default".to_owned(),
            status: "stopped".to_owned(),
            location: Some("/tmp/agbranch-base-macos".to_owned()),
            protected: true,
            prepared: true,
            size_bytes: Some(1),
            disk_bytes: Some(2),
            created_at: Some("2026-04-25T00:00:00Z".to_owned()),
            prepared_at: Some("2026-04-25T00:01:00Z".to_owned()),
            provision_fingerprint: "sha256:current".to_owned(),
            prepared_provision_fingerprint: Some("sha256:current".to_owned()),
            provision_stale: Some(false),
            agent_cli_versions: BTreeMap::new(),
        };
        let rendered = render_json(
            true,
            "macos",
            Some("2.1.1"),
            "/tmp/agbranch-smoke-state",
            &BTreeMap::new(),
            &prepared_base,
            &[String::from("ready")],
        )
        .expect("doctor json");

        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid json");
        assert_eq!(value["ok"], true);
        assert_eq!(value["platform"], "macos");
        assert_eq!(value["lima_version"], "2.1.1");
        assert_eq!(value["state_root"], "/tmp/agbranch-smoke-state");
        assert_eq!(value["prepared_base"]["status"], "stopped");
    }

    #[test]
    fn prepared_base_advisory_is_mutually_exclusive_and_prioritized() {
        let mut stale = BaseSummary {
            name: "agbranch-base-macos".to_owned(),
            name_source: "default".to_owned(),
            status: "stopped".to_owned(),
            location: Some("/tmp/agbranch-base-macos".to_owned()),
            protected: true,
            prepared: true,
            size_bytes: None,
            disk_bytes: None,
            created_at: None,
            prepared_at: Some("2026-04-25T00:01:00Z".to_owned()),
            provision_fingerprint: "sha256:current".to_owned(),
            prepared_provision_fingerprint: Some("sha256:old".to_owned()),
            provision_stale: Some(true),
            agent_cli_versions: BTreeMap::new(),
        };
        assert_eq!(
            prepared_base_advisory(&stale),
            Some("prepared base stale: run agbranch base prepare --rebuild")
        );

        stale.status = "missing".to_owned();
        stale.prepared = false;
        stale.protected = false;
        stale.prepared_provision_fingerprint = None;
        stale.provision_stale = None;
        assert_eq!(
            prepared_base_advisory(&stale),
            Some("prepared base missing: run agbranch base prepare")
        );
    }
}
