use crate::cli::ListArgs;
use crate::db::connect::open_catalog;
use crate::db::models::{LifecycleState, RepoSyncMode, SessionMode, SyncState};
use crate::db::sessions::{SessionRow, list_sessions};
use crate::error::AppError;
use crate::lima::client::{LimaClient, LimactlClient};
use crate::lima::inspect::{LimaInstance, LimaInstanceStatus};
use crate::platform::host::HostContext;
use crate::session::runtime::{
    RuntimeProbeTarget, format_bytes, format_ssh_endpoint, probe_guest_runtime,
};
use crate::types::{GuestPath, HostPath, ProviderKind, SessionName, Timestamp, VmName};
use crate::util::process::RealCommandRunner;
use comfy_table::{Attribute, Cell, ContentArrangement, Table, presets::NOTHING};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct PsSessionRow {
    pub name: SessionName,
    pub vm_name: VmName,
    pub session_mode: SessionMode,
    pub repo_sync_mode: Option<RepoSyncMode>,
    pub host_context_path: Option<HostPath>,
    pub guest_workspace_path: GuestPath,
    pub host_head_oid_at_open: Option<String>,
    pub host_head_ref_at_open: Option<String>,
    pub host_dirty_at_open: bool,
    pub lifecycle_state: LifecycleState,
    pub sync_state: SyncState,
    pub created_at: Timestamp,
    pub last_started_at: Option<Timestamp>,
    pub last_stopped_at: Option<Timestamp>,
    pub closed_at: Option<Timestamp>,
    pub provider_kind: Option<ProviderKind>,
    pub guest_tmux_socket_path: Option<GuestPath>,
    pub shell_window_name: Option<String>,
    pub agent_window_name: Option<String>,
}

impl From<SessionRow> for PsSessionRow {
    fn from(row: SessionRow) -> Self {
        Self {
            name: row.name,
            vm_name: row.vm_name,
            session_mode: row.session_mode,
            repo_sync_mode: row.repo_sync_mode,
            host_context_path: row.host_context_path,
            guest_workspace_path: row.guest_workspace_path,
            host_head_oid_at_open: row.host_head_oid_at_open,
            host_head_ref_at_open: row.host_head_ref_at_open,
            host_dirty_at_open: row.host_dirty_at_open,
            lifecycle_state: row.lifecycle_state,
            sync_state: row.sync_state,
            created_at: row.created_at,
            last_started_at: row.last_started_at,
            last_stopped_at: row.last_stopped_at,
            closed_at: row.closed_at,
            provider_kind: row.provider_kind,
            guest_tmux_socket_path: row.guest_tmux_socket_path,
            shell_window_name: row.shell_window_name,
            agent_window_name: row.agent_window_name,
        }
    }
}

pub fn render_table(
    sessions: &[PsSessionRow],
    live_instances: &[LimaInstance],
    live_agents: &BTreeMap<String, String>,
    include_all: bool,
    now: Timestamp,
) -> String {
    render_table_inner(
        sessions,
        live_instances,
        live_agents,
        include_all,
        now,
        false,
    )
}

pub fn render_table_styled(
    sessions: &[PsSessionRow],
    live_instances: &[LimaInstance],
    live_agents: &BTreeMap<String, String>,
    include_all: bool,
    now: Timestamp,
) -> String {
    render_table_inner(
        sessions,
        live_instances,
        live_agents,
        include_all,
        now,
        true,
    )
}

