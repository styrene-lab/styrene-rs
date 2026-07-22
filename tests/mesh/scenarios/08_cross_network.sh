#!/usr/bin/env bash
# Focused bidirectional cross-network delivery scenario.

source "${HARNESS_ROOT:-/harness}/harness.sh"

MSG_TIMEOUT=${STYRENE_CROSS_NETWORK_TIMEOUT:-90}
RUN_MARKER=${STYRENE_MESH_RUN_ID:-cross-network-$$}

identity_field() {
    local socket=$1 pattern=$2
    styrene --socket "$socket" identity 2>&1 | awk "$pattern"
}

ALPHA_DEST=$(identity_field "$ALPHA_SOCK" '/lxmf/ {print $2; exit}')
ALPHA_ID=$(identity_field "$ALPHA_SOCK" '/hash/ && !/dest|lxmf/ {print $2; exit}')
GAMMA_DEST=$(identity_field "$GAMMA_SOCK" '/lxmf/ {print $2; exit}')
GAMMA_ID=$(identity_field "$GAMMA_SOCK" '/hash/ && !/dest|lxmf/ {print $2; exit}')

run_direction() {
    local label=$1 sender_socket=$2 receiver_socket=$3 source_identity=$4 destination=$5
    local correlation="mesh-${label}-${RUN_MARKER}"
    local route

    if route=$(wait_for_route "$sender_socket" "$destination" 90); then
        pass "$label: destination route is ready"
        emit_correlation route_ready "$correlation" "$route"
    else
        fail "$label: destination route is ready"
        emit_correlation route_missing "$correlation" "destination=$destination"
        return
    fi

    local output rc
    output=$(styrene --socket "$sender_socket" send "$destination" "$correlation" 2>&1) && rc=0 || rc=$?
    if [ "$rc" -eq 0 ]; then
        pass "$label: send accepted"
        emit_correlation send_accepted "$correlation" "destination=$destination" "output=$(tr '\n' ' ' <<<"$output")"
    else
        fail "$label: send accepted (exit $rc)"
        emit_correlation send_failed "$correlation" "destination=$destination" "output=$(tr '\n' ' ' <<<"$output")"
        return
    fi

    local received
    if received=$(wait_for_message "$receiver_socket" "$source_identity" "$correlation" "$MSG_TIMEOUT"); then
        pass "$label: message durably observable"
        emit_correlation durable_insert "$correlation" "source_identity=$source_identity"
    else
        fail "$label: message durably observable (timeout ${MSG_TIMEOUT}s)"
        emit_correlation receive_timeout "$correlation" "source_identity=$source_identity" "destination=$destination"
        printf '    receiver messages: %s\n' "$received"
    fi
}

echo "  Suite: Focused cross-network delivery"
printf '  alpha identity=%s destination=%s\n' "$ALPHA_ID" "$ALPHA_DEST"
printf '  gamma identity=%s destination=%s\n' "$GAMMA_ID" "$GAMMA_DEST"

run_direction TCN-A2G "$ALPHA_SOCK" "$GAMMA_SOCK" "$ALPHA_ID" "$GAMMA_DEST"
run_direction TCN-G2A "$GAMMA_SOCK" "$ALPHA_SOCK" "$GAMMA_ID" "$ALPHA_DEST"

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
[ "$_FAIL_COUNT" -eq 0 ]
