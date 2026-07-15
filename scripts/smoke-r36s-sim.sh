#!/usr/bin/env bash
set -euo pipefail

image="${STYRENE_R36S_IMAGE:-localhost/styrene-r36s-sim:dev}"
platform="${STYRENE_R36S_PLATFORM:-linux/arm64}"
cpus="${STYRENE_R36S_CPUS:-4}"
memory="${STYRENE_R36S_MEMORY:-768m}"
state_size="${STYRENE_R36S_STATE_SIZE:-256m}"
engine="${CONTAINER_ENGINE:-podman}"

case "${1:-smoke}" in
  build)
    exec "$engine" build \
      --platform "$platform" \
      --file simulation/r36s/Containerfile \
      --tag "$image" \
      .
    ;;
  shell)
    exec "$engine" run --rm -it \
      --platform "$platform" \
      --cpus "$cpus" \
      --memory "$memory" \
      --tmpfs "/state:rw,size=$state_size,mode=0777" \
      --tmpfs "/run/styrene:rw,size=16m,mode=0777" \
      --entrypoint /bin/sh \
      "$image"
    ;;
  smoke)
    ;;
  *)
    echo "usage: $0 [build|smoke|shell]" >&2
    exit 2
    ;;
esac

run() {
  "$engine" run --rm \
    --platform "$platform" \
    --cpus "$cpus" \
    --memory "$memory" \
    --pids-limit 128 \
    --network none \
    --tmpfs "/state:rw,size=$state_size,mode=0777" \
    --tmpfs "/run/styrene:rw,size=16m,mode=0777" \
    "$image" "$@"
}

echo "== R36S-class simulation: binary =="
run --version

echo "== R36S-class simulation: persistent installation =="
run doctor --root /state/doctor

echo "== R36S-class simulation: ephemeral Ghost lifecycle =="
run ghost-check --root /state/ghost --timeout 15

echo "R36S-class smoke test: ok"
