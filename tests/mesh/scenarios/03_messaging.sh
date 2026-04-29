#!/usr/bin/env bash
# T09-T12: Send LXMF messages between peers and verify delivery.

source /harness/harness.sh

echo "  Suite: Messaging"

MSG_TIMEOUT=30

# T09: Send message from alpha to beta
MSG_ID_AB=""
OUTPUT=$(styrene --socket tcp://alpha:9002 send --to beta --message "hello from alpha" 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T09: alpha sends message to beta"
    MSG_ID_AB=$(echo "$OUTPUT" | grep -oE '[a-f0-9-]{36}' | head -1 || true)
else
    fail "T09: alpha sends message to beta (exit $RC)"
fi

# T10: Beta receives message from alpha
ELAPSED=0
RECEIVED=false
while [ "$ELAPSED" -lt "$MSG_TIMEOUT" ]; do
    if styrene --socket tcp://beta:9003 messages 2>/dev/null | grep -qF "hello from alpha"; then
        RECEIVED=true
        break
    fi
    sleep 2
    ELAPSED=$((ELAPSED + 2))
done
if [ "$RECEIVED" = true ]; then
    pass "T10: beta received message from alpha"
else
    fail "T10: beta received message from alpha (timeout ${MSG_TIMEOUT}s)"
fi

# T11: Send message from alpha to gamma (cross-network via hub)
OUTPUT=$(styrene --socket tcp://alpha:9002 send --to gamma --message "hello across networks" 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T11: alpha sends message to gamma (cross-network)"
else
    fail "T11: alpha sends message to gamma (exit $RC)"
fi

# T12: Gamma receives message from alpha
ELAPSED=0
RECEIVED=false
while [ "$ELAPSED" -lt "$MSG_TIMEOUT" ]; do
    if styrene --socket tcp://gamma:9004 messages 2>/dev/null | grep -qF "hello across networks"; then
        RECEIVED=true
        break
    fi
    sleep 2
    ELAPSED=$((ELAPSED + 2))
done
if [ "$RECEIVED" = true ]; then
    pass "T12: gamma received cross-network message from alpha"
else
    fail "T12: gamma received cross-network message from alpha (timeout ${MSG_TIMEOUT}s)"
fi

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
