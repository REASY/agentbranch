#!/usr/bin/env bash
# shellcheck disable=SC2154

capture_ps_json() {
  local file="$1"
  "${AGBRANCH_BIN}" ps -a --json > "${file}"
}

capture_show_json() {
  local session="$1"
  local file="$2"
  "${AGBRANCH_BIN}" show --session "${session}" --json > "${file}"
}

assert_session_ps_state() {
  local file="$1"
  local session="$2"
  local state="$3"
  assert_json "${file}" 'any(.[]; .name == "'"${session}"'" and (.lifecycle_state | ascii_downcase) == "'"${state}"'")'
}

create_agent_window() {
  local session="$1"
  local tmux_socket="$2"
  local session_escaped
  local socket_escaped
  local target_escaped

  printf -v session_escaped '%q' "${session}"
  printf -v socket_escaped '%q' "${tmux_socket}"
  printf -v target_escaped '%q' "${session}:agent"
  "${AGBRANCH_BIN}" run --session "${session}" -- bash -lc \
    "tmux -S ${socket_escaped} kill-window -t ${target_escaped} 2>/dev/null || true; tmux -S ${socket_escaped} new-window -t ${session_escaped} -n agent -c \"\$PWD\" 'sleep 300'"
}

phase_session_controls() {
  local session="smk-${SHORT_RUN_ID}-ctrl"
  local repo
  local tmux_socket
  local guest_workspace

  repo="$(create_fixture_repo "${session}")"
  remember_session "${session}"

  "${AGBRANCH_BIN}" open --session "${session}" --repo "${repo}" --json > "${ARTIFACT_DIR}/session-controls-open.json"

  "${AGBRANCH_BIN}" ps --json > "${ARTIFACT_DIR}/session-controls-ps-running.json"
  assert_json "${ARTIFACT_DIR}/session-controls-ps-running.json" 'any(.[]; .name == "'"${session}"'" and (.lifecycle_state | ascii_downcase) == "running")'

  capture_ps_json "${ARTIFACT_DIR}/session-controls-ps-all-running.json"
  assert_session_ps_state "${ARTIFACT_DIR}/session-controls-ps-all-running.json" "${session}" "running"

  capture_show_json "${session}" "${ARTIFACT_DIR}/session-controls-show-running.json"
  assert_json "${ARTIFACT_DIR}/session-controls-show-running.json" '.name == "'"${session}"'" and .lifecycle_state == "running" and .guest_workspace_path != null and .tmux.socket != null and .runtime.vm.status == "running" and .runtime.guest.shell_window.state == "live"'
  tmux_socket="$(jq -r '.tmux.socket' "${ARTIFACT_DIR}/session-controls-show-running.json")"
  guest_workspace="$(jq -r '.guest_workspace_path' "${ARTIFACT_DIR}/session-controls-show-running.json")"
  [[ "${tmux_socket}" != "null" ]]
  [[ "${guest_workspace}" != "null" ]]

  "${AGBRANCH_BIN}" shell --session "${session}" --json > "${ARTIFACT_DIR}/session-controls-shell.json"
  assert_json "${ARTIFACT_DIR}/session-controls-shell.json" '.host_alias != null and .ssh_config_file != null and .workdir == "'"${guest_workspace}"'"'

  "${AGBRANCH_BIN}" ssh --session "${session}" --json > "${ARTIFACT_DIR}/session-controls-ssh.json"
  assert_json "${ARTIFACT_DIR}/session-controls-ssh.json" '.host_alias != null and .ssh_config_file != null'

  create_agent_window "${session}" "${tmux_socket}"
  capture_show_json "${session}" "${ARTIFACT_DIR}/session-controls-show-agent-live.json"
  assert_json "${ARTIFACT_DIR}/session-controls-show-agent-live.json" '.runtime.guest.agent_window.state == "live"'

  "${AGBRANCH_BIN}" kill --session "${session}" --json > "${ARTIFACT_DIR}/session-controls-kill.json"
  assert_json "${ARTIFACT_DIR}/session-controls-kill.json" '.session == "'"${session}"'" and .force == false'
  capture_show_json "${session}" "${ARTIFACT_DIR}/session-controls-show-agent-killed.json"
  assert_json "${ARTIFACT_DIR}/session-controls-show-agent-killed.json" '.runtime.guest.agent_window.state == "missing" and .runtime.guest.agent == "shell-only"'

  "${AGBRANCH_BIN}" stop --session "${session}"
  capture_ps_json "${ARTIFACT_DIR}/session-controls-ps-stopped.json"
  assert_session_ps_state "${ARTIFACT_DIR}/session-controls-ps-stopped.json" "${session}" "stopped"
  capture_show_json "${session}" "${ARTIFACT_DIR}/session-controls-show-stopped.json"
  assert_json "${ARTIFACT_DIR}/session-controls-show-stopped.json" '.lifecycle_state == "stopped" and .runtime.vm.status == "stopped"'

  "${AGBRANCH_BIN}" start --session "${session}"
  capture_ps_json "${ARTIFACT_DIR}/session-controls-ps-restarted.json"
  assert_session_ps_state "${ARTIFACT_DIR}/session-controls-ps-restarted.json" "${session}" "running"
  capture_show_json "${session}" "${ARTIFACT_DIR}/session-controls-show-restarted.json"
  assert_json "${ARTIFACT_DIR}/session-controls-show-restarted.json" '.lifecycle_state == "running" and .runtime.vm.status == "running"'

  "${AGBRANCH_BIN}" close --session "${session}" --discard --yes --json > "${ARTIFACT_DIR}/session-controls-close.json"
  assert_json "${ARTIFACT_DIR}/session-controls-close.json" '.session == "'"${session}"'" and .destroy_result == "destroyed"'
}
