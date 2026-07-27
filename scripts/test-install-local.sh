#!/bin/sh
set -eu

root=${TMPDIR:-/tmp}/styrene-install-test.$$
bin="$root/bin"
src="$root/src"
fake="$root/fake"
state="$root/state"
trap 'rm -rf "$root"' EXIT HUP INT TERM
mkdir -p "$bin" "$src" "$fake" "$state"

make_executable() {
    path=$1
    output=$2
    cat >"$path" <<EOF
#!/bin/sh
printf '%s\n' '$output'
EOF
    chmod 755 "$path"
}

make_executable "$bin/one" old-one
make_executable "$bin/two" old-two
make_executable "$src/one" new-one
make_executable "$src/two" new-two
printf 'operator state\n' >"$state/config"
state_before=$(cksum "$state/config")

# Source preflight is all-or-nothing.
chmod 644 "$src/two"
if sh scripts/install-local.sh "$bin" "$src/one" "$src/two" 2>"$root/preflight.log"; then
    echo "test-install: invalid source unexpectedly installed" >&2
    exit 1
fi
[ "$("$bin/one")" = old-one ]
[ "$("$bin/two")" = old-two ]
chmod 755 "$src/two"

# Inject a failure during the second rename. The complete old set must return.
cat >"$fake/mv" <<'EOF'
#!/bin/sh
case "$1:$2" in
    */.two.new.*:*/two) exit 73 ;;
esac
exec /bin/mv "$@"
EOF
chmod 755 "$fake/mv"
if PATH="$fake:/usr/bin:/bin" /bin/sh scripts/install-local.sh "$bin" "$src/one" "$src/two" \
    >"$root/rollback.out" 2>"$root/rollback.err"; then
    echo "test-install: injected replacement failure unexpectedly succeeded" >&2
    exit 1
fi
[ "$("$bin/one")" = old-one ]
[ "$("$bin/two")" = old-two ]
grep -q 'previous installation restored' "$root/rollback.err"

# A successful run replaces the complete executable set and leaves no transaction files.
sh scripts/install-local.sh "$bin" "$src/one" "$src/two" >/dev/null
[ "$("$bin/one")" = new-one ]
[ "$("$bin/two")" = new-two ]
[ "$(cksum "$state/config")" = "$state_before" ]

set -- "$bin"/.*.new.* "$bin"/.*.old.*
for path in "$@"; do
    [ ! -e "$path" ] || {
        echo "test-install: transaction file remained: $path" >&2
        exit 1
    }
done

printf 'local installer contract: ok\n'
