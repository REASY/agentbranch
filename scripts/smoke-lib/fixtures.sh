#!/usr/bin/env bash
# shellcheck disable=SC2154

create_fixture_repo() {
  local name="$1"
  local target="${FIXTURE_WORK_ROOT}/${name}"
  mkdir -p "${FIXTURE_WORK_ROOT}"
  mkdir -p "${target}"
  rsync -a \
    --exclude target \
    "${ROOT_DIR}/e2e/fixtures/sandbox-workspace/" "${target}/"
  git -C "${target}" init -q
  git -C "${target}" config user.name "agbranch-smoke"
  git -C "${target}" config user.email "agbranch-smoke@example.invalid"
  git -C "${target}" add .
  git -C "${target}" commit -qm "chore: seed smoke fixture"
  printf '%s\n' "${target}"
}
