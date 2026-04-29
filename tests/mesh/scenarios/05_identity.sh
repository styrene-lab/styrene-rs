#!/usr/bin/env bash
# T17-T19: Identity and profile operations.
# T20 (nex integration) is skipped — nex is not available in this container.

source /harness/harness.sh

echo "  Suite: Identity"

# T17: Create identity on alpha
OUTPUT=$(styrene --socket tcp://alpha:9002 identity create --name "test-alpha" 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T17: create identity on alpha"
else
    fail "T17: create identity on alpha (exit $RC)"
    echo "    output: $OUTPUT"
fi

# T18: Sign profile on alpha
OUTPUT=$(styrene --socket tcp://alpha:9002 profile sign 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T18: sign profile on alpha"
else
    fail "T18: sign profile on alpha (exit $RC)"
    echo "    output: $OUTPUT"
fi

# T19: Verify profile on alpha
OUTPUT=$(styrene --socket tcp://alpha:9002 profile verify 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    pass "T19: verify profile on alpha"
else
    fail "T19: verify profile on alpha (exit $RC)"
    echo "    output: $OUTPUT"
fi

# T20: Skipped (needs nex, which is in a separate repo)
echo "  SKIP: T20: nex integration (nex not available in container)"

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
