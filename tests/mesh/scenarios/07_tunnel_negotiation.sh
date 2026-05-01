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

source /harness/harness.sh

echo "  Suite: Tunnel Negotiation"

NEGOTIATION_TIMEOUT=30

# Get node identity hashes for addressing
ALPHA_DEST=$(styrene --socket "$ALPHA_SOCK" identity 2>&1 | grep "lxmf" | awk '{print $2}')
BETA_DEST=$(styrene --socket "$BETA_SOCK" identity 2>&1 | grep "lxmf" | awk '{print $2}')

echo "  alpha lxmf: ${ALPHA_DEST:-UNKNOWN}"
echo "  beta lxmf:  ${BETA_DEST:-UNKNOWN}"

# --- T25: Verify tunnel handler is registered ---
# Check that the daemon exposes tunnel-related status or config.
# Try `styrene status` output for protocol handlers, or check daemon logs
# for handler registration.
OUTPUT=$(styrene --socket "$ALPHA_SOCK" status 2>&1) && RC=0 || RC=$?
if [ "$RC" -eq 0 ]; then
    # The daemon is running; check if tunnel handler info appears in status
    # or if a tunnel subcommand exists
    TUNNEL_OUTPUT=$(styrene --socket "$ALPHA_SOCK" tunnel status 2>&1) && TUNNEL_RC=0 || TUNNEL_RC=$?
    if [ "$TUNNEL_RC" -eq 0 ]; then
        pass "T25: tunnel handler is registered (tunnel status responded)"
    elif echo "$TUNNEL_OUTPUT" | grep -qi "no active tunnel\|tunnel.*registered\|not connected"; then
        # Handler exists but no active tunnels — that's expected
        pass "T25: tunnel handler is registered (no active tunnels)"
    elif echo "$OUTPUT" | grep -qi "tunnel\|protocol.*handler"; then
        pass "T25: tunnel handler is registered (visible in node status)"
    else
        # Tunnel subcommand not available — fail explicitly, do not mask
        fail "T25: tunnel handler is not registered (tunnel subcommand not available)"
        echo "    tunnel status output: $TUNNEL_OUTPUT"
        echo "    node status output: $OUTPUT"
    fi
else
    fail "T25: tunnel handler is registered (daemon unreachable, exit $RC)"
    echo "    output: $OUTPUT"
fi

# --- T26: Send a tunnel protocol message and check handling ---
# Send a message that mimics a TUNNEL_OFFER. If the tunnel protocol handler
# is registered, the daemon should log processing it. If not, the message
# is delivered but unhandled (which we can still verify).
if [ -z "$ALPHA_DEST" ] || [ -z "$BETA_DEST" ]; then
    echo "  SKIP: T26: LXMF destinations not available"
else
    # Wait for announce propagation
    sleep 5

    # Attempt to use the tunnel CLI if it exists
    OFFER_OUTPUT=$(styrene --socket "$ALPHA_SOCK" tunnel offer "$BETA_DEST" 2>&1) && OFFER_RC=0 || OFFER_RC=$?

    if [ "$OFFER_RC" -eq 0 ]; then
        # Tunnel offer command exists and succeeded
        pass "T26a: tunnel offer sent from alpha to beta"

        # Wait for beta to process the offer and potentially send an accept
        sleep 10
        ELAPSED=0
        NEGOTIATED=false
        while [ "$ELAPSED" -lt "$NEGOTIATION_TIMEOUT" ]; do
            # Check if beta has tunnel state referencing alpha
            BETA_TUNNEL=$(styrene --socket "$BETA_SOCK" tunnel status 2>&1) || true
            if echo "$BETA_TUNNEL" | grep -qi "alpha\|offer\|pending\|accept\|negotiat"; then
                NEGOTIATED=true
                break
            fi
            sleep 2
            ELAPSED=$((ELAPSED + 2))
        done

        if [ "$NEGOTIATED" = true ]; then
            pass "T26b: beta processed tunnel offer from alpha"
        else
            fail "T26b: beta processed tunnel offer from alpha (timeout ${NEGOTIATION_TIMEOUT}s)"
            echo "    beta tunnel status: $BETA_TUNNEL"
        fi
    else
        # Tunnel offer CLI does not exist — fail explicitly, do not mask with fallback
        fail "T26: tunnel offer command not available (exit $OFFER_RC)"
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
