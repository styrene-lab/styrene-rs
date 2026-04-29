#!/usr/bin/env bash
# T01-T04: Verify each node is reachable via its relay port.

source /harness/harness.sh

echo "  Suite: Infrastructure"

# T01: Hub is up
OUTPUT=$(styrene --socket tcp://hub:9001 status 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T01: hub status responds"
else
    fail "T01: hub status responds (exit $RC)"
fi

# T02: Alpha is up
OUTPUT=$(styrene --socket tcp://alpha:9002 status 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T02: alpha status responds"
else
    fail "T02: alpha status responds (exit $RC)"
fi

# T03: Beta is up
OUTPUT=$(styrene --socket tcp://beta:9003 status 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T03: beta status responds"
else
    fail "T03: beta status responds (exit $RC)"
fi

# T04: Gamma is up
OUTPUT=$(styrene --socket tcp://gamma:9004 status 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T04: gamma status responds"
else
    fail "T04: gamma status responds (exit $RC)"
fi

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
