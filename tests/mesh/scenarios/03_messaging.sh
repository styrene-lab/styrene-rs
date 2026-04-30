#!/usr/bin/env bash
# T09-T12: Send LXMF messages between peers and verify delivery.

source /harness/harness.sh

echo "  Suite: Messaging"

MSG_TIMEOUT=30

# Get each node's LXMF destination hash (for addressing messages)
# and identity hash (for querying received messages — the store uses identity hash)
ALPHA_DEST=$(styrene --socket tcp://alpha:9002 identity 2>&1 | grep "lxmf" | awk '{print $2}')
ALPHA_IDHASH=$(styrene --socket tcp://alpha:9002 identity 2>&1 | grep "^  hash" | awk '{print $2}')
BETA_DEST=$(styrene --socket tcp://beta:9003 identity 2>&1 | grep "lxmf" | awk '{print $2}')
GAMMA_DEST=$(styrene --socket tcp://gamma:9004 identity 2>&1 | grep "lxmf" | awk '{print $2}')

echo "  alpha lxmf: ${ALPHA_DEST:-UNKNOWN}"
echo "  alpha hash: ${ALPHA_IDHASH:-UNKNOWN}"
echo "  beta lxmf:  ${BETA_DEST:-UNKNOWN}"
echo "  gamma lxmf: ${GAMMA_DEST:-UNKNOWN}"

if [ -z "$BETA_DEST" ] || [ -z "$ALPHA_DEST" ]; then
    echo "  SKIP: T09-T10: LXMF destinations not available"
else
    # T09: Send message from alpha to beta
    OUTPUT=$(styrene --socket tcp://alpha:9002 send "$BETA_DEST" "hello from alpha" 2>&1) && RC=0 || RC=$?
    if [ "$RC" -eq 0 ]; then
        pass "T09: alpha sends message to beta"
    else
        fail "T09: alpha sends message to beta (exit $RC)"
        echo "    output: $OUTPUT"
    fi

    # T10: Beta receives message from alpha
    ELAPSED=0
    RECEIVED=false
    while [ "$ELAPSED" -lt "$MSG_TIMEOUT" ]; do
        MSGS=$(styrene --socket tcp://beta:9003 messages "$ALPHA_IDHASH" 2>&1)
        if echo "$MSGS" | grep -qF "hello from alpha"; then
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
fi

if [ -z "$GAMMA_DEST" ] || [ -z "$ALPHA_DEST" ]; then
    echo "  SKIP: T11-T12: LXMF destinations not available"
else
    # T11: Send message from alpha to gamma (cross-network via hub)
    OUTPUT=$(styrene --socket tcp://alpha:9002 send "$GAMMA_DEST" "hello across networks" 2>&1) && RC=0 || RC=$?
    if [ "$RC" -eq 0 ]; then
        pass "T11: alpha sends message to gamma (cross-network)"
    else
        fail "T11: alpha sends message to gamma (exit $RC)"
        echo "    output: $OUTPUT"
    fi

    # T12: Gamma receives message from alpha
    ELAPSED=0
    RECEIVED=false
    while [ "$ELAPSED" -lt "$MSG_TIMEOUT" ]; do
        MSGS=$(styrene --socket tcp://gamma:9004 messages "$ALPHA_IDHASH" 2>&1)
        if echo "$MSGS" | grep -qF "hello across networks"; then
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
fi

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
