use crate::cli::{LogSource, LogsArgs};
use crate::db::{
    connect::open_catalog,
    events::{SessionEventRow, list_events},
    sync_runs::{SyncRunRow, list_sync_runs_for_session},
};
use crate::error::{AppError, ValidationError, observability::ObservabilityError};
use crate::lima::shell::{SshCommandSpec, build_ssh_command};
use crate::platform::host::HostContext;
use crate::session::exec::{ensure_instance_running, resolve_connection, run_host_command};
use crate::types::SessionName;
use crate::util::process::{CommandRunner, RealCommandRunner};
use crate::util::signals::install_interrupt_flag;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

const LOG_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Serialize)]
struct EventLogLine {
    source: &'static str,
    session: String,
    at: crate::types::Timestamp,
    level: String,
    kind: String,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct SyncLogLine {
    source: &'static str,
    session: String,
    id: i64,
    direction: String,
    result: String,
    started_at: crate::types::Timestamp,
    finished_at: Option<crate::types::Timestamp>,
    staging_path: Option<String>,
    patch_path: Option<String>,
    error_text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct FileLogOutput {
    source: String,
    session: String,
    path: PathBuf,
    contents: String,
}

pub fn run(args: LogsArgs) -> Result<(), AppError> {
    let session_name_raw = args.session.resolve_owned()?;
    let session_name = SessionName::try_from(session_name_raw.as_str())?;
    match args.source {
        LogSource::Events => run_event_logs(&session_name, args.follow, args.json),
        LogSource::Sync => run_sync_logs(&session_name, args.follow, args.json),
        LogSource::Provision => run_file_logs(&session_name, "provision", args.follow, args.json),
        LogSource::Guest => run_guest_logs(&session_name, false, args.follow, args.json),
        LogSource::Kernel => run_guest_logs(&session_name, true, args.follow, args.json),
    }
}

fn run_event_logs(session_name: &SessionName, follow: bool, json: bool) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let conn = open_catalog(&host.state_roots.db).map_err(ObservabilityError::from)?;
    let interrupted = if follow {
        Some(install_interrupt_flag()?)
    } else {
        None
    };
    let mut last_id = 0_i64;

    loop {
        let events = list_events(&conn, Some(session_name)).map_err(ObservabilityError::from)?;
        for event in &events {
            if event.id <= last_id {
                continue;
            }
            emit_event_line(event, json)?;
            last_id = event.id;
        }
        if !follow {
            return Ok(());
        }
        if interrupted
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            return Ok(());
        }
        thread::sleep(LOG_POLL_INTERVAL);
    }
}

fn run_sync_logs(session_name: &SessionName, follow: bool, json: bool) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let sync_log_path = host
        .state_roots
        .logs
        .join(session_name.as_str())
        .join("sync.log");
    if sync_log_path.exists() {
        return run_file_logs(session_name, "sync", follow, json);
    }

    let host = HostContext::detect()?;
    let conn = open_catalog(&host.state_roots.db).map_err(ObservabilityError::from)?;
    let interrupted = if follow {
        Some(install_interrupt_flag()?)
    } else {
        None
    };
    let mut last_id = 0_i64;

    loop {
        let runs =
            list_sync_runs_for_session(&conn, session_name).map_err(ObservabilityError::from)?;
        for run in runs.iter().rev() {
            if run.id <= last_id {
                continue;
            }
            emit_sync_line(run, json)?;
            last_id = run.id;
        }
        if !follow {
            return Ok(());
        }
        if interrupted
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            return Ok(());
        }
        thread::sleep(LOG_POLL_INTERVAL);
    }
}

