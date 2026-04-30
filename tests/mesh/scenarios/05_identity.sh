#!/usr/bin/env bash
# T17-T19: Identity operations via styrene CLI.
# T20 (nex profile signing) is skipped — nex is not available in this container.

source /harness/harness.sh

echo "  Suite: Identity"

# T17: Query identity on hub
OUTPUT=$(styrene --socket tcp://hub:9001 identity 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ] && echo "$OUTPUT" | grep -q "hash"; then
    pass "T17: hub identity shows hash"
else
    fail "T17: hub identity shows hash (exit $RC)"
    echo "    output: $OUTPUT"
fi

# T18: Query identity on alpha
OUTPUT=$(styrene --socket tcp://alpha:9002 identity 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ] && echo "$OUTPUT" | grep -q "hash"; then
    pass "T18: alpha identity shows hash"
else
    fail "T18: alpha identity shows hash (exit $RC)"
    echo "    output: $OUTPUT"
fi

# T19: Identities are unique (hub != alpha)
HUB_HASH=$(styrene --socket tcp://hub:9001 identity 2>&1 | grep "hash" | awk '{print $2}')
ALPHA_HASH=$(styrene --socket tcp://alpha:9002 identity 2>&1 | grep "hash" | awk '{print $2}')
if [ -n "$HUB_HASH" ] && [ -n "$ALPHA_HASH" ] && [ "$HUB_HASH" != "$ALPHA_HASH" ]; then
    pass "T19: hub and alpha have different identities"
else
    fail "T19: hub and alpha have different identities"
    echo "    hub=$HUB_HASH alpha=$ALPHA_HASH"
fi

# T20: Skipped (needs nex for profile signing)
echo "  SKIP: T20: nex profile signing (nex not available in container)"

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
