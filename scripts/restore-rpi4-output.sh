#!/usr/bin/env bash
set -euo pipefail

archive=${1:?usage: restore-rpi4-output.sh ARCHIVE.nar.zst}
host=${STYRENE_RPI4_BUILDER:-$(./scripts/discover-rpi4-builder.sh)}
[[ -f $archive && -f $archive.manifest ]] || { echo "archive or manifest missing: $archive" >&2; exit 2; }
[[ $host =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9.:-]+$ ]] || { echo "invalid builder host" >&2; exit 2; }
expected=$(sed -n 's/^sha256=//p' "$archive.manifest")
actual=$(shasum -a 256 "$archive" | awk '{print $1}')
[[ -n $expected && $actual == "$expected" ]] || { echo "archive checksum mismatch" >&2; exit 1; }
store_path=$(sed -n 's/^store_path=//p' "$archive.manifest")
[[ $store_path =~ ^/nix/store/[a-z0-9]{32}-[^/]+$ ]] || { echo "invalid manifest store path" >&2; exit 1; }

zstd -dc "$archive" | ssh -o BatchMode=yes "$host" nix-store --import >/dev/null
ssh -o BatchMode=yes "$host" test -e "$store_path"
printf 'restored=%s\nbuilder=%s\n' "$store_path" "$host"
