#!/usr/bin/env bash
# Routed channel gate: two Rust nodes reach each other only through a pinned
# Python Reticulum transport instance, open a link and a reliable channel across
# it, and exchange echoed messages both ways. The pinned Python applications
# speak no channel protocol the daemon exposes, so this evidence is Rust to
# Rust across the pinned transport.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

PYTHON_BIN="${PYTHON_BIN:-python3}"
LOG_DIR="${LOG_DIR:-${REPO_ROOT}/target/interop/rust-channel-hop}"
REPORT_PATH="${REPORT_PATH:-${LOG_DIR}/report.json}"
TIMEOUT_SECS="${TIMEOUT_SECS:-45}"
LOG_LIMIT_BYTES="${LOG_LIMIT_BYTES:-2097152}"
CHANNEL_MESSAGES="${CHANNEL_MESSAGES:-6}"
CHANNEL_PAYLOAD_BYTES="${CHANNEL_PAYLOAD_BYTES:-400}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
export CARGO_TARGET_DIR
PROBE_BIN="${PROBE_BIN:-${CARGO_TARGET_DIR}/debug/styrene-channel-probe}"
SCENARIO="${SCENARIO:-routed_channel}"

PORT_SEED="${PORT_SEED:-$$}"
# The runner reserves a four-port block for the LXMF topology; the hop listens
# outside it.
PY_HOP_PORT="${PY_HOP_PORT:-$((41528 + (PORT_SEED % 2000)))}"

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --scenario)
        if [[ $# -lt 2 || -z "${2:-}" ]]; then
          echo "missing value for --scenario" >&2
          exit 2
        fi
        SCENARIO="$2"
        shift 2
        ;;
      --timeout)
        if [[ $# -lt 2 || -z "${2:-}" ]]; then
          echo "missing value for --timeout" >&2
          exit 2
        fi
        TIMEOUT_SECS="$2"
        shift 2
        ;;
      *)
        echo "unknown argument: $1" >&2
        echo "Usage: rust-channel-hop-smoke.sh [--scenario routed_channel] [--timeout SECONDS]" >&2
        exit 2
        ;;
    esac
  done
  if [[ "${SCENARIO}" != "routed_channel" ]]; then
    echo "unsupported scenario: ${SCENARIO}" >&2
    exit 2
  fi
}
parse_args "$@"

runner_milestone() {
  printf 'STYRENE_EVENT {"kind":"milestone","name":"%s","correlation_id":"%s"}\n' "$1" "${STYRENE_INTEROP_CORRELATION_ID:?}"
}

runner_assertion() {
  printf 'STYRENE_EVENT {"kind":"assertion","name":"%s","passed":true,"correlation_id":"%s"}\n' "$1" "${STYRENE_INTEROP_CORRELATION_ID:?}"
}

runner_artifact() {
  printf 'STYRENE_EVENT {"kind":"artifact","name":"%s","path":"%s","correlation_id":"%s"}\n' "$1" "$2" "${STYRENE_INTEROP_CORRELATION_ID:?}"
}

bounded_log() {
  local path="$1"
  local limit="$2"
  "${PYTHON_BIN}" -c '
import sys

path, limit = sys.argv[1], int(sys.argv[2])
written = 0
with open(path, "wb") as output:
    while chunk := sys.stdin.buffer.read1(65536):
        if written < limit:
            retained = chunk[:limit - written]
            output.write(retained)
            output.flush()
            written += len(retained)
' "${path}" "${limit}"
}

wait_for_file_pattern() {
  local file="$1"
  local pattern="$2"
  local timeout="$3"
  local start
  start="$(date +%s)"
  while true; do
    if [[ -f "${file}" ]] && grep -Eq "${pattern}" "${file}"; then
      return 0
    fi
    if (( "$(date +%s)" - start >= timeout )); then
      return 1
    fi
    sleep 1
  done
}

mkdir -p "${LOG_DIR}"
TMP_ROOT="$(mktemp -d "${LOG_DIR}/run.XXXXXX")"
PY_HOP_DIR="${TMP_ROOT}/python-hop"
PY_HOP_LOG="${TMP_ROOT}/python-hop.log"
PROBE_LOG="${TMP_ROOT}/channel-probe.log"
PROOF_PATH="${TMP_ROOT}/routed-channel-proof.json"
mkdir -p "${PY_HOP_DIR}"

cleanup_child() {
  local pid="$1"
  local deadline
  kill -TERM "${pid}" >/dev/null 2>&1 || true
  deadline=$((SECONDS + 2))
  while kill -0 "${pid}" >/dev/null 2>&1 && (( SECONDS < deadline )); do
    sleep 0.05
  done
  if kill -0 "${pid}" >/dev/null 2>&1; then
    kill -KILL "${pid}" >/dev/null 2>&1 || true
  fi
  wait "${pid}" 2>/dev/null || true
}

HOP_PID=""
cleanup() {
  local status=$?
  if [[ -n "${HOP_PID:-}" ]]; then
    cleanup_child "${HOP_PID}"
  fi
  if [[ ${status} -ne 0 ]]; then
    echo "[rust-channel-hop-smoke] failed" >&2
    echo "[rust-channel-hop-smoke] logs=${TMP_ROOT}" >&2
  fi
  runner_milestone "child-cleanup-complete"
}
trap cleanup EXIT

"${PYTHON_BIN}" - <<'PY' >/dev/null
import importlib.util
if importlib.util.find_spec("RNS") is None:
    raise SystemExit("missing Python module: RNS")
PY

runner_milestone "topology-configured"

cargo build --manifest-path "${REPO_ROOT}/Cargo.toml" -p styrene-e2e --bin styrene-channel-probe --quiet

# A pinned Python Reticulum instance with transport enabled and one server
# interface. Both Rust nodes connect to it, so every exchange crosses two hops.
cat > "${PY_HOP_DIR}/config" <<EOF
[reticulum]
  enable_transport = true
  share_instance = no
  discover_interfaces = false
  autoconnect_discovered_interfaces = 0

[logging]
  loglevel = 4

[interfaces]
  [[Rust Nodes]]
    type = TCPServerInterface
    enabled = yes
    listen_ip = 127.0.0.1
    listen_port = ${PY_HOP_PORT}
EOF

(
  PYTHONUNBUFFERED=1 exec "${PYTHON_BIN}" - <<'PY' "${PY_HOP_DIR}" "${PY_HOP_LOGLEVEL:-4}" > >(bounded_log "${PY_HOP_LOG}" "${LOG_LIMIT_BYTES}") 2>&1
import sys
import time

import RNS

RNS.Reticulum(configdir=sys.argv[1], loglevel=int(sys.argv[2]))
print("transport hop running", flush=True)
while True:
    time.sleep(1)
PY
) &
HOP_PID=$!
if ! wait_for_file_pattern "${PY_HOP_LOG}" "transport hop running" "${TIMEOUT_SECS}"; then
  echo "Python transport hop did not start" >&2
  exit 1
fi
runner_milestone "transport-hop-ready"

HOP_TRANSPORT_IDENTITY="$("${PYTHON_BIN}" - <<'PY' "${PY_HOP_DIR}/storage/transport_identity"
import sys

import RNS

identity = RNS.Identity.from_file(sys.argv[1])
print(RNS.hexrep(identity.hash, delimit=False).lower() if identity else "")
PY
)"

