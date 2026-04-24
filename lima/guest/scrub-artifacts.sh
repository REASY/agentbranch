#!/usr/bin/env bash
set -euo pipefail

repo_root="${AGBRANCH_REPO:-}"
if [[ -z "${repo_root}" ]]; then
  echo "AGBRANCH_REPO is required" >&2
  exit 1
fi

cd "${repo_root}"

paths=(
  ".venv"
  "__pycache__"
  ".pytest_cache"
  ".mypy_cache"
  ".ruff_cache"
  ".tox"
  ".nox"
  ".coverage"
  "htmlcov"
  "build"
  "dist"
  "target"
  "project/target"
  "project/project/target"
  ".bloop"
  ".metals"
  ".bsp"
  ".scala-build"
)

for path in "${paths[@]}"; do
  find . -name "${path}" -prune -exec rm -rf {} +
done

find . -name "*.pyc" -delete
find . -name "*.egg-info" -prune -exec rm -rf {} +
