#!/usr/bin/env bash
set -euo pipefail

user=${STYRENE_RPI4_USER:-nix-builder}
subnet=${STYRENE_RPI4_SUBNET:-192.168.8}
name=${STYRENE_RPI4_HOSTNAME:-styrene-builder-a.local}

try_host() {
  local host=$1
  if ssh -o BatchMode=yes -o ConnectTimeout=2 -o StrictHostKeyChecking=accept-new "$user@$host" \
      'test "$(hostname)" = styrene-builder-a && test "$(uname -m)" = aarch64' \
      >/dev/null 2>&1; then
    printf '%s@%s\n' "$user" "$host"
    return 0
  fi
  return 1
}

try_host "$name" && exit 0
candidates=()
if command -v arp >/dev/null; then
  while read -r address; do candidates+=("$address"); done < <(
    arp -an | sed -nE "s/.*\(($subnet\.[0-9]+)\).*/\1/p" | sort -u
  )
fi
for host in "${candidates[@]}"; do
  try_host "$host" && exit 0
done

echo "RPi4 builder not found by mDNS or among known ARP neighbors on $subnet.0/24" >&2
exit 1
