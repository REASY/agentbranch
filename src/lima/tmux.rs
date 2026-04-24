use crate::types::GuestPath;
use std::path::Path;

fn socket_path(socket: &GuestPath) -> String {
    crate::lima::shell::shell_escape(&socket.to_string())
}

fn socket_parent(socket: &GuestPath) -> String {
    let parent = socket
        .as_path()
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"));
    crate::lima::shell::shell_escape(&parent.display().to_string())
}

pub fn ensure_shell_window_commands(
    socket: &GuestPath,
    tmux_session: &str,
    shell_window: &str,
    session_name: &str,
    workspace: &str,
) -> Vec<String> {
    let socket_parent = socket_parent(socket);
    let socket = socket_path(socket);
    let shell_window = crate::lima::shell::shell_escape(shell_window);
    let bootstrap =
        crate::provider::launch::build_session_shell_line(session_name, &GuestPath::new(workspace));
    vec![
        format!("mkdir -p {socket_parent}",),
        format!(
            "tmux -S {socket} has-session -t {tmux_session} || tmux -S {socket} new-session -d -s {tmux_session} -n {shell_window} -c {workspace} {}",
            crate::lima::shell::shell_escape(&bootstrap),
        ),
        format!(
            "tmux -S {socket} list-windows -F '#W' -t {tmux_session} | grep -Fx {shell_window} >/dev/null || tmux -S {socket} new-window -t {tmux_session} -n {shell_window} -c {workspace} {}",
            crate::lima::shell::shell_escape(&bootstrap),
        ),
        format!(
            "tmux -S {socket} set-window-option -t {tmux_session}:{shell_window} automatic-rename off >/dev/null 2>&1 || true"
        ),
        format!(
            "tmux -S {socket} set-window-option -t {tmux_session}:{shell_window} remain-on-exit on >/dev/null 2>&1 || true"
        ),
        format!(
            "tmux -S {socket} set-option -t {tmux_session} status-right 'Detach: Ctrl-b d' >/dev/null 2>&1 || true"
        ),
    ]
}

pub fn attach_shell_command(
    socket: &GuestPath,
    tmux_session: &str,
    shell_window: &str,
    session_name: &str,
    workspace: &GuestPath,
) -> String {
    let socket = socket_path(socket);
    let window = crate::lima::shell::shell_escape(shell_window);
    let bootstrap = crate::provider::launch::build_session_shell_line(session_name, workspace);
    let bootstrap_escaped = crate::lima::shell::shell_escape(&bootstrap);
    format!(
        "if [ \"$(tmux -S {socket} display-message -p -t {tmux_session}:{window} -F '#{{pane_dead}}' 2>/dev/null)\" = \"1\" ]; then tmux -S {socket} respawn-pane -k -t {tmux_session}:{window} {bootstrap_escaped} >/dev/null; fi; tmux -S {socket} select-window -t {tmux_session}:{window} >/dev/null 2>&1 && tmux -S {socket} attach-session -t {tmux_session}"
    )
}

pub fn attach_window_command(socket: &GuestPath, tmux_session: &str, window: &str) -> String {
    let socket = socket_path(socket);
    format!(
        "tmux -S {socket} select-window -t {tmux_session}:{window} >/dev/null 2>&1 && tmux -S {socket} attach-session -t {tmux_session}"
    )
}

pub fn agent_window_launch_commands(
    socket: &GuestPath,
    tmux_session: &str,
    window: &str,
    workspace: &str,
    launch_line: &str,
) -> Vec<String> {
    let socket = socket_path(socket);
    vec![
        format!("tmux -S {socket} kill-window -t {tmux_session}:{window} 2>/dev/null || true"),
        format!("tmux -S {socket} new-window -t {tmux_session} -n {window} -c {workspace}"),
        format!(
            "tmux -S {socket} set-window-option -t {tmux_session}:{window} automatic-rename off >/dev/null 2>&1 || true"
        ),
        format!(
            "tmux -S {socket} set-window-option -t {tmux_session}:{window} remain-on-exit failed >/dev/null 2>&1 || true"
        ),
        format!(
            "tmux -S {socket} send-keys -t {tmux_session}:{window} -l {}",
            crate::lima::shell::shell_escape(launch_line)
        ),
        format!("tmux -S {socket} send-keys -t {tmux_session}:{window} Enter"),
    ]
}

