#!/usr/bin/env bash
# Focused bidirectional cross-network delivery scenario.

source "${HARNESS_ROOT:-/harness}/harness.sh"

MSG_TIMEOUT=${STYRENE_CROSS_NETWORK_TIMEOUT:-90}
BATCH_COUNT=${STYRENE_CROSS_NETWORK_BATCH_COUNT:-100}
RUN_MARKER=${STYRENE_MESH_RUN_ID:-cross-network-$$}

if ! [[ "$BATCH_COUNT" =~ ^[1-9][0-9]*$ ]] || [ "$BATCH_COUNT" -gt 100 ]; then
    echo "STYRENE_CROSS_NETWORK_BATCH_COUNT must be between 1 and 100" >&2
    exit 2
fi

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

run_batch_direction() {
    local label=$1 sender_socket=$2 receiver_socket=$3 source_identity=$4 destination=$5
    local run_tag prefix
    run_tag=$(printf '%s' "$RUN_MARKER" | cksum | awk '{print $1}')
    prefix="mesh-${label}-${run_tag}"
    local accepted=0 started_at finished_at elapsed output rc
    started_at=$(date +%s)

    for sequence in $(seq 1 "$BATCH_COUNT"); do
        local correlation
        correlation=$(printf '%s-%03d' "$prefix" "$sequence")
        output=$(styrene --socket "$sender_socket" send "$destination" "$correlation" 2>&1) && rc=0 || rc=$?
        if [ "$rc" -ne 0 ]; then
            fail "$label: batch send $sequence/$BATCH_COUNT accepted (exit $rc)"
            emit_correlation batch_send_failed "$correlation" "sequence=$sequence" "output=$(tr '\n' ' ' <<<"$output")"
            return
        fi
        accepted=$((accepted + 1))
    done
    pass "$label: all $BATCH_COUNT batch sends accepted"
    emit_correlation batch_accepted "$prefix" "count=$accepted"

    local messages="" observed=0
    local deadline=$((started_at + MSG_TIMEOUT))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        messages=$(styrene --socket "$receiver_socket" messages "$source_identity" --limit "$BATCH_COUNT" 2>&1) || messages=""
        observed=$(grep -cF -- "$prefix-" <<<"$messages" || true)
        [ "$observed" -ge "$BATCH_COUNT" ] && break
        sleep 2
    done
    finished_at=$(date +%s)
    elapsed=$((finished_at - started_at))

    local duplicates missing order_failures=0 previous=0 sequence
    duplicates=0
    missing=0
    for sequence in $(seq "$BATCH_COUNT" -1 1); do
        local correlation count position
        correlation=$(printf '%s-%03d' "$prefix" "$sequence")
        count=$(grep -cF -- "$correlation" <<<"$messages" || true)
        if [ "$count" -eq 0 ]; then
            missing=$((missing + 1))
        elif [ "$count" -gt 1 ]; then
            duplicates=$((duplicates + count - 1))
        fi
        position=$(grep -nF -- "$correlation" <<<"$messages" | head -1 | cut -d: -f1 || true)
        if [ -n "$position" ]; then
            if [ "$position" -le "$previous" ]; then order_failures=$((order_failures + 1)); fi
            previous=$position
        fi
    done

    if [ "$missing" -eq 0 ] && [ "$duplicates" -eq 0 ] && [ "$order_failures" -eq 0 ]; then
        pass "$label: $BATCH_COUNT messages are complete, unique, and ordered"
        emit_correlation batch_durable "$prefix" "count=$BATCH_COUNT" "duplicates=0" "order_failures=0" "elapsed_seconds=$elapsed"
    else
        fail "$label: batch integrity missing=$missing duplicates=$duplicates order_failures=$order_failures"
        emit_correlation batch_invalid "$prefix" "observed=$observed" "missing=$missing" "duplicates=$duplicates" "order_failures=$order_failures" "elapsed_seconds=$elapsed"
    fi
}

printf '  alpha identity=%s destination=%s\n' "$ALPHA_ID" "$ALPHA_DEST"
printf '  gamma identity=%s destination=%s\n' "$GAMMA_ID" "$GAMMA_DEST"

run_direction TCN-A2G "$ALPHA_SOCK" "$GAMMA_SOCK" "$ALPHA_ID" "$GAMMA_DEST"
run_direction TCN-G2A "$GAMMA_SOCK" "$ALPHA_SOCK" "$GAMMA_ID" "$ALPHA_DEST"
run_batch_direction TCN-BATCH-A2G "$ALPHA_SOCK" "$GAMMA_SOCK" "$ALPHA_ID" "$GAMMA_DEST"
run_batch_direction TCN-BATCH-G2A "$GAMMA_SOCK" "$ALPHA_SOCK" "$GAMMA_ID" "$ALPHA_DEST"

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
[ "$_FAIL_COUNT" -eq 0 ]
