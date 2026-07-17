#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/flash-rpi4-image.sh --image PATH --device /dev/DEVICE --confirm ERASE --dry-run
  sudo scripts/flash-rpi4-image.sh --image PATH --device /dev/DEVICE --confirm ERASE

The device must be an explicit whole removable disk. The script refuses mounted,
internal, virtual, partition, root-disk, and image-file targets.
USAGE
}

image=""
device=""
confirm=""
dry_run=false
while (($#)); do
  case "$1" in
    --image) image=${2:?missing image path}; shift 2 ;;
    --device) device=${2:?missing device path}; shift 2 ;;
    --confirm) confirm=${2:?missing confirmation}; shift 2 ;;
    --dry-run) dry_run=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n $image && -n $device ]] || { usage >&2; exit 2; }
[[ $confirm == ERASE ]] || { echo "refusing: pass --confirm ERASE" >&2; exit 2; }
[[ -f $image ]] || { echo "image not found: $image" >&2; exit 2; }
case "$image" in
  *.img|*.img.zst) ;;
  *) echo "refusing unsupported image type: $image" >&2; exit 2 ;;
esac
[[ $device == /dev/* ]] || { echo "refusing non-device target: $device" >&2; exit 2; }
[[ -b $device ]] || { echo "not a block device: $device" >&2; exit 2; }

case "$(uname -s)" in
  Darwin)
    [[ $device =~ ^/dev/(r)?disk[0-9]+$ ]] || { echo "refusing partition or unknown device: $device" >&2; exit 2; }
    disk=${device#/dev/r}; disk=${disk#/dev/}
    info=$(diskutil info "/dev/$disk")
    internal=$(awk -F: '/^[[:space:]]*Internal:/ {gsub(/[[:space:]]/,"",$2); print $2}' <<<"$info")
    virtual=$(awk -F: '/^[[:space:]]*Virtual:/ {gsub(/[[:space:]]/,"",$2); print $2}' <<<"$info")
    removable=$(awk -F: '/^[[:space:]]*Removable Media:/ {gsub(/[[:space:]]/,"",$2); print $2}' <<<"$info")
    [[ $internal == No || $removable == Removable ]] || { echo "refusing internal non-removable disk: /dev/$disk" >&2; exit 2; }
    [[ $virtual != Yes ]] || { echo "refusing virtual disk: /dev/$disk" >&2; exit 2; }
    device="/dev/r$disk"
    inspect="$(diskutil info /dev/$disk | grep -E 'Device Node|Media Name|Disk Size|Internal|Removable Media|Protocol' || true)"
    unmount=(diskutil unmountDisk "/dev/$disk")
    eject=(diskutil eject "/dev/$disk")
    ;;
  Linux)
    [[ $device =~ ^/dev/[a-zA-Z0-9._/-]+$ ]] || { echo "invalid device path" >&2; exit 2; }
    [[ $(lsblk -dnro TYPE "$device") == disk ]] || { echo "refusing partition target: $device" >&2; exit 2; }
    [[ $(lsblk -dnro RM "$device") == 1 ]] || { echo "refusing non-removable disk: $device" >&2; exit 2; }
    root_source=$(findmnt -nro SOURCE /)
    root_disk=$(lsblk -ndo PKNAME "$root_source" 2>/dev/null || true)
    [[ -n $root_disk ]] && root_source="/dev/$root_disk"
    [[ $device != "$root_source" ]] || { echo "refusing root disk: $device" >&2; exit 2; }
    [[ -z $(lsblk -nrpo MOUNTPOINTS "$device" | grep -v '^$' || true) ]] || { echo "refusing mounted disk: $device" >&2; exit 2; }
    inspect=$(lsblk -d -o NAME,SIZE,MODEL,TRAN,RM,RO "$device")
    unmount=(:)
    eject=(sync)
    ;;
  *) echo "unsupported host OS" >&2; exit 2 ;;
esac

printf 'IMAGE: %s\nDEVICE:\n%s\n' "$image" "$inspect"
if $dry_run; then
  echo "DRY RUN: validation passed; no bytes written"
  exit 0
fi
[[ $EUID -eq 0 ]] || { echo "flashing requires root; rerun with sudo" >&2; exit 2; }
"${unmount[@]}"
if [[ $image == *.zst ]]; then
  command -v zstd >/dev/null || { echo "zstd is required" >&2; exit 2; }
  zstd -dc -- "$image" | dd of="$device" bs=4m conv=sync,noerror status=progress
else
  dd if="$image" of="$device" bs=4m conv=sync,noerror status=progress
fi
sync
"${eject[@]}"
echo "flash complete: $device"
