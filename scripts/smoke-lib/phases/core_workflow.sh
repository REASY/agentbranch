#!/usr/bin/env bash
# shellcheck disable=SC2154

phase_preflight() {
  mkdir -p "${ARTIFACT_DIR}" "${STATE_ROOT}"
  require_tool jq
  require_tool git
  require_tool rsync
  require_tool sqlite3
  require_tool limactl
  test -x "${AGBRANCH_BIN}"
  run_doctor_preflight
  assert_prepared_base_exists
}

phase_happy_path() {
  local sandbox_session="smk-${SHORT_RUN_ID}-sand"
  local repo_session="smk-${SHORT_RUN_ID}-repo"
  local handoff_session="smk-${SHORT_RUN_ID}-base"
  local seed_dir
  local repo
  local export_file="${ARTIFACT_DIR}/sandbox-report.md"

  seed_dir="$(create_fixture_repo "${sandbox_session}-seed")"
  repo="$(create_fixture_repo "${repo_session}")"
  remember_session "${sandbox_session}"
  remember_session "${repo_session}"
  remember_session "${handoff_session}"

  "${AGBRANCH_BIN}" launch --session "${sandbox_session}" --seed "${seed_dir}" --json > "${ARTIFACT_DIR}/launch.json"
  assert_json "${ARTIFACT_DIR}/launch.json" '.session == "'"${sandbox_session}"'" and .guest_workspace_path != null and .lifecycle_state == "running"'
  "${AGBRANCH_BIN}" run --session "${sandbox_session}" -- bash -lc "test \"\$AGBRANCH_SESSION\" = '${sandbox_session}' && test -f README.md && grep -q 'AgentBranch Smoke Fixture' README.md"
  "${AGBRANCH_BIN}" run --session "${sandbox_session}" -- bash -lc 'printf "\nsandbox-export\n" >> README.md'
  "${AGBRANCH_BIN}" export --session "${sandbox_session}" --from \~/sandbox/"${sandbox_session}"/README.md --to "${export_file}" --json > "${ARTIFACT_DIR}/export.json"
  assert_json "${ARTIFACT_DIR}/export.json" '.session == "'"${sandbox_session}"'" and .to != null'
  grep -q 'sandbox-export' "${export_file}"
  "${AGBRANCH_BIN}" close --session "${sandbox_session}" --discard --yes --json > "${ARTIFACT_DIR}/launch-close.json"
  assert_json "${ARTIFACT_DIR}/launch-close.json" '.session == "'"${sandbox_session}"'" and .outcome == "discard"'

  "${AGBRANCH_BIN}" open --session "${repo_session}" --repo "${repo}" --json > "${ARTIFACT_DIR}/open.json"
  assert_json "${ARTIFACT_DIR}/open.json" '.session == "'"${repo_session}"'" and .vm_name != null and .lifecycle_state == "running"'
  "${AGBRANCH_BIN}" run --session "${repo_session}" -- bash -lc "test \"\$AGBRANCH_SESSION\" = '${repo_session}' && test -f README.md && grep -q 'AgentBranch Smoke Fixture' README.md"
  "${AGBRANCH_BIN}" run --session "${repo_session}" -- bash -lc 'printf "\nrepo-happy-path\n" >> README.md && git add README.md && git commit -m "docs: happy path update"'

  "${AGBRANCH_BIN}" sync-back --session "${repo_session}" --yes --json > "${ARTIFACT_DIR}/sync-back.json"
  assert_json "${ARTIFACT_DIR}/sync-back.json" '.blocked == false and .staged_path != null'
  git -C "${repo}" show "agbranch/${repo_session}:README.md" | grep -q 'repo-happy-path'
  if grep -q 'repo-happy-path' "${repo}/README.md"; then
    echo "sync-back unexpectedly modified host README" >&2
    exit 1
  fi

  "${AGBRANCH_BIN}" close --session "${repo_session}" --sync --yes --json > "${ARTIFACT_DIR}/close-sync-result.json"
  assert_json "${ARTIFACT_DIR}/close-sync-result.json" '.session == "'"${repo_session}"'" and .destroy_result == "destroyed"'

  "${AGBRANCH_BIN}" open --session "${handoff_session}" --repo "${repo}" --base "agbranch/${repo_session}" --json > "${ARTIFACT_DIR}/open-base.json"
  assert_json "${ARTIFACT_DIR}/open-base.json" '.session == "'"${handoff_session}"'" and .repo_guest_path != null'
  "${AGBRANCH_BIN}" run --session "${handoff_session}" -- bash -lc 'grep -q "repo-happy-path" README.md'
  "${AGBRANCH_BIN}" close --session "${handoff_session}" --discard --yes --json > "${ARTIFACT_DIR}/open-base-close.json"
}

