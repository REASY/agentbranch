#!/bin/bash

export AGBRANCH_HOME="${HOME}/.agbranch"
export AGBRANCH_SESSION="${AGBRANCH_SESSION:-default}"
export AGBRANCH_SESSION_SAFE="${AGBRANCH_SESSION//[^A-Za-z0-9_-]/_}"
AGBRANCH_SESSION_SAFE_LOWER="$(printf '%s' "${AGBRANCH_SESSION_SAFE}" | tr '[:upper:]' '[:lower:]')"
export AGBRANCH_SESSION_SAFE_LOWER

export PATH="${HOME}/.agbranch/bin:${HOME}/.local/bin:${PATH}"
export COMPOSE_PROJECT_NAME="agbranch_${AGBRANCH_SESSION_SAFE_LOWER}"
