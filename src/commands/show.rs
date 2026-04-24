use crate::cli::SessionArgs;
use crate::commands::{find_existing_session, resolve_session_name};
use crate::db::connect::open_catalog;
use crate::db::sessions::SessionRow;
use crate::error::AppError;
use crate::lima::client::{LimaClient, LimactlClient};
use crate::lima::inspect::{LimaInstance, LimaInstanceStatus};
use crate::platform::host::HostContext;
use crate::session::runtime::{
    GuestRuntimeProbe, RuntimeProbeTarget, probe_guest_runtime, summarize_live_vm,
};
use crate::util::process::RealCommandRunner;
use serde_json::json;
use std::collections::BTreeMap;

pub fn render_show_json(value: serde_json::Value) -> Result<String, serde_json::Error> {
    serde_json::to_string(&value)
}

pub fn run(args: SessionArgs) -> Result<(), AppError> {
    let (session_name_raw, session_name) = resolve_session_name(&args.session)?;
    let host = HostContext::detect()?;
    let conn = open_catalog(&host.state_roots.db)?;
    let session = find_existing_session(&conn, &session_name, &session_name_raw)?;

    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    let live_instances = match lima.list_instances() {
        Ok(instances) => instances,
        Err(err) => {
            eprintln!("warning: failed to inspect Lima instances: {err}");
            Vec::new()
        }
    };
    let live_by_name = live_instances
        .iter()
        .map(|instance| (instance.name.as_str(), instance))
        .collect::<BTreeMap<_, _>>();
    let live_vm = live_by_name.get(session.vm_name.as_str()).copied();
    let guest_runtime =
        if live_vm.is_some_and(|instance| instance.status == LimaInstanceStatus::Running) {
            probe_guest_runtime(
                &lima,
                RuntimeProbeTarget {
                    session_name: session.name.as_str(),
                    vm_name: &session.vm_name,
                    provider_kind: session.provider_kind,
                    guest_tmux_socket_path: session.guest_tmux_socket_path.as_ref(),
                    shell_window_name: session.shell_window_name.as_deref(),
                    agent_window_name: session.agent_window_name.as_deref(),
                },
            )
        } else {
            GuestRuntimeProbe {
                agent: "unknown".to_owned(),
                shell_window_name: session
                    .shell_window_name
                    .clone()
                    .unwrap_or_else(|| "shell".to_owned()),
                shell_window_state: crate::session::runtime::WindowState::Unknown,
                agent_window_name: session
                    .agent_window_name
                    .clone()
                    .unwrap_or_else(|| "agent".to_owned()),
                agent_window_state: crate::session::runtime::WindowState::Unknown,
            }
        };

    if args.json {
        println!(
            "{}",
            render_show_json(render_show_json_value(&session, live_vm, &guest_runtime))
                .map_err(crate::error::observability::ObservabilityError::Json)?
        );
    } else {
        println!("{}", render_show_human(&session, live_vm, &guest_runtime));
    }
    Ok(())
}

fn render_show_json_value(
    session: &SessionRow,
    live_vm: Option<&LimaInstance>,
    guest_runtime: &GuestRuntimeProbe,
) -> serde_json::Value {
    let imported_provider_files: serde_json::Value =
        serde_json::from_str(&session.imported_provider_files_json)
            .unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));

    json!({
        "name": session.name,
        "vm_name": session.vm_name,
        "session_mode": session.session_mode.as_str(),
        "repo_sync_mode": session.repo_sync_mode.map(|mode| mode.as_str()),
        "lifecycle_state": session.lifecycle_state.as_str(),
        "sync_state": session.sync_state.as_str(),
        "host_context_path": session.host_context_path,
        "guest_workspace_path": session.guest_workspace_path,
        "review_branch": session.review_branch,
        "base_ref": session.base_ref,
        "session_ref_base": session.session_ref_base,
        "session_ref_head": session.session_ref_head,
        "imported_provider_files": imported_provider_files,
        "tmux": {
            "socket": session.guest_tmux_socket_path,
            "shell_window": session.shell_window_name,
            "agent_window": session.agent_window_name,
        },
        "agent": {
            "provider": session.provider_kind.map(|kind| kind.as_str()),
            "preset": session.agent_launch_preset.map(|preset| preset.as_str()),
            "window": session.agent_window_name,
        },
        "runtime": {
            "vm": summarize_live_vm(live_vm),
            "guest": {
                "agent": guest_runtime.agent,
                "shell_window": {
                    "name": guest_runtime.shell_window_name,
                    "state": guest_runtime.shell_window_state.as_str(),
                },
                "agent_window": {
                    "name": guest_runtime.agent_window_name,
                    "state": guest_runtime.agent_window_state.as_str(),
                }
            }
        }
    })
}

