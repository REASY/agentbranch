use crate::lima::client::LimaClient;
use crate::lima::inspect::{LimaInstance, LimaInstanceStatus};
use crate::lima::shell::shell_escape;
use crate::types::{GuestPath, ProviderKind, VmName};
use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub struct RuntimeProbeTarget<'a> {
    pub session_name: &'a str,
    pub vm_name: &'a VmName,
    pub provider_kind: Option<ProviderKind>,
    pub guest_tmux_socket_path: Option<&'a GuestPath>,
    pub shell_window_name: Option<&'a str>,
    pub agent_window_name: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowState {
    Live,
    Dead,
    Missing,
    Unknown,
}

impl WindowState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Dead => "dead",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuestRuntimeProbe {
    pub agent: String,
    pub shell_window_name: String,
    pub shell_window_state: WindowState,
    pub agent_window_name: String,
    pub agent_window_state: WindowState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveVmSummary {
    pub status: String,
    pub ssh: Option<String>,
    pub vm_type: String,
    pub arch: Option<String>,
    pub cpus: Option<u16>,
    pub memory: Option<String>,
    pub disk: Option<String>,
}

pub fn probe_guest_runtime(
    client: &dyn LimaClient,
    target: RuntimeProbeTarget<'_>,
) -> GuestRuntimeProbe {
    let shell_window_name = target.shell_window_name.unwrap_or("shell").to_owned();
    let agent_window_name = target.agent_window_name.unwrap_or("agent").to_owned();
    let socket = match target.guest_tmux_socket_path {
        Some(socket) => socket,
        None => {
            return GuestRuntimeProbe {
                agent: "unknown".to_owned(),
                shell_window_name,
                shell_window_state: WindowState::Unknown,
                agent_window_name,
                agent_window_state: WindowState::Unknown,
            };
        }
    };

    let command = format!(
        "tmux -S {} list-panes -t {} -F '#{{window_name}}|#{{pane_dead}}|#{{@agbranch_provider}}'",
        shell_escape(&socket.to_string()),
        shell_escape(target.session_name),
    );
    let output = match client.bash(target.vm_name, &command) {
        Ok(output) => output.stdout,
        Err(_) => {
            return GuestRuntimeProbe {
                agent: "unknown".to_owned(),
                shell_window_name,
                shell_window_state: WindowState::Unknown,
                agent_window_name,
                agent_window_state: WindowState::Unknown,
            };
        }
    };

    infer_guest_runtime_from_panes(
        &output,
        target.provider_kind.map(|kind| kind.as_str()),
        &shell_window_name,
        &agent_window_name,
    )
}

pub fn infer_guest_runtime_from_panes(
    panes_output: &str,
    provider_label: Option<&str>,
    shell_window_name: &str,
    agent_window_name: &str,
) -> GuestRuntimeProbe {
    let mut shell_state = WindowState::Missing;
    let mut agent_state = WindowState::Missing;
    let mut live_provider = None;

    for line in panes_output.lines() {
        let Some((window, pane_dead, window_provider)) = split_probe_line(line) else {
            continue;
        };
        let state = if pane_dead == "0" {
            WindowState::Live
        } else {
            WindowState::Dead
        };
        if state == WindowState::Live && !window_provider.is_empty() {
            live_provider = Some(window_provider.to_owned());
        }
        if window == shell_window_name {
            shell_state = merge_window_state(shell_state, state);
        }
        if window == agent_window_name {
            agent_state = merge_window_state(agent_state, state);
        }
    }

    let agent = match live_provider {
        Some(provider) => provider,
        None => match agent_state {
            WindowState::Live => provider_label.unwrap_or("agent").to_owned(),
            _ if shell_state == WindowState::Live => "shell-only".to_owned(),
            _ => "unknown".to_owned(),
        },
    };

    GuestRuntimeProbe {
        agent,
        shell_window_name: shell_window_name.to_owned(),
        shell_window_state: shell_state,
        agent_window_name: agent_window_name.to_owned(),
        agent_window_state: agent_state,
    }
}

pub fn format_ssh_endpoint(instance: &LimaInstance) -> Option<String> {
    let address = instance.ssh_address.as_deref()?;
    let port = instance.ssh_local_port?;
    Some(format!("{address}:{port}"))
}

pub fn format_bytes(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    if bytes.is_multiple_of(GIB) {
        format!("{}GiB", bytes / GIB)
    } else {
        format!("{:.1}GiB", bytes as f64 / GIB as f64)
    }
}

pub fn summarize_live_vm(instance: Option<&LimaInstance>) -> Option<LiveVmSummary> {
    instance.map(|item| LiveVmSummary {
        status: match item.status {
            LimaInstanceStatus::Running => "running".to_owned(),
            LimaInstanceStatus::Stopped => "stopped".to_owned(),
            LimaInstanceStatus::Other => item.raw_status.to_lowercase(),
        },
        ssh: format_ssh_endpoint(item),
        vm_type: item.vm_type.clone(),
        arch: item.arch.clone(),
        cpus: item.cpus,
        memory: item.memory.map(format_bytes),
        disk: item.disk.map(format_bytes),
    })
}

fn merge_window_state(current: WindowState, candidate: WindowState) -> WindowState {
    match (current, candidate) {
        (WindowState::Live, _) | (_, WindowState::Live) => WindowState::Live,
        (WindowState::Dead, _) | (_, WindowState::Dead) => WindowState::Dead,
        (WindowState::Unknown, _) | (_, WindowState::Unknown) => WindowState::Unknown,
        _ => WindowState::Missing,
    }
}

fn split_probe_line(line: &str) -> Option<(&str, &str, &str)> {
    if let Some((window, rest)) = line.split_once('|') {
        if let Some((pane_dead, provider)) = rest.split_once('|') {
            return Some((window, pane_dead, provider));
        }
        return Some((window, rest, ""));
    }
    if let Some((window, pane_dead)) = line.split_once('\t') {
        return Some((window, pane_dead, ""));
    }
    line.split_once("\\t")
        .map(|(window, pane_dead)| (window, pane_dead, ""))
}

#[cfg(test)]
mod tests {
    use super::{WindowState, infer_guest_runtime_from_panes};

    #[test]
    fn infer_guest_runtime_supports_literal_backslash_t_output() {
        let probe = infer_guest_runtime_from_panes("agent\\t0\n", Some("codex"), "shell", "agent");
        assert_eq!(probe.agent, "codex");
        assert_eq!(probe.agent_window_state, WindowState::Live);
        assert_eq!(probe.shell_window_state, WindowState::Missing);
    }

    #[test]
    fn infer_guest_runtime_prefers_live_window_provider_marker() {
        let probe =
            infer_guest_runtime_from_panes("shell|0|claude\n", Some("codex"), "shell", "agent");
        assert_eq!(probe.agent, "claude");
        assert_eq!(probe.shell_window_state, WindowState::Live);
        assert_eq!(probe.agent_window_state, WindowState::Missing);
    }

    #[test]
    fn infer_guest_runtime_reports_shell_only_when_agent_not_live() {
        let probe =
            infer_guest_runtime_from_panes("shell|0\nagent|1\n", Some("claude"), "shell", "agent");
        assert_eq!(probe.agent, "shell-only");
        assert_eq!(probe.shell_window_state, WindowState::Live);
        assert_eq!(probe.agent_window_state, WindowState::Dead);
    }
}