phase_parallel_isolation() {
  local session_a="smk-${SHORT_RUN_ID}-pa"
  local session_b="smk-${SHORT_RUN_ID}-pb"
  local seed_a
  local seed_b
  local pid_a
  local pid_b

  seed_a="$(create_fixture_repo "${session_a}-seed")"
  seed_b="$(create_fixture_repo "${session_b}-seed")"
  remember_session "${session_a}"
  remember_session "${session_b}"

  "${AGBRANCH_BIN}" launch --session "${session_a}" --seed "${seed_a}" --json > "${ARTIFACT_DIR}/parallel-a-launch.json" &
  pid_a=$!
  "${AGBRANCH_BIN}" launch --session "${session_b}" --seed "${seed_b}" --json > "${ARTIFACT_DIR}/parallel-b-launch.json" &
  pid_b=$!
  wait "${pid_a}"
  wait "${pid_b}"

  "${AGBRANCH_BIN}" run --session "${session_a}" -- bash -lc "test \"\$AGBRANCH_SESSION\" = '${session_a}' && test -f README.md"
  "${AGBRANCH_BIN}" run --session "${session_b}" -- bash -lc "test \"\$AGBRANCH_SESSION\" = '${session_b}' && test -f README.md"
  "${AGBRANCH_BIN}" close --session "${session_a}" --discard --yes --json > "${ARTIFACT_DIR}/parallel-a-close.json"
  "${AGBRANCH_BIN}" close --session "${session_b}" --discard --yes --json > "${ARTIFACT_DIR}/parallel-b-close.json"
}

phase_blocked_sync() {
  local session="smk-${SHORT_RUN_ID}-blocked"
  local repo
  local host_branch
  local watch_pid
  local status

  repo="$(create_fixture_repo "${session}")"
  remember_session "${session}"

  "${AGBRANCH_BIN}" open --session "${session}" --repo "${repo}" --json > "${ARTIFACT_DIR}/blocked-open.json"
  "${AGBRANCH_BIN}" watch --session "${session}" --json > "${ARTIFACT_DIR}/watch.ndjson" &
  watch_pid=$!

  "${AGBRANCH_BIN}" run --session "${session}" -- bash -lc 'printf "\nblocked-guest-edit\n" >> README.md && git add README.md && git commit -m "docs: blocked guest edit"'
  host_branch="$(git -C "${repo}" branch --show-current)"
  git -C "${repo}" switch -q -c "agbranch/${session}"
  printf '\nblocked-host-drift\n' >> "${repo}/README.md"
  git -C "${repo}" add README.md
  git -C "${repo}" commit -qm "test: drift review branch"
  git -C "${repo}" switch -q "${host_branch}"

  set +e
  "${AGBRANCH_BIN}" sync-back --session "${session}" --export-patch "${ARTIFACT_DIR}/blocked.patch" --json > "${ARTIFACT_DIR}/sync-back.json"
  status=$?
  set -e
  test "${status}" -eq 3

  assert_json "${ARTIFACT_DIR}/sync-back.json" '.blocked == true and .patch_path != null'
  (
    cd "${repo}"
    git apply --check "${ARTIFACT_DIR}/blocked.patch"
  )

  "${AGBRANCH_BIN}" logs --session "${session}" --source events --json > "${ARTIFACT_DIR}/agbranch-logs.txt"
  jq -e '.source != null and .session != null' "${ARTIFACT_DIR}/agbranch-logs.txt" >/dev/null

  sqlite3 "${AGBRANCH_STATE_ROOT}/state.db" \
    "UPDATE sessions SET lifecycle_state = 'destroying' WHERE name = '${session}';"
  "${AGBRANCH_BIN}" repair --session "${session}"

  kill -INT "${watch_pid}" >/dev/null 2>&1 || true
  wait "${watch_pid}" || true
}

phase_discard_gc() {
  local session="smk-${SHORT_RUN_ID}-discard"
  local seed_dir
  local status

  seed_dir="$(create_fixture_repo "${session}-seed")"
  remember_session "${session}"

  "${AGBRANCH_BIN}" launch --session "${session}" --seed "${seed_dir}" --json > "${ARTIFACT_DIR}/discard-open.json"
  "${AGBRANCH_BIN}" run --session "${session}" -- bash -lc 'printf "\ndiscard-me\n" >> README.md'

  set +e
  "${AGBRANCH_BIN}" close --session "${session}" > "${ARTIFACT_DIR}/close-no-outcome.stderr" 2>&1
  status=$?
  set -e
  test "${status}" -eq 1

  set +e
  "${AGBRANCH_BIN}" close --session "${session}" --sync --yes > "${ARTIFACT_DIR}/close-sync-invalid.stderr" 2>&1
  status=$?
  set -e
  test "${status}" -eq 1

  "${AGBRANCH_BIN}" export --session "${session}" --from \~/sandbox/"${session}" --to "${ARTIFACT_DIR}/discard-export" --json > "${ARTIFACT_DIR}/discard-export.json"
  assert_json "${ARTIFACT_DIR}/discard-export.json" '.session == "'"${session}"'" and .to != null'
  grep -q 'discard-me' "${ARTIFACT_DIR}/discard-export/README.md"

  "${AGBRANCH_BIN}" close --session "${session}" --discard --yes --json > "${ARTIFACT_DIR}/close-discard.json"
  assert_json "${ARTIFACT_DIR}/close-discard.json" '.session == "'"${session}"'" and .destroy_result == "destroyed"'

  "${AGBRANCH_BIN}" gc --json > "${ARTIFACT_DIR}/gc.json"
  assert_json "${ARTIFACT_DIR}/gc.json" '.warnings != null and .reclaimed_paths != null'
}
