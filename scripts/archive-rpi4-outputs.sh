#!/usr/bin/env bash
set -euo pipefail

[[ $# -gt 0 ]] || { echo "usage: archive-rpi4-outputs.sh /nix/store/OUTPUT [...]" >&2; exit 2; }
host=${STYRENE_RPI4_BUILDER:-$(./scripts/discover-rpi4-builder.sh)}
archive_root=${STYRENE_BUILDER_ARCHIVE_ROOT:-.builder-artifacts/rpi4b}
[[ $host =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9.:-]+$ ]] || { echo "invalid builder host" >&2; exit 2; }
mkdir -p "$archive_root"

for output in "$@"; do
  [[ $output =~ ^/nix/store/[a-z0-9]{32}-[^/]+$ ]] || { echo "invalid Nix output: $output" >&2; exit 2; }
  ssh -o BatchMode=yes "$host" test -e "$output"
  base=${output##*/}
  archive="$archive_root/$base.nar.zst"
  manifest="$archive.manifest"
  tmp="$archive.tmp.$$"
  trap 'rm -f "$tmp"' EXIT

  ssh -o BatchMode=yes "$host" "nix-store --export '$output'" | zstd -T0 -10 -o "$tmp"
  mv "$tmp" "$archive"
  sha=$(shasum -a 256 "$archive" | awk '{print $1}')
  size=$(wc -c < "$archive" | tr -d ' ')
  remote_hash=$(ssh -o BatchMode=yes "$host" "nix-store -q --hash '$output'")
  refs=$(ssh -o BatchMode=yes "$host" "nix-store -q --references '$output'" | tr '\n' ' ')
  cat > "$manifest" <<EOF
schema_version=1
builder=$host
store_path=$output
nar_export=$archive
sha256=$sha
bytes=$size
nix_hash=$remote_hash
references=$refs
EOF
  ssh -o BatchMode=yes "$host" "mkdir -p /home/nix-builder/.local/state/styrene/gcroots && nix-store --add-root '/home/nix-builder/.local/state/styrene/gcroots/$base' --indirect -r '$output' >/dev/null"
  printf 'archived=%s\nmanifest=%s\nsha256=%s\n' "$archive" "$manifest" "$sha"
done
