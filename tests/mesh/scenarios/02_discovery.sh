#!/usr/bin/env bash
# T05-T08: Verify each node discovers its peers via the hub.

source /harness/harness.sh

echo "  Suite: Discovery"

TIMEOUT=60

# T05: Hub sees alpha
if wait_for_peer tcp://hub:9001 alpha "$TIMEOUT"; then
    pass "T05: hub sees alpha"
else
    fail "T05: hub sees alpha (timeout ${TIMEOUT}s)"
fi

# T06: Hub sees beta
if wait_for_peer tcp://hub:9001 beta "$TIMEOUT"; then
    pass "T06: hub sees beta"
else
    fail "T06: hub sees beta (timeout ${TIMEOUT}s)"
fi

# T07: Hub sees gamma
if wait_for_peer tcp://hub:9001 gamma "$TIMEOUT"; then
    pass "T07: hub sees gamma"
else
    fail "T07: hub sees gamma (timeout ${TIMEOUT}s)"
fi

# T08: Alpha sees beta (both on mesh-core, routed through hub)
if wait_for_peer tcp://alpha:9002 beta "$TIMEOUT"; then
    pass "T08: alpha sees beta"
else
    fail "T08: alpha sees beta (timeout ${TIMEOUT}s)"
fi

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
