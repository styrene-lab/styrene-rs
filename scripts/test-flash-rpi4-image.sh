#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
flash="$script_dir/flash-rpi4-image.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
image="$tmp/test.img"
touch "$image" "$tmp/test.img.txt"

expect_failure() {
  expected=$1
  shift
  set +e
  output=$("$@" 2>&1)
  status=$?
  set -e
  if [[ $status -eq 0 || $output != *"$expected"* ]]; then
    printf 'expected failure containing %q, got status %s:\n%s\n' "$expected" "$status" "$output" >&2
    exit 1
  fi
}

expect_failure "Usage:" "$flash"
expect_failure "refusing: pass --confirm ERASE" "$flash" --image "$image" --device /dev/null
expect_failure "image not found" "$flash" --image "$tmp/missing.img" --device /dev/null --confirm ERASE
expect_failure "refusing unsupported image type" "$flash" --image "$tmp/test.img.txt" --device /dev/null --confirm ERASE
expect_failure "refusing non-device target" "$flash" --image "$image" --device "$tmp/device" --confirm ERASE
expect_failure "not a block device" "$flash" --image "$image" --device /dev/null --confirm ERASE
expect_failure "unknown argument" "$flash" --bogus

# A compressed stream commonly produces short reads. The flash pipeline must
# not use dd conv=sync, which would pad every short read and expand the image.
if grep -Eq 'dd .*conv=(sync|[^ ]*,sync)' "$flash"; then
  echo "compressed flash path must not pad short reads with conv=sync" >&2
  exit 1
fi

echo "flash guard tests: pass"
