#!/usr/bin/env bash
# Test harness with assertion helpers for mesh integration tests.
# Source this file from scenario scripts: source /harness/harness.sh

set -euo pipefail

_PASS_COUNT=0
_FAIL_COUNT=0
_TEST_NAMES_FAILED=()

pass() {
    local msg="$1"
    _PASS_COUNT=$((_PASS_COUNT + 1))
    echo "  PASS: $msg"
}

fail() {
    local msg="$1"
    _FAIL_COUNT=$((_FAIL_COUNT + 1))
    _TEST_NAMES_FAILED+=("$msg")
    echo "  FAIL: $msg"
}

# Check that the last command exited 0.
# Usage: some_command; assert_ok "description"
assert_ok() {
    local msg="$1"
    local exit_code="${2:-$?}"
    if [ "$exit_code" -eq 0 ]; then
        pass "$msg"
    else
        fail "$msg (exit code: $exit_code)"
    fi
}

# Check that output contains expected substring.
assert_output_contains() {
    local output="$1"
    local expected="$2"
    local msg="${3:-output contains '$expected'}"
    if echo "$output" | grep -qF "$expected"; then
        pass "$msg"
    else
        fail "$msg"
        echo "    expected to find: $expected"
        echo "    in output: $(echo "$output" | head -5)"
    fi
}

# Check that output does NOT contain a substring.
assert_output_not_contains() {
    local output="$1"
    local unexpected="$2"
    local msg="${3:-output does not contain '$unexpected'}"
    if echo "$output" | grep -qF "$unexpected"; then
        fail "$msg"
        echo "    unexpectedly found: $unexpected"
    else
        pass "$msg"
    fi
}

# Poll a node's peer list until a given peer name appears.
# Usage: wait_for_peer tcp://hub:9001 alpha 60
wait_for_peer() {
    local socket_url="$1"
    local peer_name="$2"
    local timeout="${3:-60}"
    local elapsed=0

    while [ "$elapsed" -lt "$timeout" ]; do
        if styrene --socket "$socket_url" peers 2>&1 | grep -qiF "$peer_name"; then
            return 0
        fi
        sleep 2
        elapsed=$((elapsed + 2))
    done
    return 1
}

# Print summary and exit with appropriate code.
finish() {
    echo ""
    echo "========================================"
    echo "  Results: $_PASS_COUNT passed, $_FAIL_COUNT failed"
    echo "========================================"
    if [ "$_FAIL_COUNT" -gt 0 ]; then
        echo "  Failed tests:"
        for name in "${_TEST_NAMES_FAILED[@]}"; do
            echo "    - $name"
        done
        exit 1
    fi
    exit 0
}

# Export counts so they can be aggregated across scenario files.
export_counts() {
    echo "$_PASS_COUNT $_FAIL_COUNT"
}
