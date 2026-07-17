#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
usage: preserve-oem-media.sh --device /dev/diskN --output-dir DIR --confirm READ-ONLY

Read and preserve a complete removable disk without writing it. On macOS the
whole-disk target is unmounted before capture and remounted afterwards.
EOF
}

device=""
output_dir=""
confirm=""
while (($#)); do
  case "$1" in
    --device) device=${2:?missing device}; shift 2 ;;
    --output-dir) output_dir=${2:?missing output directory}; shift 2 ;;
    --confirm) confirm=${2:?missing confirmation}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[[ $confirm == READ-ONLY ]] || { echo "refusing: pass --confirm READ-ONLY" >&2; exit 2; }
[[ $device == /dev/* && -b $device ]] || { echo "not a block device: $device" >&2; exit 2; }
[[ -n $output_dir && $output_dir != / ]] || { echo "unsafe output directory" >&2; exit 2; }
mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)
image="$output_dir/oem-card.img"
first="$output_dir/first-64MiB.bin"
metadata="$output_dir/media-metadata.txt"
checksums="$output_dir/checksums.sha256"
[[ ! -e $image && ! -e $first ]] || { echo "refusing to overwrite existing evidence in $output_dir" >&2; exit 2; }

case "$(uname -s)" in
  Darwin)
    disk=${device#/dev/}; disk=${disk#r}; disk=${disk%%s[0-9]*}
    info=$(diskutil info "/dev/$disk")
    whole=$(awk -F: '/^[[:space:]]*Whole:/ {gsub(/[[:space:]]/,"",$2); print $2}' <<<"$info")
    removable=$(awk -F: '/^[[:space:]]*Removable Media:/ {gsub(/^[[:space:]]+|[[:space:]]+$/,"",$2); print $2}' <<<"$info")
    [[ $whole == Yes ]] || { echo "refusing partition target" >&2; exit 2; }
    [[ $removable == Removable ]] || { echo "refusing non-removable media" >&2; exit 2; }
    size=$(diskutil info "/dev/$disk" | awk -F'[()]' '/Disk Size:/ {gsub(/[^0-9]/,"",$2); print $2}')
    raw="/dev/r$disk"
    { diskutil info "/dev/$disk"; echo; diskutil list "/dev/$disk"; } >"$metadata"
    diskutil unmountDisk "/dev/$disk"
    trap 'diskutil mountDisk "/dev/'"$disk"'" >/dev/null 2>&1 || true' EXIT
    dd if="$raw" of="$image" bs=4M status=progress
    ;;
  Linux)
    [[ $(lsblk -dnro TYPE "$device") == disk ]] || { echo "refusing partition target" >&2; exit 2; }
    [[ $(lsblk -dnro RM "$device") == 1 ]] || { echo "refusing non-removable media" >&2; exit 2; }
    size=$(blockdev --getsize64 "$device")
    { lsblk -O "$device"; fdisk -l "$device"; } >"$metadata"
    while read -r mountpoint; do [[ -z $mountpoint ]] || umount "$mountpoint"; done < <(lsblk -nrpo MOUNTPOINT "$device")
    dd if="$device" of="$image" bs=4M status=progress iflag=fullblock
    ;;
  *) echo "unsupported host OS" >&2; exit 2 ;;
esac

actual=$(stat -f %z "$image" 2>/dev/null || stat -c %s "$image")
[[ $actual == "$size" ]] || { echo "capture size mismatch: expected $size, got $actual" >&2; exit 1; }
dd if="$image" of="$first" bs=1m count=64 2>/dev/null || dd if="$image" of="$first" bs=1M count=64 status=none
(
  cd "$output_dir"
  shasum -a 256 oem-card.img first-64MiB.bin media-metadata.txt >checksums.sha256
)
cat "$checksums"
echo "oem_media_preservation=pass output=$output_dir bytes=$actual"
