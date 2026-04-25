use crate::cli::{BaseAction, BaseArgs, PrepareArgs};
use crate::db::events::append_event;
use crate::db::locks::{LockMode, SessionLock, acquire_base_lock};
use crate::db::models::EventLevel;
use crate::error::AppError;
use crate::lima::base;
use crate::lima::base_info::{BaseSummary, ReadinessIssue, summarize_expected_base};
use crate::lima::fingerprint::CURRENT_PROVISION_FINGERPRINT;
use crate::lima::inspect::LimaInstance;
use crate::lima::instance::list_instances;
use crate::platform::detect::HostPlatform;
use crate::platform::host::HostContext;
use crate::types::SessionName;
use crate::util::process::{CommandRunner, RealCommandRunner};
use crate::util::time::utc_now;
use std::path::PathBuf;

use super::prepare;

pub fn run(args: BaseArgs) -> Result<(), AppError> {
    match args.action {
        BaseAction::Prepare(args) => prepare_with_lock(args),
        BaseAction::Show(args) => show(args),
    }
}

pub(crate) fn acquire_clone_lock_for_prepared_base<F>(
    runner: &dyn CommandRunner,
    host: &HostContext,
    clone_operation: &'static str,
    prepare_operation: &'static str,
    on_notice: &mut F,
) -> Result<SessionLock, AppError>
where
    F: FnMut(PreparedBaseNotice) -> Result<(), AppError>,
{
    let lock_path = base_lock_path(host);
    let pid = std::process::id();

    match prepare_strategy_for_runner(runner, host.platform)? {
        PrepareStrategy::Blocked(message) => Err(AppError::Blocked(message)),
        PrepareStrategy::Ready { notice } => {
            let lock = acquire_base_lock(&lock_path, pid, clone_operation, LockMode::Shared)?;
            if let Some(notice) = notice {
                on_notice(notice)?;
            }
            Ok(lock)
        }
        PrepareStrategy::Prepare { .. } => {
            let exclusive =
                acquire_base_lock(&lock_path, pid, prepare_operation, LockMode::Exclusive)?;
            let decision = prepare_strategy_for_runner(runner, host.platform)?;
            match decision {
                PrepareStrategy::Blocked(message) => Err(AppError::Blocked(message)),
                PrepareStrategy::Ready { notice } => {
                    drop(exclusive);
                    let lock =
                        acquire_base_lock(&lock_path, pid, clone_operation, LockMode::Shared)?;
                    if let Some(notice) = notice {
                        on_notice(notice)?;
                    }
                    Ok(lock)
                }
                PrepareStrategy::Prepare { rebuild, notice } => {
                    on_notice(notice)?;
                    let _ = base::prepare_base(runner, host.platform, rebuild)?;
                    drop(exclusive);
                    acquire_base_lock(&lock_path, pid, clone_operation, LockMode::Shared)
                }
            }
        }
    }
}

fn prepare_with_lock(args: PrepareArgs) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let _lock = acquire_base_lock(
        &base_lock_path(&host),
        std::process::id(),
        "base prepare",
        LockMode::Exclusive,
    )?;
    prepare::run(args)
}

fn show(args: crate::cli::BaseShowArgs) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let runner = RealCommandRunner;
    let instances = list_instances(&runner)?;
    let summary = summarize_expected_base(host.platform, &instances, CURRENT_PROVISION_FINGERPRINT);

    if args.require_ready
        && let Some(_issue) = summary.readiness_issue()
    {
        return Err(AppError::Blocked(summary.require_ready_error().to_owned()));
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string(&summary)
                .map_err(crate::error::observability::ObservabilityError::Json)?
        );
    } else {
        println!("{}", summary.render_human());
    }
    Ok(())
}

