#!/usr/bin/env bash
# T05-T08: Verify each node discovers its peers via the hub.

source /harness/harness.sh

echo "  Suite: Discovery"

TIMEOUT=60

# T05: Hub sees alpha (by name — hub receives announces directly)
if wait_for_peer "$HUB_SOCK" alpha "$TIMEOUT"; then
    pass "T05: hub sees alpha"
else
    fail "T05: hub sees alpha (timeout ${TIMEOUT}s)"
fi

# T06: Hub sees beta
if wait_for_peer "$HUB_SOCK" beta "$TIMEOUT"; then
    pass "T06: hub sees beta"
else
    fail "T06: hub sees beta (timeout ${TIMEOUT}s)"
fi

# T07: Hub sees gamma (cross-network, via mesh-isolated)
if wait_for_peer "$HUB_SOCK" gamma "$TIMEOUT"; then
    pass "T07: hub sees gamma"
else
    fail "T07: hub sees gamma (timeout ${TIMEOUT}s)"
fi

# T08: Alpha sees hub and beta by name
# Verify specific peers, not just a count
if wait_for_peer "$ALPHA_SOCK" hub "$TIMEOUT"; then
    pass "T08a: alpha sees hub"
else
    fail "T08a: alpha sees hub (timeout ${TIMEOUT}s)"
fi

if wait_for_peer "$ALPHA_SOCK" beta "$TIMEOUT"; then
    pass "T08b: alpha sees beta"
else
    fail "T08b: alpha sees beta (timeout ${TIMEOUT}s)"
fi

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