fn render_show_human(
    session: &SessionRow,
    live_vm: Option<&LimaInstance>,
    guest_runtime: &GuestRuntimeProbe,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Session: {}", session.name));
    lines.push(format!("VM: {}", session.vm_name));
    lines.push(format!(
        "Mode: {}{}",
        session.session_mode.as_str(),
        session
            .repo_sync_mode
            .map(|_| " (git-native)".to_owned())
            .unwrap_or_default()
    ));
    lines.push(format!("Lifecycle: {}", session.lifecycle_state.as_str()));
    lines.push(format!("Sync: {}", session.sync_state.as_str()));
    lines.push(String::new());

    if let Some(host_context_path) = &session.host_context_path {
        lines.push(format!("Host context: {}", host_context_path));
    }
    lines.push(format!("Guest workspace: {}", session.guest_workspace_path));
    if let Some(base_ref) = &session.base_ref {
        lines.push(format!("Base ref: {base_ref}"));
    }
    if let Some(review_branch) = &session.review_branch {
        lines.push(format!("Review branch: {review_branch}"));
    }
    if let Some(session_ref_base) = &session.session_ref_base {
        lines.push(format!("Session ref base: {session_ref_base}"));
    }
    if let Some(session_ref_head) = &session.session_ref_head {
        lines.push(format!("Session ref head: {session_ref_head}"));
    }
    lines.push(String::new());

    if let Some(vm) = summarize_live_vm(live_vm) {
        lines.push(format!("VM status: {}", live_vm.unwrap().raw_status));
        if let Some(ssh) = vm.ssh {
            lines.push(format!("SSH: {ssh}"));
        }
        lines.push(format!(
            "Platform: {} / {} / {} CPU / {} / {}",
            vm.vm_type,
            vm.arch.unwrap_or_else(|| "-".to_owned()),
            vm.cpus
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            vm.memory.unwrap_or_else(|| "-".to_owned()),
            vm.disk.unwrap_or_else(|| "-".to_owned())
        ));
    } else {
        lines.push("VM status: unavailable".to_owned());
    }
    lines.push(format!("Runtime: {}", guest_runtime.agent));
    if let Some(socket) = &session.guest_tmux_socket_path {
        lines.push(format!("tmux socket: {socket}"));
    }
    lines.push(format!(
        "Shell window: {} ({})",
        guest_runtime.shell_window_name,
        guest_runtime.shell_window_state.as_str()
    ));
    lines.push(format!(
        "Agent window: {} ({})",
        guest_runtime.agent_window_name,
        guest_runtime.agent_window_state.as_str()
    ));

    let imported_provider_files: serde_json::Value =
        serde_json::from_str(&session.imported_provider_files_json)
            .unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));
    if let Some(files) = imported_provider_files.as_array()
        && !files.is_empty()
    {
        lines.push(String::new());
        lines.push("Imported provider files:".to_owned());
        for entry in files {
            let host = entry
                .get("host_path")
                .and_then(|value| value.as_str())
                .unwrap_or("?");
            let guest = entry
                .get("guest_path")
                .and_then(|value| value.as_str())
                .unwrap_or("?");
            lines.push(format!("- {host} -> {guest}"));
        }
    }

    lines.join("\n")
}
