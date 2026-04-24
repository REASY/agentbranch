#!/bin/bash
# Install the Docker Compose plugin expected by the prepared base so sessions
# can run compose-based workflows inside the guest.
set -euxo pipefail

MARKER_DIR="/var/lib/agbranch/provision"
MARKER_FILE="${MARKER_DIR}/20-docker-compose.done"

if [ -f "${MARKER_FILE}" ]; then
  exit 0
fi

export DEBIAN_FRONTEND=noninteractive
APT_RETRY_OPTS=(
  -o Acquire::Retries=5
  -o Acquire::http::Timeout=60
  -o Acquire::https::Timeout=60
  -o Acquire::ForceIPv4=true
)
apt-get "${APT_RETRY_OPTS[@]}" update
apt-get "${APT_RETRY_OPTS[@]}" install -y docker-compose-plugin

install -d -m 0755 "${MARKER_DIR}"
touch "${MARKER_FILE}"
