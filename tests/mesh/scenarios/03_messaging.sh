#!/usr/bin/env bash
# T09-T12: Send LXMF messages between peers and verify delivery.

source /harness/harness.sh

echo "  Suite: Messaging"

MSG_TIMEOUT=90

# Get each node's LXMF destination hash (for addressing messages)
# and identity hash (for querying received messages — the store uses identity hash)
ALPHA_DEST=$(styrene --socket tcp://alpha:9002 identity 2>&1 | grep "lxmf" | awk '{print $2}')
# Extract identity hash (first "hash" line, not "destination_hash" etc)
ALPHA_IDHASH=$(styrene --socket tcp://alpha:9002 identity 2>&1 | awk '/hash/ && !/dest|lxmf/ {print $2; exit}')
BETA_DEST=$(styrene --socket tcp://beta:9003 identity 2>&1 | grep "lxmf" | awk '{print $2}')
GAMMA_DEST=$(styrene --socket tcp://gamma:9004 identity 2>&1 | grep "lxmf" | awk '{print $2}')

echo "  alpha lxmf: ${ALPHA_DEST:-UNKNOWN}"
echo "  alpha id:   ${ALPHA_IDHASH:-UNKNOWN}"
echo "  beta lxmf:  ${BETA_DEST:-UNKNOWN}"
echo "  gamma lxmf: ${GAMMA_DEST:-UNKNOWN}"

if [ -z "$BETA_DEST" ] || [ -z "$ALPHA_DEST" ]; then
    echo "  SKIP: T09-T10: LXMF destinations not available"
else
    # T09: Send message from alpha to beta
    # Wait for announce propagation to ensure alpha can resolve beta
    echo "  waiting for alpha to see beta before sending..."
    if ! wait_for_peer tcp://alpha:9002 beta 60; then
        echo "  WARNING: alpha may not see beta yet, attempting send anyway"
    fi
    sleep 5
    OUTPUT=$(styrene --socket tcp://alpha:9002 send "$BETA_DEST" "hello from alpha" 2>&1) && RC=0 || RC=$?
    if [ "$RC" -eq 0 ]; then
        pass "T09: alpha sends message to beta"
    else
        fail "T09: alpha sends message to beta (exit $RC)"
        echo "    output: $OUTPUT"
    fi

    # T10: Beta receives message from alpha
    # Allow transport time to establish link for delivery
    sleep 15
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
    # Wait for cross-network announce propagation (alpha→hub→gamma path is longer)
    echo "  waiting for cross-network announce propagation..."
    sleep 10
    OUTPUT=$(styrene --socket tcp://alpha:9002 send "$GAMMA_DEST" "hello across networks" 2>&1) && RC=0 || RC=$?
    if [ "$RC" -eq 0 ]; then
        pass "T11: alpha sends message to gamma (cross-network)"
    else
        fail "T11: alpha sends message to gamma (exit $RC)"
        echo "    output: $OUTPUT"
    fi

    # T12: Gamma receives message from alpha
    # Cross-network delivery takes longer — wait for transport + link setup
    sleep 15
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
        # Debug: check if gamma has ANY messages
        ALL_MSGS=$(styrene --socket tcp://gamma:9004 messages "$ALPHA_IDHASH" --limit 10 2>&1)
        fail "T12: gamma received cross-network message from alpha (timeout ${MSG_TIMEOUT}s)"
        echo "    query hash: $ALPHA_IDHASH"
        echo "    gamma messages: $ALL_MSGS"
    fi
fi

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
