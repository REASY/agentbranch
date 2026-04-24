#!/usr/bin/env bash
# shellcheck disable=SC2154

detect_prepared_base_name() {
  case "$(uname -s)" in
    Darwin)
      printf 'agbranch-base-macos\n'
      ;;
    Linux)
      printf 'agbranch-base-linux\n'
      ;;
    *)
      echo "unsupported host OS: $(uname -s)" >&2
      exit 1
      ;;
  esac
}

log() {
  [[ "${VERBOSE}" -eq 1 ]] || return 0
  printf '[smoke-e2e] %s\n' "$*" >&2
}

remember_session() {
  printf '%s\n' "$1" >> "${SESSION_TRACK_FILE}"
}

cleanup() {
  set +e
  mkdir -p "${ARTIFACT_DIR}"
  if [[ -f "${SESSION_TRACK_FILE}" ]]; then
    while IFS= read -r session; do
      [[ -n "${session}" ]] || continue
      "${AGBRANCH_BIN}" close --session "${session}" --discard --yes >/dev/null 2>&1 || true
    done < "${SESSION_TRACK_FILE}"
  fi
  rm -rf "${RUN_TEMP_ROOT}"
}

choose_timeout_bin() {
  if command -v timeout >/dev/null 2>&1; then
    TIMEOUT_BIN="$(command -v timeout)"
    return 0
  fi
  if command -v gtimeout >/dev/null 2>&1; then
    TIMEOUT_BIN="$(command -v gtimeout)"
    return 0
  fi
  echo "missing timeout or gtimeout" >&2
  exit 1
}

require_tool() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required tool: $1" >&2
    exit 1
  }
}

run_phase() {
  local name="$1"
  local limit="$2"
  local function_name="$3"
  local phase_script

  log "phase=${name} timeout=${limit} function=${function_name}"
  echo "== phase:${name} =="
  printf -v phase_script 'set -euo pipefail; source %q; source %q; source %q; source %q; %s' \
    "${SMOKE_LIB_DIR}/common.sh" \
    "${SMOKE_LIB_DIR}/fixtures.sh" \
    "${SMOKE_LIB_DIR}/phases/core_workflow.sh" \
    "${SMOKE_LIB_DIR}/phases/session_controls.sh" \
    "${function_name}"
  "${TIMEOUT_BIN}" "${limit}" bash -lc "${phase_script}"
}

assert_json() {
  local file="$1"
  local query="$2"
  jq -e "${query}" "${file}" >/dev/null
}

run_doctor_preflight() {
  local doctor_file="${ARTIFACT_DIR}/doctor.json"
  local doctor_status

  set +e
  "${AGBRANCH_BIN}" doctor --json > "${doctor_file}"
  doctor_status=$?
  set -e

  assert_json "${doctor_file}" '.state_root != null'
  if [[ "${doctor_status}" -eq 0 ]]; then
    assert_json "${doctor_file}" '.ok == true'
    return 0
  fi

  if jq -e '
    (.messages | length) > 0
    and all(.messages[]; startswith("orphaned Lima instances:"))
  ' "${doctor_file}" >/dev/null; then
    jq -r '.messages[] | "warning: " + .' "${doctor_file}" >&2
    return 0
  fi

  jq -r '.messages[]?' "${doctor_file}" >&2
  exit "${doctor_status}"
}

assert_prepared_base_exists() {
  local list_file="${ARTIFACT_DIR}/limactl-list.json"

  limactl list --json > "${list_file}"
  if ! jq -s -e --arg name "${PREPARED_BASE_NAME}" \
    'map(if type == "array" then .[] else . end) | .[] | select(.name == $name)' \
    "${list_file}" >/dev/null; then
    printf "missing prepared base: %s; run '%s prepare' before smoke-e2e\n" \
      "${PREPARED_BASE_NAME}" "${AGBRANCH_BIN}" >&2
    exit 1
  fi
}
