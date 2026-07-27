#!/usr/bin/env bash
set -euo pipefail

engine="${CONTAINER_ENGINE:-podman}"
image="${NIX_LINUX_IMAGE:-docker.io/nixos/nix:2.31.2}"
artifact="${1:-}"
[[ -n $artifact && -f $artifact ]] || {
  echo "usage: $0 PATH.img.zst|PATH.img" >&2
  exit 2
}
artifact=$(cd "$(dirname "$artifact")" && pwd)/$(basename "$artifact")
work=$(mktemp -d "${TMPDIR:-/tmp}/styrene-rpi4-verify.XXXXXX")
trap 'rm -rf "$work"' EXIT

case "$artifact" in
  *.img.zst)
    zstd -t -- "$artifact"
    zstd -dc -- "$artifact" >"$work/image.img"
    ;;
  *.img)
    cp "$artifact" "$work/image.img"
    ;;
  *)
    echo "unsupported image type: $artifact" >&2
    exit 2
    ;;
esac

python3 - "$work/image.img" "$work/root.img" <<'PY'
from pathlib import Path
import struct
import sys

image = Path(sys.argv[1])
root = Path(sys.argv[2])
with image.open("rb") as source:
    mbr = source.read(512)
    if mbr[510:512] != b"\x55\xaa":
        raise SystemExit("invalid MBR signature")
    entry = mbr[446 + 16:446 + 32]
    start, sectors = struct.unpack_from("<II", entry, 8)
    if entry[4] != 0x83 or not start or not sectors:
        raise SystemExit("second partition is not a Linux root partition")
    source.seek(start * 512)
    remaining = sectors * 512
    with root.open("wb") as target:
        while remaining:
            chunk = source.read(min(8 * 1024 * 1024, remaining))
            if not chunk:
                raise SystemExit("image ended inside root partition")
            target.write(chunk)
            remaining -= len(chunk)
print(f"root_partition_start={start} root_partition_sectors={sectors}")
PY

$engine run --rm --privileged \
  -v "$work:/verify" \
  "$image" sh -eu -c '
    nix --extra-experimental-features "nix-command flakes" \
      --extra-experimental-features nix-command \
      shell nixpkgs#e2fsprogs nixpkgs#util-linux --command sh -eu -c '\''
        e2fsck -fn /verify/root.img
        mkdir /verify/root
        loop=$(losetup -f --show /verify/root.img)
        mount -o rw "$loop" /verify/root
        cleanup() {
          umount /verify/root 2>/dev/null || true
          losetup -d "$loop" 2>/dev/null || true
        }
        trap cleanup EXIT INT TERM
        test -e /verify/root/nix-path-registration
        mkdir -p /verify/root/nix/var/nix/db
        nix-store --store "local?root=/verify/root" --load-db < /verify/root/nix-path-registration
        nix-store --store "local?root=/verify/root" --verify --check-contents
        sync
        umount /verify/root
        losetup -d "$loop"
        trap - EXIT INT TERM
        e2fsck -fn /verify/root.img
      '\''
  '

echo "offline_store_verification=pass"
