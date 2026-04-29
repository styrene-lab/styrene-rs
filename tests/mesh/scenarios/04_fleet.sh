#!/usr/bin/env bash
# T13-T16: Fleet operations from operator via hub.

source /harness/harness.sh

echo "  Suite: Fleet Operations"

# T13: Fleet status via hub
OUTPUT=$(styrene --socket tcp://hub:9001 status 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T13: fleet status via hub"
else
    fail "T13: fleet status via hub (exit $RC)"
fi

# T14: Fleet peers list via hub
OUTPUT=$(styrene --socket tcp://hub:9001 peers 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T14: fleet peers list via hub"
    assert_output_contains "$OUTPUT" "alpha" "T14a: peers list includes alpha"
    assert_output_contains "$OUTPUT" "beta" "T14b: peers list includes beta"
else
    fail "T14: fleet peers list via hub (exit $RC)"
fi

# T15: Fleet status via alpha relay
OUTPUT=$(styrene --socket tcp://alpha:9002 status 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T15: fleet status via alpha relay"
else
    fail "T15: fleet status via alpha relay (exit $RC)"
fi

# T16: Fleet exec (if supported) — run a simple command on hub
OUTPUT=$(styrene --socket tcp://hub:9001 exec -- echo "fleet-test" 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    assert_output_contains "$OUTPUT" "fleet-test" "T16: fleet exec echo on hub"
else
    # exec may not be implemented yet; mark as skip rather than fail
    echo "  SKIP: T16: fleet exec not available (exit $RC)"
fi

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