if ! "${PROBE_BIN}" \
    --hop "127.0.0.1:${PY_HOP_PORT}" \
    --messages "${CHANNEL_MESSAGES}" \
    --payload "${CHANNEL_PAYLOAD_BYTES}" \
    --timeout "${TIMEOUT_SECS}" \
    --proof "${PROOF_PATH}" \
    --correlation "${STYRENE_INTEROP_CORRELATION_ID}" > "${PROBE_LOG}" 2>&1; then
  echo "Rust channel probe failed" >&2
  tail -20 "${PROBE_LOG}" >&2 || true
  exit 1
fi
runner_milestone "rust-nodes-ready"

"${PYTHON_BIN}" - <<'PY' "${PROOF_PATH}" "${HOP_TRANSPORT_IDENTITY}" "${CHANNEL_MESSAGES}" "${STYRENE_INTEROP_CORRELATION_ID}"
import json
import sys
from pathlib import Path

proof_path, hop_identity, messages, correlation_id = sys.argv[1:5]
proof = json.loads(Path(proof_path).read_text(encoding="utf-8"))
messages = int(messages)
problems = []
if proof.get("status") != "passed" or proof.get("correlation_id") != correlation_id:
    problems.append(f"probe status {proof.get('status')!r}")
route = proof.get("route") or {}
for leg in ("a_to_b", "b_to_a"):
    entry = route.get(leg) or {}
    if entry.get("hops") != 2:
        problems.append(f"{leg} hops {entry.get('hops')!r}")
    if hop_identity and entry.get("next_hop") != hop_identity:
        problems.append(f"{leg} next hop {entry.get('next_hop')!r} is not the Python transport {hop_identity}")
channel = proof.get("channel") or {}
for key in ("sent", "delivered_to_b", "received_by_b", "echoed_to_a", "echoes_delivered_to_a"):
    if channel.get(key) != messages:
        problems.append(f"channel {key} {channel.get(key)!r} != {messages}")
if channel.get("integrity_verified") is not True:
    problems.append("channel payloads did not round-trip intact")
if not (proof.get("link") or {}).get("id"):
    problems.append("no link id recorded")
if problems:
    raise SystemExit("routed channel evidence failed: " + "; ".join(problems))
proof["hop_transport_identity"] = hop_identity
Path(proof_path).write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
runner_milestone "routed-path-verified"
runner_milestone "link-active"
runner_milestone "channel-delivered"
runner_milestone "channel-echoed"
runner_assertion "rust-to-rust-routed-channel"

"${PYTHON_BIN}" - <<'PY' "${REPORT_PATH}" "${TMP_ROOT}" "${PY_HOP_LOG}" "${PROBE_LOG}" "${PROOF_PATH}" "${HOP_TRANSPORT_IDENTITY}" "${SCENARIO}" "${STYRENE_INTEROP_CORRELATION_ID}"
import json
import sys
from pathlib import Path

report_path, tmp_root, hop_log, probe_log, proof_path, hop_identity, scenario, correlation_id = sys.argv[1:9]
proof = json.loads(Path(proof_path).read_text(encoding="utf-8"))
report = {
    "status": "pass",
    "scenario": scenario,
    "correlation_id": correlation_id,
    "proof": {
        "hop_transport_identity": hop_identity,
        "route": proof.get("route"),
        "link_id": (proof.get("link") or {}).get("id"),
        "channel": proof.get("channel"),
    },
    "logs": {"tmp_root": tmp_root, "python_hop": hop_log, "channel_probe": probe_log},
}
with open(report_path, "w", encoding="utf-8") as handle:
    json.dump(report, handle, indent=2)
    handle.write("\n")
PY

runner_artifact "scenario-report" "${REPORT_PATH}"
runner_artifact "routed-channel-proof" "${PROOF_PATH}"
runner_artifact "channel-probe-log" "${PROBE_LOG}"
runner_artifact "python-hop-log" "${PY_HOP_LOG}"
echo "[rust-channel-hop-smoke] pass"
echo "[rust-channel-hop-smoke] report=${REPORT_PATH}"
echo "[rust-channel-hop-smoke] logs=${TMP_ROOT}"
