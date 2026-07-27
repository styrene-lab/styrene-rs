#!/bin/sh
set -eu

# The replacement phase is process-transactional: every executable is staged
# before the first live path changes, and any replacement or smoke-check
# failure restores the complete previous set. A machine crash can still leave
# hidden .new/.old siblings; a later install safely replaces its own PID-scoped
# siblings but does not guess whether unrelated stale backups should be live.

usage() {
    echo "usage: install-local.sh <target-dir> <binary>..." >&2
    exit 2
}

[ "$#" -ge 2 ] || usage
target_dir=$1
shift

umask 022
mkdir -p "$target_dir"
[ -d "$target_dir" ] || {
    echo "install: target is not a directory: $target_dir" >&2
    exit 1
}
[ -w "$target_dir" ] || {
    echo "install: target is not writable: $target_dir" >&2
    exit 1
}

sources=""
names=""
for source in "$@"; do
    [ -f "$source" ] || {
        echo "install: built binary not found: $source" >&2
        exit 1
    }
    [ -x "$source" ] || {
        echo "install: built artifact is not executable: $source" >&2
        exit 1
    }
    name=${source##*/}
    case " $names " in
        *" $name "*)
            echo "install: duplicate binary name: $name" >&2
            exit 1
            ;;
    esac
    sources="$sources $source"
    names="$names $name"
done

staged=""
backups=""
committed=""
current_name=""
current_backup=""
current_was_installed=0
cleanup() {
    for path in $staged $backups; do
        rm -f "$path"
    done
}
rollback() {
    # The current destination may already have been moved aside even though its
    # replacement never committed. Restore it before rolling back prior names.
    if [ -n "$current_name" ] && [ "$current_was_installed" -eq 1 ]; then
        if [ -e "$current_backup" ]; then
            mv -f "$current_backup" "$target_dir/$current_name" || true
        else
            rm -f "$target_dir/$current_name"
        fi
    fi
    for name in $committed; do
        destination="$target_dir/$name"
        backup="$target_dir/.${name}.old.$$"
        if [ -e "$backup" ]; then
            mv -f "$backup" "$destination" || true
        else
            rm -f "$destination"
        fi
    done
    cleanup
}
trap 'rollback; exit 1' HUP INT TERM
trap cleanup EXIT

for source in $sources; do
    name=${source##*/}
    temporary="$target_dir/.${name}.new.$$"
    cp "$source" "$temporary"
    chmod 755 "$temporary"
    staged="$staged $temporary"
done

for name in $names; do
    destination="$target_dir/$name"
    backup="$target_dir/.${name}.old.$$"
    current_name=$name
    current_backup=$backup
    current_was_installed=0
    if [ -e "$destination" ]; then
        mv "$destination" "$backup"
        backups="$backups $backup"
    fi
    current_was_installed=1
    if ! mv "$target_dir/.${name}.new.$$" "$destination"; then
        rollback
        echo "install: failed to replace $destination; previous installation restored" >&2
        exit 1
    fi
    committed="$committed $name"
    current_name=""
    current_backup=""
    current_was_installed=0
done

for name in $names; do
    case "$name" in
        styrene|styrened)
            "$target_dir/$name" --version >/dev/null 2>&1 || {
                rollback
                echo "install: $name failed its --version smoke check; previous installation restored" >&2
                exit 1
            }
            ;;
    esac
done

for path in $backups; do
    rm -f "$path"
done
backups=""
staged=""
committed=""
trap - HUP INT TERM

for name in $names; do
    echo "installed $target_dir/$name"
done
