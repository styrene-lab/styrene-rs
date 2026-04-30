#!/usr/bin/env bash
# T05-T08: Verify each node discovers its peers via the hub.

source /harness/harness.sh

echo "  Suite: Discovery"

TIMEOUT=60

# T05: Hub sees alpha (by name — hub receives announces directly)
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

# T07: Hub sees gamma (cross-network, via mesh-isolated)
if wait_for_peer tcp://hub:9001 gamma "$TIMEOUT"; then
    pass "T07: hub sees gamma"
else
    fail "T07: hub sees gamma (timeout ${TIMEOUT}s)"
fi

# T08: Alpha sees at least 2 peers (hub + beta, possibly gamma)
# Edge nodes may not see names, but they see peer hashes via announces
ELAPSED=0
FOUND=false
while [ "$ELAPSED" -lt "$TIMEOUT" ]; do
    PEERS=$(styrene --socket tcp://alpha:9002 peers 2>&1)
    PEER_COUNT=$(echo "$PEERS" | grep -oE '[0-9]+ peers' | grep -oE '[0-9]+' | head -1)
    if [ "${PEER_COUNT:-0}" -ge 2 ]; then
        FOUND=true
        break
    fi
    sleep 2
    ELAPSED=$((ELAPSED + 2))
done
if [ "$FOUND" = true ]; then
    pass "T08: alpha sees at least 2 peers ($PEER_COUNT found)"
else
    fail "T08: alpha sees at least 2 peers (timeout ${TIMEOUT}s, found ${PEER_COUNT:-0})"
fi

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
