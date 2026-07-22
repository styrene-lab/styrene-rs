#!/usr/bin/env bash
# Run only the focused cross-network scenario against an already-running mesh.
# For Compose, set sockets to the host-mounted daemon sockets. For K3s, execute
# this script in the operator pod where /run/{alpha,gamma}/daemon.sock exist.

set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
export HARNESS_ROOT=${HARNESS_ROOT:-$ROOT/tests/mesh}
export ALPHA_SOCK=${ALPHA_SOCK:-/run/alpha/daemon.sock}
export GAMMA_SOCK=${GAMMA_SOCK:-/run/gamma/daemon.sock}
export STYRENE_MESH_RUN_ID=${STYRENE_MESH_RUN_ID:-cross-network-$(date -u +%Y%m%dT%H%M%SZ)}

exec bash "$ROOT/tests/mesh/scenarios/08_cross_network.sh"
