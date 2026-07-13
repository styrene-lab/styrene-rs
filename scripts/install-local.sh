#!/bin/sh
set -eu

usage() {
    echo "usage: install-local.sh <target-dir> <binary>..." >&2
    exit 2
}

[ "$#" -ge 2 ] || usage
target_dir=$1
shift

mkdir -p "$target_dir"

for source in "$@"; do
    [ -f "$source" ] || {
        echo "install: built binary not found: $source" >&2
        exit 1
    }
    name=${source##*/}
    temporary="$target_dir/.${name}.new.$$"
    trap 'rm -f "$temporary"' EXIT HUP INT TERM
    cp "$source" "$temporary"
    chmod 755 "$temporary"
    mv -f "$temporary" "$target_dir/$name"
    trap - EXIT HUP INT TERM
    echo "installed $target_dir/$name"
done
