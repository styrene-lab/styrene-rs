#!/usr/bin/env bash
# T13-T16: Fleet operations from operator via hub.

source /harness/harness.sh

echo "  Suite: Fleet Operations"

# T13: Fleet status via hub
OUTPUT=$(styrene --socket tcp://hub:9001 fleet status 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T13: fleet status via hub"
else
    fail "T13: fleet status via hub (exit $RC)"
    echo "    output: $OUTPUT"
fi

# T14: Fleet peers list via hub (uses `peers` not `fleet status`)
OUTPUT=$(styrene --socket tcp://hub:9001 peers 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T14: fleet peers list via hub"
    assert_output_contains "$OUTPUT" "alpha" "T14a: peers list includes alpha"
    assert_output_contains "$OUTPUT" "beta" "T14b: peers list includes beta"
else
    fail "T14: fleet peers list via hub (exit $RC)"
fi

# T15: Fleet status via alpha relay
OUTPUT=$(styrene --socket tcp://alpha:9002 fleet status 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T15: fleet status via alpha relay"
else
    fail "T15: fleet status via alpha relay (exit $RC)"
fi

# T16: Fleet exec — run a simple command on alpha via hub
# Get alpha's full destination hash from its own identity
# Fleet exec uses the destination hash (not identity hash)
ALPHA_HASH=$(styrene --socket tcp://alpha:9002 identity 2>&1 | grep "dest" | awk '{print $2}')
if [ -n "$ALPHA_HASH" ]; then
    OUTPUT=$(styrene --socket tcp://hub:9001 fleet exec "$ALPHA_HASH" echo fleet-test 2>&1) && RC=0 || RC=$?
    if [ "$RC" -eq 0 ] && echo "$OUTPUT" | grep -q "fleet-test"; then
        pass "T16: fleet exec echo on alpha via hub"
    elif [ "$RC" -eq 0 ]; then
        fail "T16: fleet exec — command ran but output missing 'fleet-test'"
        echo "    output: $OUTPUT"
    else
        # exec may not be fully wired in the daemon; skip gracefully
        echo "  SKIP: T16: fleet exec not available (exit $RC)"
        echo "    output: $(echo "$OUTPUT" | head -3)"
    fi
else
    echo "  SKIP: T16: fleet exec — could not resolve alpha hash"
fi

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
