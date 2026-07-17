#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
usage: preserve-oem-media.sh --device /dev/diskN --output-dir DIR [--mode bounded|full] --confirm READ-ONLY

Read evidence from a removable disk without writing it. Bounded mode (default)
preserves partition metadata, the first 64 MiB boot prefix, and each partition
up to 256 MiB. Full mode preserves the complete disk image.
EOF
}

device=""
output_dir=""
confirm=""
mode="bounded"
while (($#)); do
  case "$1" in
    --device) device=${2:?missing device}; shift 2 ;;
    --output-dir) output_dir=${2:?missing output directory}; shift 2 ;;
    --mode) mode=${2:?missing mode}; shift 2 ;;
    --confirm) confirm=${2:?missing confirmation}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[[ $confirm == READ-ONLY ]] || { echo "refusing: pass --confirm READ-ONLY" >&2; exit 2; }
[[ $mode == bounded || $mode == full ]] || { echo "mode must be bounded or full" >&2; exit 2; }
[[ $device == /dev/* && -b $device ]] || { echo "not a block device: $device" >&2; exit 2; }
[[ -n $output_dir && $output_dir != / ]] || { echo "unsafe output directory" >&2; exit 2; }
mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)
image="$output_dir/oem-card.img"
first="$output_dir/first-64MiB.bin"
metadata="$output_dir/media-metadata.txt"
checksums="$output_dir/checksums.sha256"
[[ ! -e $image && ! -e $first && ! -e $checksums ]] || { echo "refusing to overwrite existing evidence in $output_dir" >&2; exit 2; }

capture_partition() {
  local source=$1 name=$2 bytes=$3
  ((bytes <= 268435456)) || return 0
  dd if="$source" of="$output_dir/$name.img" bs=1m 2>/dev/null || \
    dd if="$source" of="$output_dir/$name.img" bs=1M status=none
}

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
    if [[ $mode == full ]]; then
      dd if="$raw" of="$image" bs=4m status=progress
    else
      dd if="$raw" of="$first" bs=1m count=64 2>/dev/null || dd if="$raw" of="$first" bs=1M count=64 status=none
      while read -r identifier bytes; do
        [[ -n $identifier && -n $bytes ]] || continue
        capture_partition "/dev/r$identifier" "$identifier" "$bytes"
      done < <(diskutil list -plist "/dev/$disk" | plutil -convert json -o - - | \
        python3 -c 'import json,sys; d=json.load(sys.stdin); [print(p["DeviceIdentifier"], p["Size"]) for p in d.get("AllDisksAndPartitions", [])[0].get("Partitions", [])]')
    fi
    ;;
  Linux)
    [[ $(lsblk -dnro TYPE "$device") == disk ]] || { echo "refusing partition target" >&2; exit 2; }
    [[ $(lsblk -dnro RM "$device") == 1 ]] || { echo "refusing non-removable media" >&2; exit 2; }
    size=$(blockdev --getsize64 "$device")
    { lsblk -O "$device"; fdisk -l "$device"; } >"$metadata"
    while read -r mountpoint; do [[ -z $mountpoint ]] || umount "$mountpoint"; done < <(lsblk -nrpo MOUNTPOINT "$device")
    if [[ $mode == full ]]; then
      dd if="$device" of="$image" bs=4M status=progress iflag=fullblock
    else
      dd if="$device" of="$first" bs=1M count=64 status=none iflag=fullblock
      while read -r name bytes; do
        capture_partition "/dev/$name" "$name" "$bytes"
      done < <(lsblk -bnro NAME,SIZE,TYPE "$device" | awk '$3 == "part" { print $1, $2 }')
    fi
    ;;
  *) echo "unsupported host OS" >&2; exit 2 ;;
esac

if [[ $mode == full ]]; then
  actual=$(stat -f %z "$image" 2>/dev/null || stat -c %s "$image")
  [[ $actual == "$size" ]] || { echo "capture size mismatch: expected $size, got $actual" >&2; exit 1; }
  dd if="$image" of="$first" bs=1m count=64 2>/dev/null || dd if="$image" of="$first" bs=1M count=64 status=none
fi
(
  cd "$output_dir"
  find . -maxdepth 1 -type f ! -name checksums.sha256 -print | sort | while read -r file; do
    shasum -a 256 "$file"
  done >checksums.sha256
)
cat "$checksums"
echo "oem_media_preservation=pass mode=$mode output=$output_dir source_bytes=$size"
