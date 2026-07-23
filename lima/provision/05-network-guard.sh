#!/bin/bash
# Refresh the guest egress guard. This blocks access to common private
# subnets from both host processes and rootful Docker containers, over IPv4
# and IPv6, while still allowing loopback, DNS, established traffic, and
# session-local Docker bridges.
set -euxo pipefail

MARKER_DIR="/var/lib/agbranch/provision"
MARKER_FILE="${MARKER_DIR}/05-network-guard.done"
OUTPUT_CHAIN="AGBRANCH_OUTPUT_GUARD"
FORWARD_CHAIN="AGBRANCH_FORWARD_GUARD"

install -d -m 0755 "${MARKER_DIR}"

if [ "${1:-}" != "--apply-only" ]; then
  install -m 0755 "$0" /usr/local/sbin/agbranch-network-guard
  cat > /etc/systemd/system/agbranch-network-guard.service <<'EOF'
[Unit]
Description=AgentBranch private-network egress guard
Wants=network-online.target
After=network-online.target docker.service

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/agbranch-network-guard --apply-only
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOF
  systemctl daemon-reload
  systemctl enable agbranch-network-guard.service
fi

iptables -N "${OUTPUT_CHAIN}" 2>/dev/null || true
iptables -F "${OUTPUT_CHAIN}"
iptables -N "${FORWARD_CHAIN}" 2>/dev/null || true
iptables -F "${FORWARD_CHAIN}"

iptables -A "${OUTPUT_CHAIN}" -o lo -j RETURN
iptables -A "${OUTPUT_CHAIN}" -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN
iptables -A "${FORWARD_CHAIN}" -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN

while read -r dns_ip; do
  if [ -n "${dns_ip}" ]; then
    iptables -A "${OUTPUT_CHAIN}" -p udp -d "${dns_ip}" --dport 53 -j RETURN
    iptables -A "${OUTPUT_CHAIN}" -p tcp -d "${dns_ip}" --dport 53 -j RETURN
  fi
done < <(resolvectl dns 2>/dev/null | grep -Eo '([0-9]{1,3}\.){3}[0-9]{1,3}' | sort -u)

# Lima's hostResolver DNATs guest DNS requests in nat/OUTPUT to a host-side
# private IP and ephemeral port. Allow those translated destinations
# explicitly so the private-subnet rejects below do not break name resolution.
while read -r proto host_dns_ip host_dns_port; do
  if [ -n "${proto}" ] && [ -n "${host_dns_ip}" ] && [ -n "${host_dns_port}" ]; then
    iptables -A "${OUTPUT_CHAIN}" -p "${proto}" -d "${host_dns_ip}" --dport "${host_dns_port}" -j RETURN
  fi
done < <(
  iptables -t nat -S LIMADNS 2>/dev/null \
    | sed -nE 's/^-A LIMADNS -d [0-9.]+\/32 -p (udp|tcp).* --to-destination ([0-9.]+):([0-9]+)$/\1 \2 \3/p'
)

while read -r subnet; do
  if [ -n "${subnet}" ]; then
    iptables -A "${OUTPUT_CHAIN}" -d "${subnet}" -j RETURN
    iptables -A "${FORWARD_CHAIN}" -d "${subnet}" -j RETURN
  fi
done < <(ip -o -4 route show | awk '/ dev (docker0|br-)/ { print $1 }')

for subnet in 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 169.254.0.0/16; do
  iptables -A "${OUTPUT_CHAIN}" -d "${subnet}" -j REJECT
  iptables -A "${FORWARD_CHAIN}" -d "${subnet}" -j REJECT
done
iptables -A "${OUTPUT_CHAIN}" -j RETURN
iptables -A "${FORWARD_CHAIN}" -j RETURN

iptables -C OUTPUT -j "${OUTPUT_CHAIN}" 2>/dev/null \
  || iptables -I OUTPUT 1 -j "${OUTPUT_CHAIN}"
iptables -C FORWARD -j "${FORWARD_CHAIN}" 2>/dev/null \
  || iptables -I FORWARD 1 -j "${FORWARD_CHAIN}"
# Docker evaluates DOCKER-USER before its own accept rules. Hook there as
# well so a later Docker rules refresh cannot bypass the guard.
if iptables -S DOCKER-USER >/dev/null 2>&1; then
  iptables -C DOCKER-USER -j "${FORWARD_CHAIN}" 2>/dev/null \
    || iptables -I DOCKER-USER 1 -j "${FORWARD_CHAIN}"
fi

if command -v ip6tables >/dev/null 2>&1; then
  ip6tables -N "${OUTPUT_CHAIN}" 2>/dev/null || true
  ip6tables -F "${OUTPUT_CHAIN}"
  ip6tables -N "${FORWARD_CHAIN}" 2>/dev/null || true
  ip6tables -F "${FORWARD_CHAIN}"

  ip6tables -A "${OUTPUT_CHAIN}" -o lo -j RETURN
  ip6tables -A "${OUTPUT_CHAIN}" -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN
  ip6tables -A "${FORWARD_CHAIN}" -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN

  while read -r dns_ip; do
    if [ -n "${dns_ip}" ]; then
      ip6tables -A "${OUTPUT_CHAIN}" -p udp -d "${dns_ip}" --dport 53 -j RETURN
      ip6tables -A "${OUTPUT_CHAIN}" -p tcp -d "${dns_ip}" --dport 53 -j RETURN
    fi
  done < <(
    resolvectl dns 2>/dev/null \
      | grep -Eo '([0-9A-Fa-f]{0,4}:){2,7}[0-9A-Fa-f]{0,4}' \
      | sort -u
  )

  while read -r subnet; do
    if [ -n "${subnet}" ]; then
      ip6tables -A "${OUTPUT_CHAIN}" -d "${subnet}" -j RETURN
      ip6tables -A "${FORWARD_CHAIN}" -d "${subnet}" -j RETURN
    fi
  done < <(ip -o -6 route show | awk '/ dev (docker0|br-)/ { print $1 }')

  for subnet in fc00::/7 fe80::/10; do
    ip6tables -A "${OUTPUT_CHAIN}" -d "${subnet}" -j REJECT
    ip6tables -A "${FORWARD_CHAIN}" -d "${subnet}" -j REJECT
  done
  ip6tables -A "${OUTPUT_CHAIN}" -j RETURN
  ip6tables -A "${FORWARD_CHAIN}" -j RETURN

  ip6tables -C OUTPUT -j "${OUTPUT_CHAIN}" 2>/dev/null \
    || ip6tables -I OUTPUT 1 -j "${OUTPUT_CHAIN}"
  ip6tables -C FORWARD -j "${FORWARD_CHAIN}" 2>/dev/null \
    || ip6tables -I FORWARD 1 -j "${FORWARD_CHAIN}"
  if ip6tables -S DOCKER-USER >/dev/null 2>&1; then
    ip6tables -C DOCKER-USER -j "${FORWARD_CHAIN}" 2>/dev/null \
      || ip6tables -I DOCKER-USER 1 -j "${FORWARD_CHAIN}"
  fi
fi

if [ "${1:-}" != "--apply-only" ]; then
  touch "${MARKER_FILE}"
fi
