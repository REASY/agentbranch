#!/bin/bash
# Refresh the guest egress guard. This blocks direct access to common private
# subnets while still allowing loopback, DNS, established traffic, and Docker bridges.
set -euxo pipefail

MARKER_DIR="/var/lib/agbranch/provision"
MARKER_FILE="${MARKER_DIR}/05-network-guard.done"
CHAIN="AGBRANCH_OUTPUT_GUARD"

install -d -m 0755 "${MARKER_DIR}"

iptables -N "${CHAIN}" 2>/dev/null || true
iptables -F "${CHAIN}"

iptables -A "${CHAIN}" -o lo -j RETURN
iptables -A "${CHAIN}" -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN

while read -r dns_ip; do
  if [ -n "${dns_ip}" ]; then
    iptables -A "${CHAIN}" -p udp -d "${dns_ip}" --dport 53 -j RETURN
    iptables -A "${CHAIN}" -p tcp -d "${dns_ip}" --dport 53 -j RETURN
  fi
done < <(resolvectl dns 2>/dev/null | grep -Eo '([0-9]{1,3}\.){3}[0-9]{1,3}' | sort -u)

while read -r subnet; do
  if [ -n "${subnet}" ]; then
    iptables -A "${CHAIN}" -d "${subnet}" -j RETURN
  fi
done < <(ip -o -4 route show | awk '/ dev (docker0|br-)/ { print $1 }')

iptables -A "${CHAIN}" -d 10.0.0.0/8 -j REJECT
iptables -A "${CHAIN}" -d 172.16.0.0/12 -j REJECT
iptables -A "${CHAIN}" -d 192.168.0.0/16 -j REJECT
iptables -A "${CHAIN}" -d 169.254.0.0/16 -j REJECT

iptables -C OUTPUT -j "${CHAIN}" 2>/dev/null || iptables -A OUTPUT -j "${CHAIN}"

touch "${MARKER_FILE}"
