#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SMOKE_LIB_DIR="${ROOT_DIR}/scripts/smoke-lib"
AGBRANCH_BIN="${ROOT_DIR}/target/debug/agbranch"
VERBOSE=0

# shellcheck source=scripts/smoke-lib/common.sh
source "${SMOKE_LIB_DIR}/common.sh"
# shellcheck source=scripts/smoke-lib/fixtures.sh
source "${SMOKE_LIB_DIR}/fixtures.sh"
# shellcheck source=scripts/smoke-lib/phases/core_workflow.sh
source "${SMOKE_LIB_DIR}/phases/core_workflow.sh"
# shellcheck source=scripts/smoke-lib/phases/session_controls.sh
source "${SMOKE_LIB_DIR}/phases/session_controls.sh"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary)
      AGBRANCH_BIN="$2"
      shift 2
      ;;
    --verbose)
      VERBOSE=1
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
ARTIFACT_DIR="${ROOT_DIR}/e2e/artifacts/${RUN_ID}"
RUN_TEMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/agbranch-smoke-run.XXXXXX")"
STATE_ROOT="${RUN_TEMP_ROOT}/state"
FIXTURE_WORK_ROOT="${RUN_TEMP_ROOT}/workspaces"
SESSION_TRACK_FILE="${RUN_TEMP_ROOT}/sessions.txt"
TIMEOUT_BIN=""
PREPARED_BASE_NAME="$(detect_prepared_base_name)"
SHORT_RUN_ID="$(printf '%s' "${RUN_ID}" | tr -cd '[:alnum:]' | tr '[:upper:]' '[:lower:]' | cut -c1-16)"
touch "${SESSION_TRACK_FILE}"

export AGBRANCH_STATE_ROOT="${STATE_ROOT}"
export AGBRANCH_BIN ARTIFACT_DIR FIXTURE_WORK_ROOT PREPARED_BASE_NAME ROOT_DIR RUN_ID \
  RUN_TEMP_ROOT SESSION_TRACK_FILE SHORT_RUN_ID SMOKE_LIB_DIR STATE_ROOT TIMEOUT_BIN VERBOSE

trap cleanup EXIT

main() {
  choose_timeout_bin
  log "agbranch_bin=${AGBRANCH_BIN} artifact_dir=${ARTIFACT_DIR} temp_root=${RUN_TEMP_ROOT}"
  run_phase preflight 30s phase_preflight
  run_phase happy-path 10m phase_happy_path
  run_phase session-controls 7m phase_session_controls
  run_phase parallel-isolation 5m phase_parallel_isolation
  run_phase blocked-sync 5m phase_blocked_sync
  run_phase discard-gc 3m phase_discard_gc
}

main "$@"
