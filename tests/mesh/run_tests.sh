#!/usr/bin/env bash
# Mesh integration test orchestrator.
#
# Waits for mesh stabilization, then runs each scenario in order.
# Exits 0 on all pass, 1 on any failure.

set -euo pipefail

SCENARIO_DIR="/harness/scenarios"
TOTAL_PASS=0
TOTAL_FAIL=0

echo "========================================"
echo "  Mesh Integration Tests"
echo "========================================"
echo ""

# --- Wait for mesh stabilization ---
echo "Waiting for mesh to stabilize..."
MAX_WAIT=120
ELAPSED=0
while [ "$ELAPSED" -lt "$MAX_WAIT" ]; do
    # Check if hub is responding and has at least one peer
    # Note: styrene outputs to stderr, not stdout
    if OUTPUT=$(styrene --socket tcp://hub:9001 status 2>&1); then
        PEERS=$(styrene --socket tcp://hub:9001 peers 2>&1)
        if echo "$PEERS" | grep -q "peers)"; then
            PEER_COUNT=$(echo "$PEERS" | grep "peers)" | grep -oE "[0-9]+" | head -1)
            if [ "${PEER_COUNT:-0}" -gt 0 ]; then
                echo "Mesh is up — $PEER_COUNT peers (waited ${ELAPSED}s)"
                break
            fi
        fi
    fi
    sleep 3
    ELAPSED=$((ELAPSED + 3))
done

if [ "$ELAPSED" -ge "$MAX_WAIT" ]; then
    echo "ERROR: Mesh did not stabilize within ${MAX_WAIT}s"
    exit 1
fi

echo ""

# --- Run scenario files in order ---
for scenario in "$SCENARIO_DIR"/*.sh; do
    [ -f "$scenario" ] || continue
    scenario_name="$(basename "$scenario")"

    echo "----------------------------------------"
    echo "Running: $scenario_name"
    echo "----------------------------------------"

    # Run scenario in a subshell to isolate failures
    set +e
    SCENARIO_OUTPUT=$(bash "$scenario" 2>&1)
    SCENARIO_EXIT=$?
    set -e

    echo "$SCENARIO_OUTPUT"

    # Parse pass/fail counts from the last line of output
    # Scenarios should print "RESULTS: <pass> <fail>" as their last line
    RESULTS_LINE=$(echo "$SCENARIO_OUTPUT" | grep "^RESULTS:" | tail -1)
    if [ -n "$RESULTS_LINE" ]; then
        S_PASS=$(echo "$RESULTS_LINE" | awk '{print $2}')
        S_FAIL=$(echo "$RESULTS_LINE" | awk '{print $3}')
        TOTAL_PASS=$((TOTAL_PASS + S_PASS))
        TOTAL_FAIL=$((TOTAL_FAIL + S_FAIL))
    elif [ "$SCENARIO_EXIT" -ne 0 ]; then
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
    fi

    echo ""
done

# --- Summary ---
echo "========================================"
echo "  Final Results: $TOTAL_PASS passed, $TOTAL_FAIL failed"
echo "========================================"

if [ "$TOTAL_FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
