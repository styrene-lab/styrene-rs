#!/usr/bin/env bash
set -euo pipefail

image="${STYRENE_R36S_IMAGE:-localhost/styrene-r36s-sim:dev}"
platform="${STYRENE_R36S_PLATFORM:-linux/arm64}"
cpus="${STYRENE_R36S_CPUS:-4}"
state_size="${STYRENE_R36S_STATE_SIZE:-256m}"
engine="${CONTAINER_ENGINE:-podman}"
limits="${STYRENE_R36S_MEMORY_MATRIX:-768m 512m 256m 128m 96m 64m}"
report_dir="${STYRENE_R36S_REPORT_DIR:-target/simulation/r36s}"
mkdir -p "$report_dir"
report="$report_dir/memory-matrix.tsv"

printf 'limit\tversion\tdoctor\tghost\n' >"$report"

run_at_limit() {
  local limit="$1"
  shift
  "$engine" run --rm \
    --platform "$platform" \
    --cpus "$cpus" \
    --memory "$limit" \
    --memory-swap "$limit" \
    --pids-limit 128 \
    --network none \
    --tmpfs "/state:rw,size=$state_size,mode=0777" \
    --tmpfs "/run/styrene:rw,size=16m,mode=0777" \
    "$image" "$@" >/dev/null 2>&1
}

for limit in $limits; do
  version=fail
  doctor=fail
  ghost=fail
  if run_at_limit "$limit" --version; then version=pass; fi
  if run_at_limit "$limit" doctor --root /state/doctor; then doctor=pass; fi
  if run_at_limit "$limit" ghost-check --root /state/ghost --timeout 15; then ghost=pass; fi
  printf '%s\t%s\t%s\t%s\n' "$limit" "$version" "$doctor" "$ghost" | tee -a "$report"
done

echo "report: $report"