fn render_table_inner(
    sessions: &[PsSessionRow],
    live_instances: &[LimaInstance],
    live_agents: &BTreeMap<String, String>,
    include_all: bool,
    now: Timestamp,
    styled_headers: bool,
) -> String {
    let sessions = sessions
        .iter()
        .filter(|session| include_all || is_active_session(session.lifecycle_state))
        .collect::<Vec<_>>();
    let live_by_name = live_instances
        .iter()
        .map(|instance| (instance.name.as_str(), instance))
        .collect::<BTreeMap<_, _>>();
    let mut table = Table::new();
    table.load_preset(NOTHING);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    if styled_headers {
        table.force_no_tty().enforce_styling();
    } else {
        table.force_no_tty();
    }
    if include_all {
        table.set_header(
            [
                "SESSION",
                "VM",
                "STATE",
                "SYNC",
                "CREATED",
                "LAST STARTED",
                "LAST STOPPED",
                "CLOSED",
            ]
            .map(|value| header_cell(value, styled_headers)),
        );
        for session in sessions {
            table.add_row([
                session.name.to_string(),
                session.vm_name.to_string(),
                session.lifecycle_state.to_string(),
                session.sync_state.to_string(),
                session.created_at.to_string(),
                format_optional_timestamp(session.last_started_at),
                format_last_stopped_for_history(session),
                format_optional_timestamp(session.closed_at),
            ]);
        }
    } else {
        table.set_header(
            [
                "SESSION",
                "VM",
                "STATE",
                "UP FOR",
                "SYNC",
                "AGENT",
                "VM STATUS",
                "SSH",
                "VMTYPE",
                "ARCH",
                "CPUS",
                "MEMORY",
                "DISK",
            ]
            .map(|value| header_cell(value, styled_headers)),
        );
        for session in sessions {
            let live = live_by_name.get(session.vm_name.as_str()).copied();
            table.add_row([
                session.name.to_string(),
                session.vm_name.to_string(),
                session.lifecycle_state.to_string(),
                format_running_for(session, now),
                session.sync_state.to_string(),
                live_agents
                    .get(session.name.as_str())
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_owned()),
                live.map(|instance| instance.raw_status.clone())
                    .unwrap_or_else(|| "-".to_owned()),
                live.and_then(format_ssh_endpoint)
                    .unwrap_or_else(|| "-".to_owned()),
                live.map(|instance| instance.vm_type.clone())
                    .unwrap_or_else(|| "-".to_owned()),
                live.and_then(|instance| instance.arch.clone())
                    .unwrap_or_else(|| "-".to_owned()),
                live.and_then(|instance| instance.cpus.map(|value| value.to_string()))
                    .unwrap_or_else(|| "-".to_owned()),
                live.and_then(|instance| instance.memory.map(format_bytes))
                    .unwrap_or_else(|| "-".to_owned()),
                live.and_then(|instance| instance.disk.map(format_bytes))
                    .unwrap_or_else(|| "-".to_owned()),
            ]);
        }
    }
    for column in table.column_iter_mut() {
        column.set_padding((0, 2));
    }
    table.trim_fmt()
}

pub fn run(args: ListArgs) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let conn = open_catalog(&host.state_roots.db)?;
    let sessions = list_sessions(&conn)?
        .into_iter()
        .map(PsSessionRow::from)
        .collect::<Vec<_>>();
    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    let live_instances = match lima.list_instances() {
        Ok(instances) => instances,
        Err(err) => {
            eprintln!("warning: failed to inspect Lima instances: {err}");
            Vec::new()
        }
    };
    let sessions = reconcile_sessions_with_live_instances(&sessions, &live_instances);
    let sessions = visible_sessions(&sessions, args.all);
    let live_agents = live_agent_states(&lima, &sessions, &live_instances);
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&sessions)
                .map_err(crate::error::observability::ObservabilityError::Json)?
        );
    } else {
        if sessions.is_empty() && !args.all {
            println!("No active sessions. Use `agbranch ps -a` to show all sessions.");
        } else {
            let now = Timestamp::now_utc();
            let table = if std::io::stdout().is_terminal() {
                render_table_styled(&sessions, &live_instances, &live_agents, args.all, now)
            } else {
                render_table(&sessions, &live_instances, &live_agents, args.all, now)
            };
            println!("{table}");
        }
    }
    Ok(())
}

fn header_cell(value: &str, styled_headers: bool) -> Cell {
    let cell = Cell::new(value);
    if styled_headers {
        cell.add_attribute(Attribute::Bold)
    } else {
        cell
    }
}

