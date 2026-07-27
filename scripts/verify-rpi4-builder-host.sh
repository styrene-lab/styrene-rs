#!/usr/bin/env bash
set -euo pipefail

host=""
derivation=""
while (($#)); do
  case "$1" in
    --host) host=${2:?missing host}; shift 2 ;;
    --derivation) derivation=${2:?missing derivation}; shift 2 ;;
    -h|--help)
      echo "usage: $0 --host USER@HOST [--derivation /nix/store/NAME.drv]"
      exit 0
      ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[[ $host =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9._:-]+$ ]] || {
  echo "invalid --host; expected USER@HOST" >&2
  exit 2
}
if [[ -n $derivation ]]; then
  [[ $derivation =~ ^/nix/store/[a-z0-9]{32}-[^/]+\.drv$ && -e $derivation ]] || {
    echo "invalid or missing derivation: $derivation" >&2
    exit 2
  }
fi

ssh_args=(-o BatchMode=yes -o ConnectTimeout=10)
report=$(ssh "${ssh_args[@]}" "$host" 'set -eu
  test "$(uname -m)" = aarch64
  test "$(systemctl is-active nix-daemon.service)" = active
  test "$(nix --extra-experimental-features nix-command config show sandbox)" = true
  boot_source=$(findmnt -nro SOURCE /boot || true)
  [[ $boot_source == /dev/mmcblk0p1 || -e /boot/extlinux/extlinux.conf ]]
  printf "hostname=%s\n" "$(hostname)"
  printf "architecture=%s\n" "$(uname -m)"
  printf "kernel=%s\n" "$(uname -r)"
  printf "nix=%s\n" "$(nix --version)"
  printf "root_source=%s\n" "$(findmnt -nro SOURCE /)"
  printf "boot_source=%s\n" "$boot_source"
  printf "nix_daemon=active\n"
  printf "sandbox=true\n"')
printf '%s\n' "$report"

if [[ -n $derivation ]]; then
  nix copy --to "ssh-ng://$host" "$derivation"
  output=$(ssh "${ssh_args[@]}" "$host" nix-store -r "$derivation")
  [[ $output == /nix/store/* ]] || {
    echo "native derivation did not return a Nix store output" >&2
    exit 1
  }
  printf 'native_derivation=%s\nnative_output=%s\n' "$derivation" "$output"
fi

echo "rpi4_builder_acceptance=pass"
