#!/usr/bin/env bash
set -euo pipefail

engine="${CONTAINER_ENGINE:-podman}"
image="${NIX_LINUX_IMAGE:-docker.io/nixos/nix:2.31.2}"
volume="${NIX_LINUX_VOLUME:-styrene-nix-builder}"
backup="${NIX_LINUX_BACKUP:-$PWD/.nix-linux-store/styrene-nix-builder.nar.zst}"
container="${NIX_LINUX_CONTAINER:-styrene-nix-builder-job}"
attr="${1:-.#nixosConfigurations.rpi4-builder.config.system.build.sdImage}"
out="${2:-result-rpi4-builder}"
if [[ "$out" == "--out-link" ]]; then
  out="${3:?missing output-link path after --out-link}"
fi
if [[ "$out" == -* || "$out" == */* ]]; then
  echo "output link must be a project-root name, got: $out" >&2
  exit 2
fi
rm -rf "$PWD/$out"

: "${STYRENE_BUILDER_SSH_KEY:?set STYRENE_BUILDER_SSH_KEY to the operator public key}"

$engine volume inspect "$volume" >/dev/null 2>&1 || $engine volume create "$volume" >/dev/null
if [[ -f "$backup" ]]; then
  store_entries="$($engine run --rm -v "$volume:/nix" "$image" sh -lc 'find /nix/store -mindepth 1 -maxdepth 1 2>/dev/null | head -1')"
  if [[ -z "$store_entries" ]]; then
    echo "restoring persistent Nix store from $backup"
    $engine run --rm -i -v "$volume:/nix" "$image" \
      sh -lc 'zstd -d -c | tar -C / -xf -' <"$backup"
  fi
fi
if $engine container exists "$container"; then
  echo "builder container already exists: $container" >&2
  echo "inspect with: $engine logs -f $container" >&2
  exit 2
fi

cleanup() {
  $engine rm -f "$container" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

# macOS virtiofs can impose default ACL/xattr behavior on directories created
# below the bind-mounted workspace. Build temporary files on tmpfs; export only
# the final result link through /artifacts.
$engine run --name "$container" --privileged \
  -e STYRENE_BUILDER_SSH_KEY \
  -e TMPDIR=/build-tmp \
  --tmpfs /build-tmp:rw,exec,mode=1777 \
  -v "$volume:/nix" \
  -v "$PWD:/workspace:ro" \
  -v "$PWD:/artifacts" \
  -w /workspace \
  "$image" \
  sh -eu -c '
    mkdir -p /build-tmp/styrene-firmware
    chmod 0777 /build-tmp/styrene-firmware
    rm -f /build-result
    nix --extra-experimental-features "nix-command flakes" build --impure "$1" --out-link /build-result
    cp -aL /build-result "$2"
  ' sh "$attr" "/artifacts/$out"

printf 'built %s -> %s/%s\n' "$attr" "$PWD" "$out"
