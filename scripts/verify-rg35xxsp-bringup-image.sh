#!/usr/bin/env bash
set -euo pipefail
artifact=${1:?usage: verify-rg35xxsp-bringup-image.sh ARTIFACT_DIR}
image=$(find "$artifact/sd-image" -maxdepth 1 -type f \( -name '*.img' -o -name '*.img.zst' \) -print -quit)
[[ -n $image ]] || { echo "RG35XXSP image missing" >&2; exit 1; }
[[ -f $artifact/manifest/layout && -f $artifact/manifest/image.sha256 ]] || { echo "RG35XXSP image manifest missing" >&2; exit 1; }
grep -qx 'delivery_authorized=false' "$artifact/manifest/layout"
grep -qx 'bootloader_offset_bytes=8192' "$artifact/manifest/layout"
grep -qx 'boot_partition_offset_bytes=8388608' "$artifact/manifest/layout"
grep -qx 'root_partition_offset_bytes=276824064' "$artifact/manifest/layout"
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
raw=$image
if [[ $image == *.zst ]]; then raw="$work/image.img"; zstd -dc "$image" > "$raw"; fi
expected=$(awk '{print $1}' "$artifact/manifest/image.sha256")
actual=$(shasum -a 256 "$raw" | awk '{print $1}')
[[ $actual == "$expected" ]] || { echo "image checksum mismatch" >&2; exit 1; }
python3 - "$raw" <<'PY'
import pathlib, struct, sys
p = pathlib.Path(sys.argv[1])
with p.open('rb') as f:
    f.seek(510); assert f.read(2) == b'\x55\xaa'
    f.seek(446); entries = [f.read(16) for _ in range(4)]
    p1, p2 = entries[:2]
    assert p1[4] == 0x0c and struct.unpack_from('<I', p1, 8)[0] == 16384
    assert p2[4] == 0x83 and struct.unpack_from('<I', p2, 8)[0] == 540672
    f.seek(8192); magic = f.read(8)
    assert b'eGON' in magic, f'missing Allwinner SPL magic: {magic!r}'
PY
printf 'rg35xxsp_structural_image=pass image=%s sha256=%s delivery_authorized=false\n' "$image" "$actual"
