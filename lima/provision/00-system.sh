#!/bin/bash
# Install the base OS packages that later provisioning steps and session runtime
# features depend on. This is the foundational bootstrap for the guest image.
set -euxo pipefail

MARKER_DIR="/var/lib/agbranch/provision"
MARKER_FILE="${MARKER_DIR}/00-system.done"

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
apt-get "${APT_RETRY_OPTS[@]}" install -y \
  bash \
  build-essential \
  ca-certificates \
  curl \
  git \
  gnupg \
  iproute2 \
  iptables \
  jq \
  pkg-config \
  python3 \
  rsync \
  tmux \
  unzip \
  zip

install -d -m 0755 "${MARKER_DIR}"
touch "${MARKER_FILE}"