fn format_optional_timestamp(value: Option<Timestamp>) -> String {
    value
        .map(|timestamp| timestamp.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn format_last_stopped_for_history(session: &PsSessionRow) -> String {
    if session.lifecycle_state == LifecycleState::Running {
        String::new()
    } else {
        format_optional_timestamp(session.last_stopped_at)
    }
}

fn live_agent_states(
    client: &dyn LimaClient,
    sessions: &[PsSessionRow],
    live_instances: &[LimaInstance],
) -> BTreeMap<String, String> {
    let live_by_name = live_instances
        .iter()
        .filter(|instance| instance.status == LimaInstanceStatus::Running)
        .map(|instance| (instance.name.as_str(), instance))
        .collect::<BTreeMap<_, _>>();

    sessions
        .iter()
        .filter_map(|session| {
            let _ = live_by_name.get(session.vm_name.as_str())?;
            Some((
                session.name.to_string(),
                probe_guest_runtime(
                    client,
                    RuntimeProbeTarget {
                        session_name: session.name.as_str(),
                        vm_name: &session.vm_name,
                        provider_kind: session.provider_kind,
                        guest_tmux_socket_path: session.guest_tmux_socket_path.as_ref(),
                        shell_window_name: session.shell_window_name.as_deref(),
                        agent_window_name: session.agent_window_name.as_deref(),
                    },
                )
                .agent,
            ))
        })
        .collect()
}

fn format_running_for(session: &PsSessionRow, now: Timestamp) -> String {
    if matches!(
        session.lifecycle_state,
        LifecycleState::Stopped | LifecycleState::Closed
    ) {
        return "-".to_owned();
    }
    session
        .last_started_at
        .map(|started_at| {
            format_duration(now.as_offset_date_time() - started_at.as_offset_date_time())
        })
        .unwrap_or_else(|| "-".to_owned())
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.whole_seconds().max(0);
    if total_seconds < 60 {
        return format!("{total_seconds}s");
    }
    if total_seconds < 60 * 60 {
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        return format!("{minutes}m{seconds:02}s");
    }
    if total_seconds < 60 * 60 * 24 {
        let hours = total_seconds / (60 * 60);
        let minutes = (total_seconds % (60 * 60)) / 60;
        return format!("{hours}h{minutes:02}m");
    }
    let days = total_seconds / (60 * 60 * 24);
    let hours = (total_seconds % (60 * 60 * 24)) / (60 * 60);
    format!("{days}d{hours:02}h")
}

fn visible_sessions(sessions: &[PsSessionRow], include_all: bool) -> Vec<PsSessionRow> {
    sessions
        .iter()
        .filter(|session| include_all || is_active_session(session.lifecycle_state))
        .cloned()
        .collect()
}

fn reconcile_sessions_with_live_instances(
    sessions: &[PsSessionRow],
    live_instances: &[LimaInstance],
) -> Vec<PsSessionRow> {
    let live_by_name = live_instances
        .iter()
        .map(|instance| (instance.name.as_str(), instance))
        .collect::<BTreeMap<_, _>>();
    sessions
        .iter()
        .cloned()
        .map(|mut session| {
            if let Some(instance) = live_by_name.get(session.vm_name.as_str()) {
                session.lifecycle_state =
                    reconcile_lifecycle_state(session.lifecycle_state, instance.status);
            }
            session
        })
        .collect()
}

fn reconcile_lifecycle_state(
    catalog_state: LifecycleState,
    live_status: LimaInstanceStatus,
) -> LifecycleState {
    match catalog_state {
        LifecycleState::Closed => LifecycleState::Closed,
        LifecycleState::Error
        | LifecycleState::PreparingBase
        | LifecycleState::Cloning
        | LifecycleState::Starting
        | LifecycleState::Seeding
        | LifecycleState::Syncing
        | LifecycleState::Staging
        | LifecycleState::Applying
        | LifecycleState::Destroying
        | LifecycleState::Repairing => catalog_state,
        LifecycleState::Running | LifecycleState::Stopped => match live_status {
            LimaInstanceStatus::Running => LifecycleState::Running,
            LimaInstanceStatus::Stopped => LifecycleState::Stopped,
            LimaInstanceStatus::Other => catalog_state,
        },
    }
}

fn is_active_session(state: LifecycleState) -> bool {
    !matches!(state, LifecycleState::Closed | LifecycleState::Stopped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{LifecycleState, ProviderKind, RepoSyncMode, SessionMode, SyncState};
    use crate::error::lima::LimaError;
    use crate::lima::client::LimaClient;
    use crate::lima::inspect::{LimaConfig, LimaInstance, LimaInstanceStatus};
    use crate::session::runtime::{WindowState, infer_guest_runtime_from_panes};
    use crate::types::{DiskSize, GuestPath, HostPath, MemorySize, SessionName, Timestamp, VmName};
    use crate::util::process::CommandOutput;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    struct TimingFields<'a> {
        created_at: &'a str,
        last_started_at: Option<&'a str>,
        last_stopped_at: Option<&'a str>,
        closed_at: Option<&'a str>,
    }

    fn row(
        name: &str,
        vm_name: &str,
        lifecycle_state: LifecycleState,
        sync_state: SyncState,
        timing: TimingFields<'_>,
    ) -> PsSessionRow {
        PsSessionRow {
            name: SessionName::try_from(name).expect("session"),
            vm_name: VmName::new(vm_name),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new(PathBuf::from("/tmp/fixture"))),
            guest_workspace_path: GuestPath::new(PathBuf::from(
                "/home/tester.guest/workspaces/demo/repo",
            )),
            host_head_oid_at_open: None,
            host_head_ref_at_open: None,
            host_dirty_at_open: false,
            lifecycle_state,
            sync_state,
            created_at: Timestamp::parse_rfc3339(timing.created_at).expect("created_at"),
            last_started_at: timing
                .last_started_at
                .map(|value| Timestamp::parse_rfc3339(value).expect("last_started_at")),
            last_stopped_at: timing
                .last_stopped_at
                .map(|value| Timestamp::parse_rfc3339(value).expect("last_stopped_at")),
            closed_at: timing
                .closed_at
                .map(|value| Timestamp::parse_rfc3339(value).expect("closed_at")),
            provider_kind: Some(ProviderKind::Claude),
            guest_tmux_socket_path: Some(GuestPath::new(
                "/home/tester.guest/.agbranch/tmux/demo.sock",
            )),
            shell_window_name: Some("shell".to_owned()),
            agent_window_name: Some("agent".to_owned()),
        }
    }

    fn live_instance(name: &str, port: u16) -> LimaInstance {
        LimaInstance {
            name: name.to_owned(),
            instance_dir: format!("/Users/tester/.lima/{name}"),
            ssh_config_file: format!("/Users/tester/.lima/{name}/ssh.config"),
            vm_type: "vz".to_owned(),
            raw_status: "Running".to_owned(),
            status: LimaInstanceStatus::Running,
            arch: Some("aarch64".to_owned()),
            cpus: Some(4),
            memory: Some(4 * 1024 * 1024 * 1024),
            disk: Some(100 * 1024 * 1024 * 1024),
            ssh_local_port: Some(port),
            ssh_address: Some("127.0.0.1".to_owned()),
            config: LimaConfig::default(),
        }
    }

    #[test]
    fn ps_render_includes_running_duration_for_active_sessions() {
        let now = Timestamp::parse_rfc3339("2026-04-15T02:00:00Z").expect("now");
        let rendered = render_table(
            &[
                row(
                    "demo",
                    "agbranch-demo",
                    LifecycleState::Running,
                    SyncState::Pending,
                    TimingFields {
                        created_at: "2026-04-15T00:00:00Z",
                        last_started_at: Some("2026-04-15T00:54:55Z"),
                        last_stopped_at: None,
                        closed_at: None,
                    },
                ),
                row(
                    "manual-happy",
                    "agbranch-manual-happy",
                    LifecycleState::Closed,
                    SyncState::Discarded,
                    TimingFields {
                        created_at: "2026-04-15T00:00:00Z",
                        last_started_at: Some("2026-04-15T00:10:00Z"),
                        last_stopped_at: Some("2026-04-15T01:20:00Z"),
                        closed_at: Some("2026-04-15T01:20:00Z"),
                    },
                ),
            ],
            &[live_instance("agbranch-demo", 50738)],
            &BTreeMap::from([("demo".to_owned(), "claude".to_owned())]),
            false,
            now,
        );

        assert_eq!(
            rendered,
            concat!(
                "SESSION  VM             STATE    UP FOR  SYNC     AGENT   VM STATUS  SSH              VMTYPE  ARCH     CPUS  MEMORY  DISK\n",
                "demo     agbranch-demo  running  1h05m   pending  claude  Running    127.0.0.1:50738  vz      aarch64  4     4GiB    100GiB"
            )
        );
    }

    #[test]
    fn ps_render_includes_timeline_columns_for_all_sessions_view() {
        let now = Timestamp::parse_rfc3339("2026-04-15T02:00:00Z").expect("now");
        let rendered = render_table(
            &[
                row(
                    "demo",
                    "agbranch-demo",
                    LifecycleState::Running,
                    SyncState::Pending,
                    TimingFields {
                        created_at: "2026-04-15T00:00:00Z",
                        last_started_at: Some("2026-04-15T00:54:55Z"),
                        last_stopped_at: Some("2026-04-15T00:50:00Z"),
                        closed_at: None,
                    },
                ),
                row(
                    "manual-happy",
                    "agbranch-manual-happy",
                    LifecycleState::Closed,
                    SyncState::Discarded,
                    TimingFields {
                        created_at: "2026-04-15T00:00:00Z",
                        last_started_at: Some("2026-04-15T00:10:00Z"),
                        last_stopped_at: Some("2026-04-15T01:20:00Z"),
                        closed_at: Some("2026-04-15T01:20:00Z"),
                    },
                ),
            ],
            &[],
            &BTreeMap::new(),
            true,
            now,
        );

        assert!(rendered.contains("CREATED"));
        assert!(rendered.contains("LAST STARTED"));
        assert!(rendered.contains("LAST STOPPED"));
        assert!(rendered.contains("2026-04-15T01:20:00Z"));
        assert!(
            !rendered.contains("2026-04-15T00:50:00Z"),
            "running sessions should not show a historical last stop timestamp: {rendered}"
        );
    }

    #[test]
    fn ps_render_styled_preserves_header_emphasis_for_tty_output() {
        let now = Timestamp::parse_rfc3339("2026-04-15T02:00:00Z").expect("now");
        let rendered = render_table_styled(
            &[row(
                "demo",
                "agbranch-demo",
                LifecycleState::Running,
                SyncState::Pending,
                TimingFields {
                    created_at: "2026-04-15T00:00:00Z",
                    last_started_at: Some("2026-04-15T00:54:55Z"),
                    last_stopped_at: None,
                    closed_at: None,
                },
            )],
            &[live_instance("agbranch-demo", 50738)],
            &BTreeMap::from([("demo".to_owned(), "claude".to_owned())]),
            false,
            now,
        );

        assert!(rendered.contains("\u{1b}[1mSESSION"));
    }

    struct StubRunner {
        stdout: String,
    }

    impl LimaClient for StubRunner {
        fn list_instances(&self) -> Result<Vec<LimaInstance>, LimaError> {
            unreachable!("list_instances is not used by ps unit tests");
        }

        fn clone_instance(
            &self,
            _source: &VmName,
            _target: &VmName,
            _cpus: Option<u16>,
            _memory: Option<&MemorySize>,
            _disk: Option<&DiskSize>,
        ) -> Result<(), LimaError> {
            unreachable!("clone_instance is not used by ps unit tests");
        }

        fn start_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("start_instance is not used by ps unit tests");
        }

        fn stop_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("stop_instance is not used by ps unit tests");
        }

        fn delete_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("delete_instance is not used by ps unit tests");
        }

        fn bash(&self, _vm: &VmName, _command: &str) -> Result<CommandOutput, LimaError> {
            Ok(CommandOutput {
                stdout: self.stdout.clone(),
                stderr: String::new(),
            })
        }

        fn copy_host_path_to_guest(
            &self,
            _host_path: &HostPath,
            _instance_name: &VmName,
            _guest_path: &GuestPath,
        ) -> Result<(), LimaError> {
            unreachable!("copy_host_path_to_guest is not used by ps unit tests");
        }

        fn seed_repo(
            &self,
            _filtered_seed_root: &HostPath,
            _instance_name: &VmName,
            _guest_repo: &GuestPath,
        ) -> Result<(), LimaError> {
            unreachable!("seed_repo is not used by ps unit tests");
        }

        fn copy_host_file_to_guest(
            &self,
            _host_file: &HostPath,
            _instance_name: &VmName,
            _guest_file: &GuestPath,
        ) -> Result<(), LimaError> {
            unreachable!("copy_host_file_to_guest is not used by ps unit tests");
        }

        fn copy_guest_secret_file(
            &self,
            _host_secret_file: &HostPath,
            _instance_name: &VmName,
            _guest_secret_file: &GuestPath,
        ) -> Result<(), LimaError> {
            unreachable!("copy_guest_secret_file is not used by ps unit tests");
        }
    }

    #[test]
    fn stopped_live_vm_reconciles_running_session_to_stopped() {
        let created = Timestamp::parse_rfc3339("2026-04-20T00:00:00Z").expect("timestamp");
        let session = PsSessionRow {
            name: SessionName::try_from("demo").expect("session"),
            vm_name: VmName::new("agbranch-demo"),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new("/repo/demo")),
            guest_workspace_path: GuestPath::new("/guest/demo"),
            host_head_oid_at_open: None,
            host_head_ref_at_open: None,
            host_dirty_at_open: false,
            lifecycle_state: LifecycleState::Running,
            sync_state: SyncState::Pending,
            created_at: created,
            last_started_at: Some(created),
            last_stopped_at: None,
            closed_at: None,
            provider_kind: Some(ProviderKind::Claude),
            guest_tmux_socket_path: Some(GuestPath::new("/home/demo/.agbranch/tmux/demo.sock")),
            shell_window_name: Some("shell".to_owned()),
            agent_window_name: Some("agent".to_owned()),
        };
        let live = LimaInstance {
            name: "agbranch-demo".to_owned(),
            instance_dir: "~/.lima/agbranch-demo".to_owned(),
            ssh_config_file: "~/.lima/agbranch-demo/ssh.config".to_owned(),
            vm_type: "vz".to_owned(),
            raw_status: "Stopped".to_owned(),
            arch: Some("aarch64".to_owned()),
            cpus: Some(4),
            memory: Some(4 * 1024 * 1024 * 1024),
            disk: Some(100 * 1024 * 1024 * 1024),
            ssh_local_port: Some(60022),
            ssh_address: Some("127.0.0.1".to_owned()),
            config: LimaConfig::default(),
            status: LimaInstanceStatus::Stopped,
        };

        let reconciled = reconcile_sessions_with_live_instances(&[session], &[live]);
        assert_eq!(reconciled[0].lifecycle_state, LifecycleState::Stopped);
        assert!(!is_active_session(reconciled[0].lifecycle_state));
    }

    #[test]
    fn running_live_vm_keeps_running_session_active() {
        let created = Timestamp::parse_rfc3339("2026-04-20T00:00:00Z").expect("timestamp");
        let session = PsSessionRow {
            name: SessionName::try_from("demo").expect("session"),
            vm_name: VmName::new("agbranch-demo"),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new("/repo/demo")),
            guest_workspace_path: GuestPath::new("/guest/demo"),
            host_head_oid_at_open: None,
            host_head_ref_at_open: None,
            host_dirty_at_open: false,
            lifecycle_state: LifecycleState::Running,
            sync_state: SyncState::Pending,
            created_at: created,
            last_started_at: Some(created),
            last_stopped_at: None,
            closed_at: None,
            provider_kind: Some(ProviderKind::Claude),
            guest_tmux_socket_path: Some(GuestPath::new("/home/demo/.agbranch/tmux/demo.sock")),
            shell_window_name: Some("shell".to_owned()),
            agent_window_name: Some("agent".to_owned()),
        };
        let live = LimaInstance {
            name: "agbranch-demo".to_owned(),
            instance_dir: "~/.lima/agbranch-demo".to_owned(),
            ssh_config_file: "~/.lima/agbranch-demo/ssh.config".to_owned(),
            vm_type: "vz".to_owned(),
            raw_status: "Running".to_owned(),
            arch: Some("aarch64".to_owned()),
            cpus: Some(4),
            memory: Some(4 * 1024 * 1024 * 1024),
            disk: Some(100 * 1024 * 1024 * 1024),
            ssh_local_port: Some(60022),
            ssh_address: Some("127.0.0.1".to_owned()),
            config: LimaConfig::default(),
            status: LimaInstanceStatus::Running,
        };

        let reconciled = reconcile_sessions_with_live_instances(&[session], &[live]);
        assert_eq!(reconciled[0].lifecycle_state, LifecycleState::Running);
        assert!(is_active_session(reconciled[0].lifecycle_state));
    }

    #[test]
    fn running_live_vm_reconciles_stopped_catalog_session_to_running() {
        assert_eq!(
            reconcile_lifecycle_state(LifecycleState::Stopped, LimaInstanceStatus::Running),
            LifecycleState::Running
        );
    }

    #[test]
    fn closed_catalog_session_stays_closed_even_if_vm_is_stopped() {
        assert_eq!(
            reconcile_lifecycle_state(LifecycleState::Closed, LimaInstanceStatus::Stopped),
            LifecycleState::Closed
        );
    }

    #[test]
    fn workflow_state_is_not_overridden_by_lima_runtime() {
        assert_eq!(
            reconcile_lifecycle_state(LifecycleState::Syncing, LimaInstanceStatus::Stopped),
            LifecycleState::Syncing
        );
    }

    #[test]
    fn infer_agent_state_prefers_live_agent_window() {
        let output = "shell\t0\nagent\t0\n";
        assert_eq!(
            infer_guest_runtime_from_panes(output, Some("claude"), "shell", "agent").agent,
            "claude"
        );
    }

    #[test]
    fn infer_agent_state_reports_shell_only_without_live_agent_window() {
        let output = "shell\t0\nagent\t1\n";
        let runtime = infer_guest_runtime_from_panes(output, Some("claude"), "shell", "agent");
        assert_eq!(runtime.agent, "shell-only");
        assert_eq!(runtime.agent_window_state, WindowState::Dead);
    }

    #[test]
    fn infer_agent_state_supports_literal_backslash_t_output() {
        let output = "agent\\t0\n";
        let runtime = infer_guest_runtime_from_panes(output, Some("codex"), "shell", "agent");
        assert_eq!(runtime.agent, "codex");
        assert_eq!(runtime.agent_window_state, WindowState::Live);
    }

    #[test]
    fn live_agent_states_reports_shell_only_from_tmux_probe() {
        let created = Timestamp::parse_rfc3339("2026-04-20T00:00:00Z").expect("timestamp");
        let session = PsSessionRow {
            name: SessionName::try_from("demo").expect("session"),
            vm_name: VmName::new("agbranch-demo"),
            session_mode: SessionMode::Repo,
            repo_sync_mode: Some(RepoSyncMode::GitNative),
            host_context_path: Some(HostPath::new("/repo/demo")),
            guest_workspace_path: GuestPath::new("/guest/demo"),
            host_head_oid_at_open: None,
            host_head_ref_at_open: None,
            host_dirty_at_open: false,
            lifecycle_state: LifecycleState::Running,
            sync_state: SyncState::Pending,
            created_at: created,
            last_started_at: Some(created),
            last_stopped_at: None,
            closed_at: None,
            provider_kind: Some(ProviderKind::Claude),
            guest_tmux_socket_path: Some(GuestPath::new("/home/demo/.agbranch/tmux/demo.sock")),
            shell_window_name: Some("shell".to_owned()),
            agent_window_name: Some("agent".to_owned()),
        };
        let live = LimaInstance {
            name: "agbranch-demo".to_owned(),
            instance_dir: "~/.lima/agbranch-demo".to_owned(),
            ssh_config_file: "~/.lima/agbranch-demo/ssh.config".to_owned(),
            vm_type: "vz".to_owned(),
            raw_status: "Running".to_owned(),
            arch: Some("aarch64".to_owned()),
            cpus: Some(4),
            memory: Some(4 * 1024 * 1024 * 1024),
            disk: Some(100 * 1024 * 1024 * 1024),
            ssh_local_port: Some(60022),
            ssh_address: Some("127.0.0.1".to_owned()),
            config: LimaConfig::default(),
            status: LimaInstanceStatus::Running,
        };

        let statuses = live_agent_states(
            &StubRunner {
                stdout: "shell\t0\nagent\t1\n".to_owned(),
            },
            &[session],
            &[live],
        );

        assert_eq!(statuses.get("demo").map(String::as_str), Some("shell-only"));
    }
}
