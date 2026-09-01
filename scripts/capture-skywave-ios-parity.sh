#!/usr/bin/env bash
set -euo pipefail

readonly BUNDLE_ID="co.horsfalldesign.skywave"
readonly ACTION="${1:-prepare}"
readonly LABEL="${2:-capture}"
readonly RUN_ROOT="${SKYWAVE_CAPTURE_ROOT:-target/mobile-integration/skywave-ios}"
readonly RUN_ID="${SKYWAVE_CAPTURE_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
readonly RUN_DIR="${RUN_ROOT}/${RUN_ID}"

require() {
    command -v "$1" >/dev/null 2>&1 || {
        printf 'missing required command: %s\n' "$1" >&2
        exit 1
    }
}

require_env() {
    if [[ -z "${!1:-}" ]]; then
        printf 'set %s for the paired iPhone; identifiers must not be committed\n' "$1" >&2
        exit 1
    fi
}

prepare() {
    mkdir -p "$RUN_DIR"
    xcrun devicectl device info apps \
        --device "$SKYWAVE_COREDEVICE_ID" \
        --bundle-id "$BUNDLE_ID" \
        --columns '*' \
        --json-output "$RUN_DIR/app-info.raw.json" >/dev/null
    xcrun devicectl device info details \
        --device "$SKYWAVE_COREDEVICE_ID" \
        --json-output "$RUN_DIR/device-info.raw.json" >/dev/null

    local app_count
    app_count="$(jq '.result.apps | length' "$RUN_DIR/app-info.raw.json")"
    if [[ "$app_count" != "1" ]]; then
        printf 'expected one installed %s application, found %s\n' "$BUNDLE_ID" "$app_count" >&2
        exit 1
    fi

    jq -n \
        --arg collected_on "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
        --arg pymobiledevice3 "$(pymobiledevice3 version)" \
        --slurpfile app "$RUN_DIR/app-info.raw.json" \
        --slurpfile device "$RUN_DIR/device-info.raw.json" \
        '{
            corpus: "styrene-mobile-application-parity-v1",
            reference_id: "skywave-1.0-build-9-ios-beta",
            evidence_status: "capture_pending_review",
            collected_on: $collected_on,
            connection: "native_remotexpc_local_network",
            tools: {pymobiledevice3: $pymobiledevice3},
            application: {
                name: $app[0].result.apps[0].name,
                bundle_identifier: $app[0].result.apps[0].bundleIdentifier,
                version: $app[0].result.apps[0].version,
                build: $app[0].result.apps[0].bundleVersion
            },
            target: {
                class: "physical_ios",
                model: $device[0].result.hardwareProperties.marketingName,
                os: $device[0].result.deviceProperties.osVersionNumber,
                developer_mode: $device[0].result.deviceProperties.developerModeStatus,
                transport: $device[0].result.connectionProperties.transportType
            },
            privacy: {
                raw_artifacts_may_contain_device_identifiers: true,
                commit_raw_artifacts: false,
                review_and_redact_before_summary: true
            },
            unexecuted_scenarios: [],
            notes: []
        }' >"$RUN_DIR/manifest.json"
    printf '%s\n' "$RUN_DIR"
}

assert_running() {
    local pid
    pid="$(pymobiledevice3 developer dvt process-id-for-bundle-id "$BUNDLE_ID" \
        --native --udid "$SKYWAVE_DEVICE_UDID")"
    if [[ ! "$pid" =~ ^[1-9][0-9]*$ ]]; then
        printf 'Skywave is not running; unlock the iPhone and open Skywave first\n' >&2
        exit 1
    fi
    printf '%s' "$pid"
}

snapshot() {
    prepare >/dev/null
    assert_running >/dev/null
    local output="$RUN_DIR/${LABEL}.png"
    pymobiledevice3 developer dvt screenshot "$output" \
        --native --udid "$SKYWAVE_DEVICE_UDID"
    shasum -a 256 "$output" >"$output.sha256"
    printf '%s\n' "$output"
}

logs() {
    prepare >/dev/null
    local pid
    pid="$(assert_running)"
    local seconds="${SKYWAVE_LOG_SECONDS:-30}"
    local output="$RUN_DIR/${LABEL}.oslog.ndjson"
    local logger_pid

    pymobiledevice3 developer dvt oslog --pid "$pid" --format json \
        --native --udid "$SKYWAVE_DEVICE_UDID" >"$output" &
    logger_pid=$!
    trap 'kill -INT "$logger_pid" 2>/dev/null || true' EXIT INT TERM
    sleep "$seconds"
    kill -INT "$logger_pid" 2>/dev/null || true
    for _ in {1..10}; do
        if ! kill -0 "$logger_pid" 2>/dev/null; then
            break
        fi
        sleep 0.1
    done
    if kill -0 "$logger_pid" 2>/dev/null; then
        kill -TERM "$logger_pid" 2>/dev/null || true
    fi
    wait "$logger_pid" 2>/dev/null || true
    trap - EXIT INT TERM
    shasum -a 256 "$output" >"$output.sha256"
    printf '%s\n' "$output"
}

require xcrun
require pymobiledevice3
require jq
require shasum
require_env SKYWAVE_COREDEVICE_ID
require_env SKYWAVE_DEVICE_UDID

case "$ACTION" in
    prepare) prepare ;;
    snapshot) snapshot ;;
    logs) logs ;;
    *)
        printf 'usage: %s {prepare|snapshot|logs} [artifact-label]\n' "$0" >&2
        exit 2
        ;;
esac