pub fn send_ctrl_c_command(socket: &GuestPath, tmux_session: &str, window: &str) -> String {
    let socket = socket_path(socket);
    format!("tmux -S {socket} send-keys -t {tmux_session}:{window} C-c")
}

pub fn kill_window_command(socket: &GuestPath, tmux_session: &str, window: &str) -> String {
    let socket = socket_path(socket);
    format!("tmux -S {socket} kill-window -t {tmux_session}:{window}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_shell_window_is_created_before_attach() {
        let socket = GuestPath::new("/home/tester.guest/.agbranch/tmux/session.sock");
        let commands = ensure_shell_window_commands(
            &socket,
            "session",
            "shell",
            "demo",
            "/home/tester.guest/sandbox/demo",
        );
        assert_eq!(
            commands,
            vec![
                "mkdir -p '/home/tester.guest/.agbranch/tmux'",
                "tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' has-session -t session || tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' new-session -d -s session -n 'shell' -c /home/tester.guest/sandbox/demo 'export AGBRANCH_SESSION='\"'\"'demo'\"'\"' && source \"$HOME/.agbranch/shellenv.sh\" && cd '\"'\"'/home/tester.guest/sandbox/demo'\"'\"' && exec ${SHELL:-/bin/bash} -l'",
                "tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' list-windows -F '#W' -t session | grep -Fx 'shell' >/dev/null || tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' new-window -t session -n 'shell' -c /home/tester.guest/sandbox/demo 'export AGBRANCH_SESSION='\"'\"'demo'\"'\"' && source \"$HOME/.agbranch/shellenv.sh\" && cd '\"'\"'/home/tester.guest/sandbox/demo'\"'\"' && exec ${SHELL:-/bin/bash} -l'",
                "tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' set-window-option -t session:'shell' automatic-rename off >/dev/null 2>&1 || true",
                "tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' set-window-option -t session:'shell' remain-on-exit on >/dev/null 2>&1 || true",
                "tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' set-option -t session status-right 'Detach: Ctrl-b d' >/dev/null 2>&1 || true",
            ]
        );
        assert_eq!(
            attach_shell_command(
                &socket,
                "session",
                "shell",
                "demo",
                &GuestPath::new("/home/tester.guest/sandbox/demo"),
            ),
            "if [ \"$(tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' display-message -p -t session:'shell' -F '#{pane_dead}' 2>/dev/null)\" = \"1\" ]; then tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' respawn-pane -k -t session:'shell' 'export AGBRANCH_SESSION='\"'\"'demo'\"'\"' && source \"$HOME/.agbranch/shellenv.sh\" && cd '\"'\"'/home/tester.guest/sandbox/demo'\"'\"' && exec ${SHELL:-/bin/bash} -l' >/dev/null; fi; tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' select-window -t session:'shell' >/dev/null 2>&1 && tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' attach-session -t session"
        );
    }

    #[test]
    fn tmux_agent_window_is_recreated_and_bootstrapped_before_attach() {
        let socket = GuestPath::new("/home/tester.guest/.agbranch/tmux/session.sock");
        let commands = agent_window_launch_commands(
            &socket,
            "session",
            "agent",
            "/home/tester.guest/sandbox/demo",
            "export AGBRANCH_SESSION='demo' && source \"$HOME/.agbranch/shellenv.sh\" && cd '/home/tester.guest/sandbox/demo' && exec codex",
        );
        assert_eq!(
            commands,
            vec![
                "tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' kill-window -t session:agent 2>/dev/null || true",
                "tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' new-window -t session -n agent -c /home/tester.guest/sandbox/demo",
                "tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' set-window-option -t session:agent automatic-rename off >/dev/null 2>&1 || true",
                "tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' set-window-option -t session:agent remain-on-exit failed >/dev/null 2>&1 || true",
                "tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' send-keys -t session:agent -l 'export AGBRANCH_SESSION='\"'\"'demo'\"'\"' && source \"$HOME/.agbranch/shellenv.sh\" && cd '\"'\"'/home/tester.guest/sandbox/demo'\"'\"' && exec codex'",
                "tmux -S '/home/tester.guest/.agbranch/tmux/session.sock' send-keys -t session:agent Enter",
            ]
        );
    }
}
