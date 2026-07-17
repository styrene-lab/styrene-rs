#!/bin/sh
# Read-only Linux handheld inventory. Writes only beneath --output.
set -u

usage() {
  cat <<'EOF'
usage: device-report.sh [--output DIR] [--include-sensitive]

Collect a read-only hardware/OS report. By default, likely identifiers and
network configuration are omitted. No package installation, service changes,
mounts, or firmware writes are performed.
EOF
}

output="./device-report-$(date -u +%Y%m%dT%H%M%SZ)"
include_sensitive=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output=${2:?missing output directory}; shift 2 ;;
    --include-sensitive) include_sensitive=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

case "$output" in
  ""|/|/proc|/sys|/dev|/etc|/boot|/mnt|/media) echo "refusing unsafe output: $output" >&2; exit 2 ;;
esac
mkdir -p "$output"

have() { command -v "$1" >/dev/null 2>&1; }
collect() {
  name=$1
  shift
  file="$output/$name.txt"
  {
    printf '# command:'
    for arg in "$@"; do printf ' %s' "$arg"; done
    printf '\n'
    "$@"
  } >"$file" 2>&1 || printf '\n# exit: %s\n' "$?" >>"$file"
}
collect_sh() {
  name=$1
  command=$2
  file="$output/$name.txt"
  {
    printf '# shell: %s\n' "$command"
    sh -c "$command"
  } >"$file" 2>&1 || printf '\n# exit: %s\n' "$?" >>"$file"
}
copy_tree_file() {
  source=$1
  destination=$2
  if [ -r "$source" ]; then
    tr '\000' '\n' <"$source" >"$output/$destination.txt" 2>&1 || true
  fi
}

cat >"$output/REPORT.txt" <<EOF
schema_version=1
collector=device-report.sh
captured_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)
include_sensitive=$include_sensitive
read_only_source_operations=true
EOF

collect uname uname -a
collect os-release cat /etc/os-release
collect cpuinfo cat /proc/cpuinfo
collect meminfo cat /proc/meminfo
collect cmdline cat /proc/cmdline
collect mounts cat /proc/mounts
collect partitions cat /proc/partitions
collect modules cat /proc/modules
collect interrupts cat /proc/interrupts
collect input-devices cat /proc/bus/input/devices
collect filesystems cat /proc/filesystems
collect device-tree-tree find /proc/device-tree -maxdepth 4 -type f -print
copy_tree_file /proc/device-tree/model device-tree-model
copy_tree_file /proc/device-tree/compatible device-tree-compatible
copy_tree_file /proc/device-tree/serial-number device-tree-serial-number-REDACTED

if have lsblk; then collect lsblk lsblk -O; fi
if have findmnt; then collect findmnt findmnt; fi
if have lsusb; then collect lsusb lsusb -v; fi
if have lspci; then collect lspci lspci -nnvv; fi
if have rfkill; then collect rfkill rfkill list; fi
if have ip; then
  collect ip-link ip -details link show
  if [ "$include_sensitive" -eq 1 ]; then collect ip-address-SENSITIVE ip address show; fi
fi
if have iw; then collect iw-phy iw phy; fi
if have aplay; then collect aplay aplay -l; fi
if have arecord; then collect arecord arecord -l; fi
if have modetest; then collect modetest modetest -c -p; fi
if have fbset; then collect fbset fbset -i; fi
if have systemctl; then collect systemd-units systemctl list-units --all --no-pager; fi

collect_sh clocks 'for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_{cur,min,max}_freq /sys/devices/system/cpu/cpufreq/policy*/scaling_available_frequencies; do [ -r "$f" ] && printf "%s: " "$f" && cat "$f"; done'
collect_sh graphics 'for f in /sys/class/graphics/fb*/{name,virtual_size,bits_per_pixel} /sys/class/drm/*/{status,modes}; do [ -r "$f" ] && printf "=== %s ===\n" "$f" && cat "$f"; done'
collect_sh power 'for f in /sys/class/power_supply/*/{type,status,technology,capacity,voltage_now,current_now,charge_now,charge_full,charge_full_design}; do [ -r "$f" ] && printf "%s: " "$f" && cat "$f"; done'
collect_sh thermal 'for f in /sys/class/thermal/thermal_zone*/{type,temp}; do [ -r "$f" ] && printf "%s: " "$f" && cat "$f"; done'
collect_sh usb-role 'for f in /sys/class/usb_role/*/role; do [ -r "$f" ] && printf "%s: " "$f" && cat "$f"; done; find /sys/class/udc -mindepth 1 -maxdepth 1 -print 2>/dev/null || true'
collect_sh block-identifiers 'for f in /sys/class/block/mmcblk*/device/{name,type,manfid,oemid,date}; do [ -r "$f" ] && printf "%s: " "$f" && cat "$f"; done'
collect_sh firmware-files 'find /boot /mnt/vendor /vendor -xdev -maxdepth 4 -type f \( -name "*.dtb" -o -name "*.dtbo" -o -name "uImage" -o -name "Image" -o -iname "*u-boot*" -o -iname "*boot*bin*" \) -print 2>/dev/null'

if [ -r /proc/config.gz ] && have gzip; then gzip -dc /proc/config.gz >"$output/kernel-config.txt" 2>&1 || true; fi
if have dmesg; then
  if [ "$include_sensitive" -eq 1 ]; then collect dmesg-SENSITIVE dmesg; else collect_sh dmesg-redacted 'dmesg | sed -E "s/([[:xdigit:]]{2}:){5}[[:xdigit:]]{2}/<MAC-REDACTED>/g; s/(serial|Serial|SERIAL)[=: ]+[^ ]+/\1=<REDACTED>/g"'; fi
fi

# Hash small boot metadata without copying firmware or user data.
if have sha256sum; then
  collect_sh boot-hashes 'find /boot -xdev -type f -size -64M -print0 2>/dev/null | sort -z | xargs -0 -r sha256sum'
elif have shasum; then
  collect_sh boot-hashes 'find /boot -xdev -type f -size -64M -exec shasum -a 256 {} + 2>/dev/null'
fi

# Remove the serial-number capture unless the operator explicitly opted in.
if [ "$include_sensitive" -ne 1 ]; then rm -f "$output/device-tree-serial-number-REDACTED.txt"; fi

if have tar; then
  archive="$output.tar.gz"
  parent=$(dirname "$output")
  base=$(basename "$output")
  tar -C "$parent" -czf "$archive" "$base"
  printf 'report_dir=%s\narchive=%s\n' "$output" "$archive"
else
  printf 'report_dir=%s\narchive=unavailable\n' "$output"
fi
