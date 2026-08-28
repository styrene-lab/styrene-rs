#!/bin/sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(dirname "$script_dir")
root=${STYRENE_MOBILE_HUB_ROOT:-"$repo_root/target/mobile-integration/hub"}
binary=${STYRENE_MOBILE_HUB_BINARY:-"$repo_root/target/debug/styrened"}
config=${STYRENE_MOBILE_HUB_CONFIG:-"$repo_root/deploy/hub.toml"}
transport=${STYRENE_MOBILE_HUB_TRANSPORT:-"0.0.0.0:4242"}
rpc=${STYRENE_MOBILE_HUB_RPC:-"127.0.0.1:4243"}
announce_interval=${STYRENE_MOBILE_HUB_ANNOUNCE_INTERVAL:-15}
pid_file="$root/styrened.pid"
log_file="$root/styrened.log"

usage() {
    echo "usage: local-mobile-hub.sh <start|status|android-probe|logs|stop> [android-serial]" >&2
    exit 2
}

read_pid() {
    [ -f "$pid_file" ] || return 1
    pid=$(sed -n '1p' "$pid_file")
    case "$pid" in
        ''|*[!0-9]*) return 1 ;;
    esac
    printf '%s\n' "$pid"
}

is_running() {
    pid=$(read_pid) || return 1
    kill -0 "$pid" 2>/dev/null
}

transport_port=${transport##*:}
rpc_port=${rpc##*:}
rpc_url="http://$rpc"

ready() {
    is_running || return 1
    nc -z 127.0.0.1 "$transport_port" >/dev/null 2>&1 || return 1
    curl -fsS "$rpc_url/readyz" >/dev/null 2>&1 || return 1
    grep -Fq '[daemon] node role: hub' "$log_file" || return 1
    grep -Fq '[daemon] propagation store enabled (hub mode)' "$log_file" || return 1
}

status() {
    if ! ready; then
        echo "mobile hub is not ready" >&2
        [ -f "$log_file" ] && tail -n 30 "$log_file" >&2
        return 1
    fi
    pid=$(read_pid)
    delivery_hash=$(sed -n 's/^\[daemon\] delivery destination hash=//p' "$log_file" | tail -n 1)
    propagation_hash=$(sed -n 's/^\[daemon\] propagation control destination hash=//p' "$log_file" | tail -n 1)
    echo "mobile hub ready"
    echo "pid: $pid"
    echo "transport: $transport"
    echo "iOS Simulator: 127.0.0.1:$transport_port"
    echo "Android Emulator: 10.0.2.2:$transport_port"
    echo "delivery destination: ${delivery_hash:-unavailable}"
    echo "propagation control destination: ${propagation_hash:-unavailable}"
    echo "log: $log_file"
}

start() {
    [ -x "$binary" ] || {
        echo "mobile hub binary not found: $binary" >&2
        echo "run: cargo build -p styrened --bin styrened" >&2
        exit 1
    }
    [ -f "$config" ] || {
        echo "mobile hub config not found: $config" >&2
        exit 1
    }
    if is_running; then
        status
        return
    fi
    if nc -z 127.0.0.1 "$transport_port" >/dev/null 2>&1; then
        echo "transport port $transport_port is already in use" >&2
        exit 1
    fi
    if nc -z 127.0.0.1 "$rpc_port" >/dev/null 2>&1; then
        echo "RPC port $rpc_port is already in use" >&2
        exit 1
    fi
    mkdir -p "$root"
    rm -f "$pid_file"
    : > "$log_file"
    LXMF_DISPLAY_NAME="Local Simulator Hub" \
    STYRENED_DIAGNOSTICS=1 \
        "$binary" \
        --config "$config" \
        --db "$root/messages.db" \
        --identity "$root/identity" \
        --socket "$root/daemon.sock" \
        --transport "$transport" \
        --rpc "$rpc" \
        --announce-interval-secs "$announce_interval" \
        >> "$log_file" 2>&1 &
    pid=$!
    printf '%s\n' "$pid" > "$pid_file"

    attempts=0
    while [ "$attempts" -lt 100 ]; do
        if ready; then
            status
            return
        fi
        if ! kill -0 "$pid" 2>/dev/null; then
            echo "mobile hub exited during startup" >&2
            tail -n 50 "$log_file" >&2
            rm -f "$pid_file"
            exit 1
        fi
        attempts=$((attempts + 1))
        sleep 0.1
    done

    echo "mobile hub did not become ready" >&2
    tail -n 50 "$log_file" >&2
    kill "$pid" 2>/dev/null || true
    rm -f "$pid_file"
    exit 1
}

android_probe() {
    serial=${1:-emulator-5554}
    android_home=${ANDROID_HOME:-"$HOME/Library/Android/sdk"}
    adb="$android_home/platform-tools/adb"
    [ -x "$adb" ] || {
        echo "adb not found: $adb" >&2
        exit 1
    }
    ready
    "$adb" -s "$serial" shell toybox nc -z -w 2 10.0.2.2 "$transport_port"
    echo "Android emulator $serial can reach 10.0.2.2:$transport_port"
}

stop() {
    pid=$(read_pid) || {
        echo "mobile hub is not running"
        return
    }
    command_line=$(ps -p "$pid" -o command= 2>/dev/null || true)
    case "$command_line" in
        *styrened*"$root/messages.db"*) ;;
        *)
            echo "refusing to stop unexpected process $pid: $command_line" >&2
            exit 1
            ;;
    esac
    kill "$pid"
    attempts=0
    while kill -0 "$pid" 2>/dev/null && [ "$attempts" -lt 50 ]; do
        attempts=$((attempts + 1))
        sleep 0.1
    done
    if kill -0 "$pid" 2>/dev/null; then
        echo "mobile hub did not stop within five seconds" >&2
        exit 1
    fi
    rm -f "$pid_file"
    echo "mobile hub stopped"
}

case ${1:-} in
    start) start ;;
    status) status ;;
    android-probe) android_probe "${2:-}" ;;
    logs) [ -f "$log_file" ] && tail -n 100 "$log_file" ;;
    stop) stop ;;
    *) usage ;;
esac
