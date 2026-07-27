#!/usr/bin/env bash
set -euo pipefail

attr=${1:?usage: build-on-rpi4-builder.sh FLAKE-ATTRIBUTE [OUT-LINK]}
out=${2:-result-rpi-build}
host=${STYRENE_RPI4_BUILDER:-$(./scripts/discover-rpi4-builder.sh)}
[[ $host =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9.:-]+$ ]] || { echo "invalid builder host" >&2; exit 2; }
[[ $out =~ ^[A-Za-z0-9._-]+$ ]] || { echo "output link must be a project-root name" >&2; exit 2; }

./scripts/verify-rpi4-builder-host.sh --host "$host"

store_uri="ssh-ng://$host"
drv=$(nix --extra-experimental-features 'nix-command flakes' path-info --derivation "$attr")
[[ $drv =~ ^/nix/store/[a-z0-9]{32}-[^/]+\.drv$ ]] || { echo "unexpected derivation: $drv" >&2; exit 1; }

nix --extra-experimental-features 'nix-command flakes' copy --to "$store_uri" "$drv"
./scripts/remote-rpi4-build-job.sh start "$host" "$drv"
./scripts/remote-rpi4-build-job.sh wait "$host" "$drv"
report=$(./scripts/remote-rpi4-build-job.sh status "$host" "$drv")
outputs=()
while IFS= read -r line; do outputs+=("$line"); done < <(sed -n 's/^output=//p' <<<"$report")
((${#outputs[@]} > 0)) || { echo "remote build succeeded without outputs" >&2; exit 1; }
output=${outputs[0]}
[[ $output == /nix/store/* ]] || { echo "remote build returned invalid output: $output" >&2; exit 1; }

# Preserve every completed output as a restorable NAR before exporting its
# presentation files. This retains Nix metadata and the transitive closure.
./scripts/archive-rpi4-outputs.sh "${outputs[@]}"

rm -rf "$out"
mkdir -p "$out"
# The dedicated builder does not yet sign Nix store outputs. Export artifacts
# through authenticated SSH instead of weakening the workstation Nix daemon's
# signature policy. Release/image verification remains a separate required gate.
if ssh -o BatchMode=yes "$host" test -d "$output"; then
  ssh -o BatchMode=yes "$host" tar -C "$output" -cf - . | tar -C "$out" -xf -
else
  ssh -o BatchMode=yes "$host" cat "$output" > "$out/artifact"
fi
printf '%s\n' "$output" > "$out/.remote-nix-output"
printf 'builder=%s\nderivation=%s\noutput=%s\nout_dir=%s\n' "$host" "$drv" "$output" "$out"
