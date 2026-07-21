#!/usr/bin/env bash
# T25-T28: Tunnel negotiation tests.
#
# Phase 5 tunnel negotiation happens over LXMF messaging, dispatched by the
# ProtocolService. These tests verify the negotiation handshake without
# requiring WireGuard kernel modules (NET_ADMIN capability).
#
# T25: Verify tunnel protocol handler is registered in the daemon
# T26: Send a tunnel offer message and verify the handler processes it
# T27: (SKIP) Actual WireGuard tunnel establishment — requires NET_ADMIN
# T28: (SKIP) Tunnel data plane connectivity — requires NET_ADMIN + wireguard

source "${HARNESS_ROOT:-/harness}/harness.sh"

echo "  Suite: Tunnel Negotiation"

NEGOTIATION_TIMEOUT=30

# Get node identity hashes for tunnel control. Tunnel IPC accepts identity
# hashes, not derived LXMF delivery destination hashes.
ALPHA_ID=$(styrene --socket "$ALPHA_SOCK" identity 2>&1 | awk '/hash/ && !/dest|lxmf/ {print $2; exit}')
BETA_ID=$(styrene --socket "$BETA_SOCK" identity 2>&1 | awk '/hash/ && !/dest|lxmf/ {print $2; exit}')

printf '  alpha identity: %s\n' "${ALPHA_ID:-UNKNOWN}"
printf '  beta identity:  %s\n' "${BETA_ID:-UNKNOWN}"

# --- T25: Verify tunnel IPC surface is registered ---
LIST_OUTPUT=$(styrene --socket "$ALPHA_SOCK" tunnel list 2>&1) && LIST_RC=0 || LIST_RC=$?
if [ "$LIST_RC" -eq 0 ]; then
    pass "T25a: tunnel list IPC responds"
else
    fail "T25a: tunnel list IPC responds (exit $LIST_RC)"
    echo "    output: $LIST_OUTPUT"
fi

if [ -z "$BETA_ID" ]; then
    echo "  SKIP: T25b: beta identity not available"
else
    STATUS_OUTPUT=$(styrene --socket "$ALPHA_SOCK" tunnel status "$BETA_ID" 2>&1) && STATUS_RC=0 || STATUS_RC=$?
    if [ "$STATUS_RC" -eq 0 ] && grep -qiE 'styrene tunnel status|status|not found' <<<"$STATUS_OUTPUT"; then
        pass "T25b: tunnel status IPC accepts peer identity"
    else
        fail "T25b: tunnel status IPC accepts peer identity (exit $STATUS_RC)"
        echo "    output: $STATUS_OUTPUT"
    fi
fi

# --- T26: Send a tunnel protocol message and check handling ---
# Send a message that mimics a TUNNEL_OFFER. If the tunnel protocol handler
# is registered, the daemon should log processing it. If not, the message
# is delivered but unhandled (which we can still verify).
if [ -z "$ALPHA_ID" ] || [ -z "$BETA_ID" ]; then
    echo "  SKIP: T26: identity hashes not available"
else
    # Wait for announce propagation
    sleep 5

    # Attempt to use the tunnel CLI if it exists
    OFFER_OUTPUT=$(styrene --socket "$ALPHA_SOCK" tunnel offer "$BETA_ID" 2>&1) && OFFER_RC=0 || OFFER_RC=$?

    if [ "$OFFER_RC" -eq 0 ]; then
        pass "T26a: tunnel offer operation accepted"
        echo "    output: $OFFER_OUTPUT"

        ELAPSED=0
        OBSERVED=false
        while [ "$ELAPSED" -lt "$NEGOTIATION_TIMEOUT" ]; do
            ALPHA_TUNNEL=$(styrene --socket "$ALPHA_SOCK" tunnel status "$BETA_ID" 2>&1) || true
            if grep -qiE 'queued|sending_offer|offer_sent|established|failed' <<<"$ALPHA_TUNNEL"; then
                OBSERVED=true
                break
            fi
            sleep 2
            ELAPSED=$((ELAPSED + 2))
        done

        if [ "$OBSERVED" = true ]; then
            pass "T26b: tunnel operation state is observable"
            echo "    alpha tunnel status: $ALPHA_TUNNEL"
        else
            fail "T26b: tunnel operation state is observable (timeout ${NEGOTIATION_TIMEOUT}s)"
            echo "    alpha tunnel status: $ALPHA_TUNNEL"
        fi
    else
        fail "T26: tunnel offer failed (exit $OFFER_RC)"
        echo "    peer identity: $BETA_ID"
        echo "    output: $OFFER_OUTPUT"
    fi
fi

# --- T27: WireGuard tunnel establishment (requires NET_ADMIN) ---
echo "  SKIP: T27: WireGuard tunnel establishment requires NET_ADMIN capability"
echo "  NOTE: Add 'cap_add: [NET_ADMIN]' and wireguard-tools to test actual tunnels"

# --- T28: Tunnel data plane (requires NET_ADMIN + wireguard kernel module) ---
echo "  SKIP: T28: Tunnel data plane test requires NET_ADMIN + wireguard kernel module"
echo "  NOTE: Future: ping across tunnel, measure throughput, verify encryption"

echo "RESULTS: $_PASS_COUNT $_FAIL_COUNT"
