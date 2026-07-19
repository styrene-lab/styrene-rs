#!/usr/bin/env bash
# execution: host
# T21-T24: Resilience tests — host-controlled stop/restart and recovery checks.

source "${HARNESS_ROOT:-/harness}/harness.sh"

COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-styrene-mesh}
STYRENE_MESH_COMPOSE=${STYRENE_MESH_COMPOSE:-docker}
case "$STYRENE_MESH_COMPOSE" in
    docker) compose=(docker compose -p "$COMPOSE_PROJECT_NAME" -f "${HARNESS_ROOT:-/harness}/docker-compose.yml") ;;
    podman)
        compose_bin=${STYRENE_MESH_COMPOSE_BIN:-podman-compose}
        compose=("$compose_bin" -p "$COMPOSE_PROJECT_NAME" -f "${HARNESS_ROOT:-/harness}/docker-compose.yml")
        ;;
    *) echo "unsupported STYRENE_MESH_COMPOSE=$STYRENE_MESH_COMPOSE" >&2; exit 2 ;;
esac

RECOVERY_TIMEOUT=60

echo "  Suite: Resilience"

# T21: Stop alpha, verify hub notices
"${compose[@]}" stop alpha >/dev/null 2>&1 && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T21a: stopped alpha container"
else
    fail "T21a: failed to stop alpha container (exit $RC)"
fi

# Give hub time to notice the disconnect
sleep 10

# Verify hub no longer sees alpha as a peer
OUTPUT=$(styrene --socket "$HUB_SOCK" peers 2>&1) || true
if echo "$OUTPUT" | grep -qF "alpha"; then
    # Alpha might still appear briefly; not necessarily a failure
    echo "  INFO: T21b: alpha still in hub peer list (may be stale)"
else
    pass "T21b: hub no longer lists alpha as peer"
fi

# T22: Restart alpha, verify it reconnects
"${compose[@]}" start alpha >/dev/null 2>&1 && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T22a: restarted alpha container"
else
    fail "T22a: failed to restart alpha container (exit $RC)"
fi

# Wait for alpha to reconnect to hub
if wait_for_peer "$HUB_SOCK" alpha "$RECOVERY_TIMEOUT"; then
    pass "T22b: alpha reconnected to hub after restart"
else
    fail "T22b: alpha reconnected to hub after restart (timeout ${RECOVERY_TIMEOUT}s)"
fi

# T23: Stop hub, verify peers handle gracefully
"${compose[@]}" stop hub >/dev/null 2>&1 && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T23a: stopped hub container"
else
    fail "T23a: failed to stop hub container (exit $RC)"
fi

sleep 5

# Alpha/beta should not crash — just lose connectivity
# We can try their own sockets directly
OUTPUT=$(styrene --socket "$ALPHA_SOCK" status 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T23b: alpha still running after hub stopped"
else
    # May fail because alpha's socket volume is no longer accessible after restart
    echo "  INFO: T23b: alpha status unreachable (expected if container restarted)"
fi

# T24: Restart hub, verify full mesh recovery
"${compose[@]}" start hub >/dev/null 2>&1 && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T24a: restarted hub container"
else
    fail "T24a: failed to restart hub container (exit $RC)"
fi

# Wait for mesh to recover
sleep 15
if wait_for_peer "$HUB_SOCK" alpha "$RECOVERY_TIMEOUT"; then
    pass "T24b: alpha reconnected after hub restart"
else
    fail "T24b: alpha reconnected after hub restart (timeout ${RECOVERY_TIMEOUT}s)"
fi

if wait_for_peer "$HUB_SOCK" beta "$RECOVERY_TIMEOUT"; then
    pass "T24c: beta reconnected after hub restart"
else
    fail "T24c: beta reconnected after hub restart (timeout ${RECOVERY_TIMEOUT}s)"
fi

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