fn base_lock_path(host: &HostContext) -> PathBuf {
    host.state_roots.locks.join("base.lock")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedBaseNotice {
    pub level: EventLevel,
    pub kind: &'static str,
    pub message: String,
}

pub(crate) fn emit_prepared_base_notice(
    catalog: &rusqlite::Connection,
    session_name: &SessionName,
    notice: &PreparedBaseNotice,
) -> Result<(), AppError> {
    eprintln!("{}", notice.message);
    append_event(
        catalog,
        session_name,
        notice.level,
        notice.kind,
        &notice.message,
        utc_now(),
    )?;
    Ok(())
}

fn prepare_strategy_for_runner(
    runner: &dyn CommandRunner,
    platform: HostPlatform,
) -> Result<PrepareStrategy, AppError> {
    let instances = list_instances(runner)?;
    Ok(prepare_strategy_for_instances(platform, &instances))
}

fn prepare_strategy_for_instances(
    platform: HostPlatform,
    instances: &[LimaInstance],
) -> PrepareStrategy {
    let base_name = crate::util::ids::prepared_base_name(platform);
    let instance = instances
        .iter()
        .find(|item| item.name == base_name.as_str());
    let summary = summarize_expected_base(platform, instances, CURRENT_PROVISION_FINGERPRINT);
    prepare_strategy(&summary, instance)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrepareStrategy {
    Blocked(String),
    Ready {
        notice: Option<PreparedBaseNotice>,
    },
    Prepare {
        rebuild: bool,
        notice: PreparedBaseNotice,
    },
}

fn prepare_strategy(summary: &BaseSummary, instance: Option<&LimaInstance>) -> PrepareStrategy {
    if instance.is_some_and(LimaInstance::is_running) {
        return PrepareStrategy::Blocked(format!(
            "prepared base {} is running: stop it with limactl stop {} or run agbranch base prepare",
            summary.name, summary.name
        ));
    }

    if instance.is_some_and(base::prepared_base_requires_rebuild) {
        return PrepareStrategy::Prepare {
            rebuild: true,
            notice: prepared_base_notice(
                EventLevel::Info,
                "prepared_base.auto_rebuild",
                format!(
                    "prepared base {} has deprecated configuration; rebuilding it before clone",
                    summary.name
                ),
            ),
        };
    }

    match summary.readiness_issue() {
        None => PrepareStrategy::Ready { notice: None },
        Some(ReadinessIssue::Missing) => PrepareStrategy::Prepare {
            rebuild: false,
            notice: prepared_base_notice(
                EventLevel::Info,
                "prepared_base.auto_prepare",
                format!(
                    "prepared base {} is missing; preparing it now",
                    summary.name
                ),
            ),
        },
        Some(ReadinessIssue::MetadataMissing) => PrepareStrategy::Ready {
            notice: Some(prepared_base_notice(
                EventLevel::Warn,
                "prepared_base.warning",
                format!(
                    "prepared base {} metadata is missing or invalid; cloning anyway",
                    summary.name
                ),
            )),
        },
        Some(ReadinessIssue::Stale) => PrepareStrategy::Ready {
            notice: Some(prepared_base_notice(
                EventLevel::Warn,
                "prepared_base.warning",
                format!(
                    "prepared base {} is stale for this binary; cloning anyway",
                    summary.name
                ),
            )),
        },
        Some(ReadinessIssue::Unprotected) => PrepareStrategy::Ready {
            notice: Some(prepared_base_notice(
                EventLevel::Warn,
                "prepared_base.warning",
                format!(
                    "prepared base {} is unprotected; cloning anyway",
                    summary.name
                ),
            )),
        },
    }
}

fn prepared_base_notice(
    level: EventLevel,
    kind: &'static str,
    message: impl Into<String>,
) -> PreparedBaseNotice {
    PreparedBaseNotice {
        level,
        kind,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::open_catalog;
    use crate::db::events::list_events;
    use crate::db::models::EventLevel;
    use crate::db::sessions::insert_session;
    use crate::lima::base_info::{BaseMetadata, BaseSummary, BaseSummaryInput, NameSource};
    use crate::lima::inspect::{LimaConfig, LimaInstance, LimaInstanceStatus};
    use crate::testing::{host_context, test_sandbox_session, ts};
    use crate::util::ids::prepared_base_name;
    use crate::util::process::{CommandOutput, CommandRunner};
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn sample_instance(protected: bool) -> LimaInstance {
        LimaInstance {
            name: "agbranch-base-macos".to_owned(),
            instance_dir: "/tmp/agbranch-base-macos".to_owned(),
            ssh_config_file: "/tmp/agbranch-base-macos/ssh.config".to_owned(),
            vm_type: "vz".to_owned(),
            raw_status: "Stopped".to_owned(),
            arch: None,
            cpus: Some(4),
            memory: Some(8 * 1024 * 1024 * 1024),
            disk: Some(100 * 1024 * 1024 * 1024),
            protected,
            ssh_local_port: None,
            ssh_address: None,
            config: LimaConfig::default(),
            status: LimaInstanceStatus::Stopped,
        }
    }

    fn ready_summary(protected: bool) -> BaseSummary {
        BaseSummary::from_parts(BaseSummaryInput {
            name: "agbranch-base-macos".to_owned(),
            name_source: NameSource::Default,
            instance: Some(sample_instance(protected)),
            metadata: Some(BaseMetadata {
                schema_version: 1,
                prepared_at: "2026-04-25T00:00:00Z".to_owned(),
                provision_fingerprint: "sha256:current".to_owned(),
                agent_cli_versions: BTreeMap::new(),
            }),
            metadata_valid: true,
            current_fingerprint: "sha256:current".to_owned(),
            size_bytes: None,
            created_at: None,
        })
    }

    #[test]
    fn prepare_strategy_returns_ready_for_ready_base() {
        let instance = sample_instance(true);
        let summary = ready_summary(true);

        assert_eq!(
            prepare_strategy(&summary, Some(&instance)),
            PrepareStrategy::Ready { notice: None }
        );
    }

    #[test]
    fn prepare_strategy_requires_non_rebuild_prepare_for_unprotected_base() {
        let instance = sample_instance(false);
        let summary = ready_summary(false);

        assert_eq!(
            prepare_strategy(&summary, Some(&instance)),
            PrepareStrategy::Ready {
                notice: Some(prepared_base_notice(
                    EventLevel::Warn,
                    "prepared_base.warning",
                    "prepared base agbranch-base-macos is unprotected; cloning anyway",
                ))
            }
        );
    }

    #[test]
    fn prepare_strategy_warns_for_stale_base_but_rebuilds_structural_bases() {
        let stale_instance = sample_instance(true);
        let mut stale_summary = ready_summary(true);
        stale_summary.provision_stale = Some(true);

        assert_eq!(
            prepare_strategy(&stale_summary, Some(&stale_instance)),
            PrepareStrategy::Ready {
                notice: Some(prepared_base_notice(
                    EventLevel::Warn,
                    "prepared_base.warning",
                    "prepared base agbranch-base-macos is stale for this binary; cloning anyway",
                ))
            }
        );

        let mut mounted_instance = sample_instance(true);
        mounted_instance
            .config
            .mounts
            .push(crate::lima::inspect::LimaMount {
                location: "/Users/tester".to_owned(),
            });

        assert_eq!(
            prepare_strategy(&ready_summary(true), Some(&mounted_instance)),
            PrepareStrategy::Prepare {
                rebuild: true,
                notice: prepared_base_notice(
                    EventLevel::Info,
                    "prepared_base.auto_rebuild",
                    "prepared base agbranch-base-macos has deprecated configuration; rebuilding it before clone",
                )
            }
        );
    }

    #[test]
    fn prepare_strategy_requires_prepare_for_missing_base() {
        let summary =
            BaseSummary::missing("agbranch-base-macos", NameSource::Default, "sha256:current");

        assert_eq!(
            prepare_strategy(&summary, None),
            PrepareStrategy::Prepare {
                rebuild: false,
                notice: prepared_base_notice(
                    EventLevel::Info,
                    "prepared_base.auto_prepare",
                    "prepared base agbranch-base-macos is missing; preparing it now",
                )
            }
        );
    }

    #[derive(Default)]
    struct FakeRunner {
        list_outputs: RefCell<Vec<String>>,
        commands: RefCell<Vec<Vec<String>>>,
        notice_seen: Cell<bool>,
        assert_notice_before_create: Cell<bool>,
    }

    impl FakeRunner {
        fn with_list_outputs(list_outputs: Vec<String>) -> Self {
            Self {
                list_outputs: RefCell::new(list_outputs),
                ..Self::default()
            }
        }

        fn subcommands(&self) -> Vec<String> {
            self.commands
                .borrow()
                .iter()
                .map(|args| args.first().cloned().unwrap_or_default())
                .collect()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            program: &str,
            args: &[String],
            _cwd: Option<&Path>,
            _env: &BTreeMap<String, String>,
        ) -> Result<CommandOutput, crate::error::process::ProcessError> {
            assert_eq!(program, "limactl");
            self.commands.borrow_mut().push(args.to_vec());
            if args == ["list", "--json"] {
                let stdout = self.list_outputs.borrow_mut().remove(0);
                return Ok(CommandOutput {
                    stdout,
                    stderr: String::new(),
                });
            }
            if self.assert_notice_before_create.get()
                && args.first().map(String::as_str) == Some("create")
            {
                assert!(
                    self.notice_seen.get(),
                    "notice should be emitted before automatic prepare starts"
                );
            }
            Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn stale_base_clones_with_warning_without_rebuild() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let base_name = prepared_base_name(host.platform);
        let base_dir = dir.path().join(base_name.as_str());
        fs::create_dir_all(&base_dir).expect("base dir");
        fs::write(
            base_dir.join("agbranch-base.json"),
            serde_json::to_string(&BaseMetadata {
                schema_version: 1,
                prepared_at: "2026-04-25T04:24:06Z".to_owned(),
                provision_fingerprint: "sha256:old".to_owned(),
                agent_cli_versions: BTreeMap::new(),
            })
            .expect("metadata json"),
        )
        .expect("write metadata");
        let runner = FakeRunner::with_list_outputs(vec![format!(
            r#"[{{"name":"{}","dir":"{}","sshConfigFile":"{}/ssh.config","vmType":"vz","status":"Stopped","protected":true}}]"#,
            base_name.as_str(),
            base_dir.display(),
            base_dir.display()
        )]);
        let notices = RefCell::new(Vec::new());

        let lock = acquire_clone_lock_for_prepared_base(
            &runner,
            &host,
            "launch clone",
            "launch prepare-base",
            &mut |notice| {
                notices.borrow_mut().push(notice);
                Ok(())
            },
        )
        .expect("clone lock");

        drop(lock);
        assert_eq!(runner.subcommands(), vec!["list"]);
        let notices = notices.into_inner();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].level, EventLevel::Warn);
        assert_eq!(notices[0].kind, "prepared_base.warning");
        assert!(notices[0].message.contains("stale for this binary"));
    }

    #[test]
    fn running_base_is_blocked_before_clone() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let base_name = prepared_base_name(host.platform);
        let base_dir = dir.path().join(base_name.as_str());
        fs::create_dir_all(&base_dir).expect("base dir");
        fs::write(
            base_dir.join("agbranch-base.json"),
            serde_json::to_string(&BaseMetadata {
                schema_version: 1,
                prepared_at: "2026-04-25T04:24:06Z".to_owned(),
                provision_fingerprint: "sha256:current".to_owned(),
                agent_cli_versions: BTreeMap::new(),
            })
            .expect("metadata json"),
        )
        .expect("write metadata");
        let runner = FakeRunner::with_list_outputs(vec![format!(
            r#"[{{"name":"{}","dir":"{}","sshConfigFile":"{}/ssh.config","vmType":"vz","status":"Running","protected":true}}]"#,
            base_name.as_str(),
            base_dir.display(),
            base_dir.display()
        )]);
        let notices = RefCell::new(Vec::new());

        let err = acquire_clone_lock_for_prepared_base(
            &runner,
            &host,
            "launch clone",
            "launch prepare-base",
            &mut |notice| {
                notices.borrow_mut().push(notice);
                Ok(())
            },
        )
        .expect_err("running base should block clone");

        assert!(matches!(
            err,
            AppError::Blocked(message)
                if message
                    == "prepared base agbranch-base-macos is running: stop it with limactl stop agbranch-base-macos or run agbranch base prepare"
        ));
        assert_eq!(runner.subcommands(), vec!["list"]);
        assert!(notices.into_inner().is_empty());
    }

    #[test]
    fn missing_base_auto_prepare_emits_notice_before_prepare_steps() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let base_name = prepared_base_name(host.platform);
        let base_dir = dir.path().join(base_name.as_str());
        fs::create_dir_all(&base_dir).expect("base dir");
        let runner = FakeRunner::with_list_outputs(vec![
            "[]".to_owned(),
            "[]".to_owned(),
            "[]".to_owned(),
            format!(
                r#"[{{"name":"{}","dir":"{}","sshConfigFile":"{}/ssh.config","vmType":"vz","status":"Stopped","protected":false}}]"#,
                base_name.as_str(),
                base_dir.display(),
                base_dir.display()
            ),
        ]);
        runner.assert_notice_before_create.set(true);
        let notices = RefCell::new(Vec::new());

        let lock = acquire_clone_lock_for_prepared_base(
            &runner,
            &host,
            "launch clone",
            "launch prepare-base",
            &mut |notice| {
                runner.notice_seen.set(true);
                notices.borrow_mut().push(notice);
                Ok(())
            },
        )
        .expect("clone lock");

        drop(lock);
        let subcommands = runner.subcommands();
        assert_eq!(
            &subcommands[..5],
            ["list", "list", "list", "create", "start"]
        );
        assert!(subcommands.contains(&"protect".to_owned()));
        assert!(
            subcommands
                .iter()
                .filter(|entry| entry.as_str() == "shell")
                .count()
                >= 4,
            "prepare path should include readiness and version shell probes"
        );
        let notices = notices.into_inner();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].level, EventLevel::Info);
        assert_eq!(notices[0].kind, "prepared_base.auto_prepare");
        assert!(notices[0].message.contains("is missing; preparing it now"));
    }

    #[test]
    fn emitting_prepared_base_notice_records_session_event() {
        let dir = tempdir().expect("tempdir");
        let host = host_context(&dir);
        let conn = open_catalog(&host.state_roots.db).expect("catalog");
        let session = crate::types::SessionName::try_from("demo-base-notice").expect("session");
        insert_session(
            &conn,
            &test_sandbox_session(session.as_str(), ts("2026-04-25T00:00:00Z")),
        )
        .expect("seed session");
        let notice = prepared_base_notice(
            EventLevel::Warn,
            "prepared_base.warning",
            "prepared base agbranch-base-macos is stale for this binary; cloning anyway",
        );

        emit_prepared_base_notice(&conn, &session, &notice).expect("emit notice");

        let events = list_events(&conn, Some(&session)).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].level, EventLevel::Warn);
        assert_eq!(events[0].kind, "prepared_base.warning");
        assert_eq!(events[0].message, notice.message);
    }
}