fn run_file_logs(
    session_name: &SessionName,
    kind: &str,
    follow: bool,
    json: bool,
) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let path = host
        .state_roots
        .logs
        .join(session_name.as_str())
        .join(format!("{kind}.log"));
    if !path.exists() {
        return Err(AppError::Validation(
            ValidationError::LogSourceNotAvailable {
                kind: kind.to_string(),
                session: session_name.to_string(),
            },
        ));
    }

    if follow && !json {
        run_follow_tail(&path)?;
        return Ok(());
    }
    if follow && json {
        return Err(AppError::Validation(
            ValidationError::LogsFollowJsonUnsupported {
                channel: "file-backed",
            },
        ));
    }

    let contents = fs::read_to_string(&path).map_err(|source| ObservabilityError::Io {
        path: path.clone(),
        source,
    })?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&FileLogOutput {
                source: kind.to_owned(),
                session: session_name.to_string(),
                path,
                contents,
            })
            .map_err(ObservabilityError::from)?
        );
    } else {
        print!("{contents}");
    }
    Ok(())
}

fn run_guest_logs(
    session_name: &SessionName,
    kernel: bool,
    follow: bool,
    json: bool,
) -> Result<(), AppError> {
    if follow && json {
        return Err(AppError::Validation(
            ValidationError::LogsFollowJsonUnsupported { channel: "guest" },
        ));
    }

    let connection = resolve_connection(session_name)?;
    ensure_instance_running(&connection)?;
    let command = if kernel {
        let mut command = vec![
            "journalctl".to_owned(),
            "--no-pager".to_owned(),
            "-k".to_owned(),
        ];
        if follow {
            command.push("--follow".to_owned());
        } else {
            command.push("-n".to_owned());
            command.push("200".to_owned());
        }
        command
    } else if follow {
        vec![
            "journalctl".to_owned(),
            "--follow".to_owned(),
            "--no-pager".to_owned(),
        ]
    } else {
        vec![
            "journalctl".to_owned(),
            "--no-pager".to_owned(),
            "-n".to_owned(),
            "200".to_owned(),
        ]
    };

    let ssh_args = build_ssh_command(SshCommandSpec {
        ssh_config_file: &connection.ssh_config_file,
        host_alias: &connection.host_alias,
        session: connection.session_name.as_str(),
        workdir: connection.guest_repo_path.as_path(),
        forward_agent: false,
        force_tty: false,
        guest_secret_file: None,
        command: Some(&command),
    });

    if follow {
        run_host_command("ssh", &ssh_args)
    } else {
        let output = RealCommandRunner
            .run("ssh", &ssh_args, None, &Default::default())
            .map_err(ObservabilityError::from)?;
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "source": if kernel { "kernel" } else { "guest" },
                    "session": session_name,
                    "output": output.stdout,
                })
            );
        } else {
            print!("{}", output.stdout);
        }
        Ok(())
    }
}

fn run_follow_tail(path: &Path) -> Result<(), AppError> {
    let args = vec![
        "-n".to_owned(),
        "+1".to_owned(),
        "-f".to_owned(),
        path.display().to_string(),
    ];
    run_host_command("tail", &args)
}

fn emit_event_line(event: &SessionEventRow, json: bool) -> Result<(), ObservabilityError> {
    let line = EventLogLine {
        source: "events",
        session: event.session_id.to_string(),
        at: event.at,
        level: event.level.to_string(),
        kind: event.kind.clone(),
        message: event.message.clone(),
    };
    if json {
        println!("{}", serde_json::to_string(&line)?);
    } else {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            line.at, line.level, line.kind, line.session, line.message
        );
    }
    Ok(())
}

fn emit_sync_line(run: &SyncRunRow, json: bool) -> Result<(), ObservabilityError> {
    let line = SyncLogLine {
        source: "sync",
        session: run.session_id.to_string(),
        id: run.id,
        direction: run.direction.to_string(),
        result: run.result.to_string(),
        started_at: run.started_at,
        finished_at: run.finished_at,
        staging_path: run.staging_path.clone(),
        patch_path: run.patch_path.clone(),
        error_text: run.error_text.clone(),
    };
    if json {
        println!("{}", serde_json::to_string(&line)?);
    } else {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            line.started_at,
            line.direction,
            line.result,
            line.session,
            line.error_text.as_deref().unwrap_or("")
        );
    }
    Ok(())
}
