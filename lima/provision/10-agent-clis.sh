#!/bin/bash
# Install Node.js 20 and the supported agent CLIs used by agbranch-managed
# sessions: Codex, Claude Code, and Gemini CLI.
set -euxo pipefail

MARKER_DIR="/var/lib/agbranch/provision"
MARKER_FILE="${MARKER_DIR}/10-agent-clis.done"
NODE_MAJOR=20
NODE_VERSION="20.20.2-1nodesource1"
CODEX_VERSION="0.145.0"
CLAUDE_CODE_VERSION="2.1.218"
GEMINI_CLI_VERSION="0.52.0"
VERSION_STAMP="node=${NODE_VERSION} codex=${CODEX_VERSION} claude=${CLAUDE_CODE_VERSION} gemini=${GEMINI_CLI_VERSION}"

if [ -f "${MARKER_FILE}" ] && [ "$(cat "${MARKER_FILE}")" = "${VERSION_STAMP}" ]; then
  exit 0
fi

export DEBIAN_FRONTEND=noninteractive
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
cat > /etc/apt/preferences.d/nodesource <<'EOF'
Package: nodejs
Pin: origin deb.nodesource.com
Pin-Priority: 600
EOF
apt-get "${APT_RETRY_OPTS[@]}" update
apt-get "${APT_RETRY_OPTS[@]}" install -y --allow-downgrades "nodejs=${NODE_VERSION}"
command -v npm >/dev/null
npm install -g \
  "@openai/codex@${CODEX_VERSION}" \
  "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" \
  "@google/gemini-cli@${GEMINI_CLI_VERSION}"

install -d -m 0755 "${MARKER_DIR}"
printf '%s\n' "${VERSION_STAMP}" > "${MARKER_FILE}"
