#!/usr/bin/env bash
# Host-side mesh scenario controller.
# Owns container lifecycle/fault injection; the operator container remains
# unprivileged and only exercises Styrene's public CLI/IPC boundary.

set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
COMPOSE=${STYRENE_MESH_COMPOSE:-docker}
COMPOSE_BIN=${STYRENE_MESH_COMPOSE_BIN:-}
PROJECT=${STYRENE_MESH_PROJECT:-styrene-mesh}
RUN_ID=${STYRENE_MESH_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)-$$}
RESULT_DIR=${STYRENE_MESH_RESULT_DIR:-$ROOT/target/mesh-scenarios/$RUN_ID}
KEEP=${STYRENE_MESH_KEEP:-0}

case "$COMPOSE" in
  docker) compose=(docker compose -p "$PROJECT" -f "$ROOT/tests/mesh/docker-compose.yml") ;;
  podman)
    if [[ -n $COMPOSE_BIN ]]; then
      compose=("$COMPOSE_BIN" -p "$PROJECT" -f "$ROOT/tests/mesh/docker-compose.yml")
    elif command -v podman-compose >/dev/null 2>&1; then
      compose=(podman-compose -p "$PROJECT" -f "$ROOT/tests/mesh/docker-compose.yml")
    else
      echo "podman compose requires podman-compose or STYRENE_MESH_COMPOSE_BIN" >&2
      exit 2
    fi
    ;;
  *) echo "unsupported STYRENE_MESH_COMPOSE=$COMPOSE" >&2; exit 2 ;;
esac

mkdir -p "$RESULT_DIR"
started=$(date -u +%Y-%m-%dT%H:%M:%SZ)
status=failed
operator_rc=1
resilience_rc=1

collect() {
  "${compose[@]}" ps --all > "$RESULT_DIR/compose-ps.txt" 2>&1 || true
  "${compose[@]}" logs --no-color > "$RESULT_DIR/compose.log" 2>&1 || true
  cat > "$RESULT_DIR/result.json" <<EOF
{
  "schema_version": 1,
  "run_id": "$RUN_ID",
  "started_at": "$started",
  "finished_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "result": "$status",
  "operator_exit_code": $operator_rc,
  "resilience_exit_code": $resilience_rc,
  "artifacts": ["compose-ps.txt", "compose.log", "operator.log", "resilience.log"]
}
EOF
  if [[ $KEEP != 1 ]]; then "${compose[@]}" down -v --remove-orphans >/dev/null 2>&1 || true; fi
}
trap collect EXIT

"${compose[@]}" up -d --build hub alpha beta gamma
"${compose[@]}" run --rm operator > "$RESULT_DIR/operator.log" 2>&1 && operator_rc=0 || operator_rc=$?
if ((operator_rc != 0)); then
  echo "operator scenarios failed; see $RESULT_DIR/operator.log" >&2
  exit "$operator_rc"
fi

HARNESS_ROOT="$ROOT/tests/mesh" \
HUB_SOCK="$ROOT/target/unused-hub.sock" \
ALPHA_SOCK="$ROOT/target/unused-alpha.sock" \
BETA_SOCK="$ROOT/target/unused-beta.sock" \
GAMMA_SOCK="$ROOT/target/unused-gamma.sock" \
COMPOSE_PROJECT_NAME="$PROJECT" \
STYRENE_MESH_COMPOSE="$COMPOSE" \
STYRENE_MESH_COMPOSE_BIN="${compose[0]}" \
"$ROOT/tests/mesh/scenarios/06_resilience.sh" > "$RESULT_DIR/resilience.log" 2>&1 \
  && resilience_rc=0 || resilience_rc=$?

if ((resilience_rc != 0)); then
  echo "resilience scenario failed; see $RESULT_DIR/resilience.log" >&2
  exit "$resilience_rc"
fi

status=passed
echo "mesh scenarios passed: $RESULT_DIR"
