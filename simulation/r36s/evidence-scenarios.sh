#!/bin/sh
set -eu

state=/state/evidence
root="$state/doctor"
ghost="$state/ghost"
mkdir -p "$state"

fail() {
    echo "evidence failure: $*" >&2
    exit 1
}

record_identity() {
    sha256sum "$root/config/identity" | cut -d' ' -f1
}

# Persistent first run must create complete, parseable state.
styrene doctor --root "$root"
[ -f "$root/config/setup_complete" ] || fail "setup marker missing"
[ "$(wc -c < "$root/config/identity")" -eq 64 ] || fail "identity is not 64 bytes"
identity_before="$(record_identity)"

# Same-artifact reinstall must preserve identity and operator-owned data.
printf '%s\n' 'preserve-me' > "$root/data/operator-sentinel"
styrene doctor --root "$root"
[ "$(record_identity)" = "$identity_before" ] || fail "identity changed on second doctor run"
[ "$(cat "$root/data/operator-sentinel")" = 'preserve-me' ] || fail "operator data changed"

# Private state must retain Unix permissions.
[ "$(stat -c '%a' "$root/config")" = 700 ] || fail "config directory mode is not 0700"
[ "$(stat -c '%a' "$root/data")" = 700 ] || fail "data directory mode is not 0700"
[ "$(stat -c '%a' "$root/config/identity")" = 600 ] || fail "identity mode is not 0600"
[ "$(stat -c '%a' "$root/config/setup_complete")" = 600 ] || fail "marker mode is not 0600"

# A corrupt committed identity must fail closed and invalidate completion.
printf corrupt > "$root/config/identity"
if styrene doctor --root "$root" >/tmp/corrupt.out 2>/tmp/corrupt.err; then
    fail "doctor accepted corrupt identity"
fi
[ ! -e "$root/config/setup_complete" ] || fail "stale completion marker survived failed repair"

# Ghost must reach readiness and remove all session-owned state.
styrene ghost-check --root "$ghost" --timeout 15
if find "$ghost" -mindepth 1 -print -quit | grep -q .; then
    fail "ghost state survived cleanup"
fi

echo "persistent_first_run=pass"
echo "persistent_reinstall=pass"
echo "private_permissions=pass"
echo "corruption_fail_closed=pass"
echo "ghost_cleanup=pass"
