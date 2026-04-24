#!/bin/bash
# Install Node.js 20 and the supported agent CLIs used by agbranch-managed
# sessions: Codex, Claude Code, and Gemini CLI.
set -euxo pipefail

MARKER_DIR="/var/lib/agbranch/provision"
MARKER_FILE="${MARKER_DIR}/10-agent-clis.done"

if [ -f "${MARKER_FILE}" ]; then
  exit 0
fi

export DEBIAN_FRONTEND=noninteractive
NODE_MAJOR=20
APT_RETRY_OPTS=(
  -o Acquire::Retries=5
  -o Acquire::http::Timeout=60
  -o Acquire::https::Timeout=60
  -o Acquire::ForceIPv4=true
)

install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key \
  | gpg --batch --yes --dearmor -o /etc/apt/keyrings/nodesource.gpg
echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_${NODE_MAJOR}.x nodistro main" \
  > /etc/apt/sources.list.d/nodesource.list
apt-get "${APT_RETRY_OPTS[@]}" update
apt-get "${APT_RETRY_OPTS[@]}" install -y nodejs
npm install -g @openai/codex @anthropic-ai/claude-code @google/gemini-cli

install -d -m 0755 "${MARKER_DIR}"
touch "${MARKER_FILE}"
