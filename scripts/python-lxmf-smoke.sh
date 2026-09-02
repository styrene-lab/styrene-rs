#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

PYTHON_BIN="${PYTHON_BIN:-python3}"
LOG_DIR="${LOG_DIR:-${REPO_ROOT}/target/interop/python-lxmd-rust-lxmd}"
REPORT_PATH="${REPORT_PATH:-${LOG_DIR}/report.json}"
TIMEOUT_SECS="${TIMEOUT_SECS:-45}"
SENDER_WAIT_SECS="${SENDER_WAIT_SECS:-240}"
REMOTE_STATUS_TIMEOUT_SECS="${REMOTE_STATUS_TIMEOUT_SECS:-10}"
PROPAGATION_TARGET_COST="${PROPAGATION_TARGET_COST:-16}"
# Queue bound for the capacity scenario: smaller than any stamped LXMF message.
CAPACITY_QUEUE_MAX_BYTES="${CAPACITY_QUEUE_MAX_BYTES:-64}"
# Queue expiry for the expiry scenario: long enough to persist the item, short
# enough to expire it inside one run. Eight seconds raced the persisted check
# under CI load.
EXPIRY_SECS="${EXPIRY_SECS:-20}"
LOG_LIMIT_BYTES="${LOG_LIMIT_BYTES:-2097152}"
SCENARIO="${SCENARIO:-direct}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
export CARGO_TARGET_DIR
STYRENED_BIN="${STYRENED_BIN:-${CARGO_TARGET_DIR}/debug/styrened}"
STYRENE_CLI_BIN="${STYRENE_CLI_BIN:-${CARGO_TARGET_DIR}/debug/styrene}"

PORT_SEED="${PORT_SEED:-$$}"
RUST_RPC_PORT="${RUST_RPC_PORT:-$((4243 + (PORT_SEED % 2000)))}"
RUST_TRANSPORT_PORT="${RUST_TRANSPORT_PORT:-$((37429 + (PORT_SEED % 2000)))}"
RUST_RPC_ADDR="${RUST_RPC_ADDR:-127.0.0.1:${RUST_RPC_PORT}}"
RUST_TRANSPORT_ADDR="${RUST_TRANSPORT_ADDR:-127.0.0.1:${RUST_TRANSPORT_PORT}}"
RUST_TRANSPORT_HOST="${RUST_TRANSPORT_ADDR%:*}"
RUST_TRANSPORT_PORT="${RUST_TRANSPORT_ADDR##*:}"

PY_SHARED_INSTANCE_PORT="${PY_SHARED_INSTANCE_PORT:-$((39428 + (PORT_SEED % 2000)))}"
PY_INSTANCE_CONTROL_PORT="${PY_INSTANCE_CONTROL_PORT:-$((PY_SHARED_INSTANCE_PORT + 1))}"
# Routed scenarios make the Python lxmd instance a transport hop: it listens
# here and the Python sender reaches the Rust node only through it.
PY_HOP_PORT="${PY_HOP_PORT:-$((PY_SHARED_INSTANCE_PORT + 2))}"

usage() {
  cat <<'EOF'
Usage: python-lxmd-rust-lxmd-smoke.sh [--scenario direct|direct_resource|opportunistic|propagated_resource_lxm|propagated_retrieval|propagated_capacity|propagated_expiry|routed_direct|routed_direct_resource] [--timeout SECONDS]
EOF
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --scenario)
        if [[ $# -lt 2 || -z "${2:-}" ]]; then
          echo "missing value for --scenario" >&2
          usage >&2
          exit 2
        fi
        SCENARIO="$2"
        shift 2
        ;;
      --timeout)
        if [[ $# -lt 2 || -z "${2:-}" ]]; then
          echo "missing value for --timeout" >&2
          usage >&2
          exit 2
        fi
        TIMEOUT_SECS="$2"
        shift 2
        ;;
      --help|-h)
        usage
        exit 0
        ;;
      *)
        echo "unknown argument: $1" >&2
        usage >&2
        exit 2
        ;;
    esac
  done

  case "${SCENARIO}" in
    direct|direct_resource|opportunistic|propagated_resource_lxm|propagated_retrieval|propagated_capacity|propagated_expiry|routed_direct|routed_direct_resource) ;;
    *)
      echo "unsupported scenario: ${SCENARIO}" >&2
      usage >&2
      exit 2
      ;;
  esac
}

require_python_modules() {
  "${PYTHON_BIN}" - <<'PY' >/dev/null
import importlib.util
for module in ("RNS", "LXMF"):
    if importlib.util.find_spec(module) is None:
        raise SystemExit(f"missing Python module: {module}")
PY
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

extract_hash() {
  local file="$1"
  local marker="$2"
  "${PYTHON_BIN}" - <<'PY' "${file}" "${marker}"
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
marker = sys.argv[2]
pattern = re.compile(r"([0-9a-f]{32})", re.IGNORECASE)

for line in path.read_text(encoding="utf-8", errors="ignore").splitlines():
    if marker in line:
        match = pattern.search(line)
        if match:
            print(match.group(1).lower())
            raise SystemExit(0)

raise SystemExit(1)
PY
}

destination_hash_from_identity() {
  local identity_path="$1"
  local aspect_one="$2"
  local aspect_two="$3"
  local aspect_three="${4:-}"
  "${PYTHON_BIN}" - <<'PY' "${identity_path}" "${aspect_one}" "${aspect_two}" "${aspect_three}"
import os
import sys
import tempfile

import RNS

identity_path, aspect_one, aspect_two, aspect_three = sys.argv[1:5]
cfg = tempfile.mkdtemp(prefix="rns-hash-")
with open(os.path.join(cfg, "config"), "w", encoding="utf-8") as handle:
    handle.write(
        "[reticulum]\n"
        "share_instance = no\n"
        "enable_transport = no\n"
        "discover_interfaces = false\n"
        "autoconnect_discovered_interfaces = 0\n"
    )

RNS.Reticulum(configdir=cfg, loglevel=0)
identity = RNS.Identity.from_file(identity_path)
if identity is None:
    raise SystemExit(f"failed to load identity from {identity_path}")

aspects = [aspect_one, aspect_two]
if aspect_three:
    aspects.append(aspect_three)

destination = RNS.Destination(identity, RNS.Destination.IN, RNS.Destination.SINGLE, *aspects)
print(RNS.hexrep(destination.hash, delimit=False).lower())
PY
}

identity_hash_from_file() {
  local identity_path="$1"
  "${PYTHON_BIN}" - <<'PY' "${identity_path}"
import os
import sys
import tempfile

import RNS

identity_path = sys.argv[1]
cfg = tempfile.mkdtemp(prefix="rns-ident-")
with open(os.path.join(cfg, "config"), "w", encoding="utf-8") as handle:
    handle.write(
        "[reticulum]\n"
        "share_instance = no\n"
        "enable_transport = no\n"
        "discover_interfaces = false\n"
        "autoconnect_discovered_interfaces = 0\n"
    )

RNS.Reticulum(configdir=cfg, loglevel=0)
identity = RNS.Identity.from_file(identity_path)
if identity is None:
    raise SystemExit(f"failed to load identity from {identity_path}")
print(RNS.hexrep(identity.hash, delimit=False).lower())
PY
}

assert_contains() {
  local file="$1"
  local pattern="$2"
  local description="$3"
  if ! grep -Eq "${pattern}" "${file}"; then
    echo "missing expected output: ${description}" >&2
    echo "looked for pattern '${pattern}' in ${file}" >&2
    return 1
  fi
}

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

mkdir -p "${LOG_DIR}"
TMP_ROOT="$(mktemp -d "${LOG_DIR}/run.XXXXXX")"

RUST_DIR="${TMP_ROOT}/rust-lxmd"
PY_DIR="${TMP_ROOT}/python-lxmd"
PY_RNS_DIR="${TMP_ROOT}/python-rns"
PY_SENDER_DIR="${TMP_ROOT}/python-sender"
PY_SENDER_RNS_DIR="${TMP_ROOT}/python-sender-rns"
HOOK_STATE_DIR="${TMP_ROOT}/hook-state"

RUST_LOG="${TMP_ROOT}/rust-lxmd.log"
PY_LOG="${TMP_ROOT}/python-lxmd.log"
PY_REMOTE_STATUS_LOG="${TMP_ROOT}/python-remote-status.log"
PY_SEND_LOG="${TMP_ROOT}/python-send.json"
PY_RECEIVE_LOG="${TMP_ROOT}/python-receive.json"
RUST_SEND_LOG="${TMP_ROOT}/rust-send.log"
RUST_OUTBOUND_PROOF_PATH="${TMP_ROOT}/rust-outbound-proof.json"
RUST_RESTART_LOG="${TMP_ROOT}/rust-lxmd-restart.log"
RUST_RETRIEVAL_PROOF_PATH="${TMP_ROOT}/rust-retrieval-proof.json"
PY_RETRIEVE_LOG="${TMP_ROOT}/python-retrieve.json"
RETRIEVE_SIGNAL="${TMP_ROOT}/retrieve.go"
RUST_PROPAGATION_SNAPSHOT_LOG="${TMP_ROOT}/rust-propagation-snapshot.json"
PY_REMOTE_STATS_LOG="${TMP_ROOT}/python-remote-stats.json"
RUST_CAPACITY_PROOF_PATH="${TMP_ROOT}/rust-capacity-proof.json"
ROUTED_PATH_PROOF_PATH="${TMP_ROOT}/routed-path-proof.json"
RUST_PATH_LOG="${TMP_ROOT}/rust-path.log"
RUST_EXPIRY_PROOF_PATH="${TMP_ROOT}/rust-expiry-proof.json"
# Unix socket paths are limited to about 100 bytes, so the IPC socket lives in
# a short-lived directory outside the run root. Cleanup removes it.
RUST_SOCKET_DIR="$(mktemp -d /tmp/styrene-smoke.XXXXXX)"
RUST_SOCKET="${RUST_SOCKET_DIR}/rust.sock"
HOOK_LOG="${HOOK_STATE_DIR}/hook.log"
DATASTORE_PROOF_PATH="${TMP_ROOT}/datastore-proof.json"

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
  wait "${pid}" >/dev/null 2>&1 || true
}

cleanup() {
  local status=$?
  if [[ -n "${PY_SENDER_PID:-}" ]]; then
    cleanup_child "${PY_SENDER_PID}"
  fi
  if [[ -n "${PY_PID:-}" ]]; then
    cleanup_child "${PY_PID}"
  fi
  if [[ -n "${RUST_PID:-}" ]]; then
    cleanup_child "${RUST_PID}"
  fi
  if [[ -n "${RUST_SOCKET_DIR:-}" ]]; then
    rm -rf "${RUST_SOCKET_DIR}"
  fi
  if [[ ${status} -ne 0 ]]; then
    echo "[python-lxmd-rust-lxmd-smoke] failed" >&2
    echo "[python-lxmd-rust-lxmd-smoke] logs=${TMP_ROOT}" >&2
  fi
  runner_milestone "child-cleanup-complete"
}
trap cleanup EXIT
parse_args "$@"

require_python_modules

mkdir -p "${RUST_DIR}" "${PY_DIR}" "${PY_RNS_DIR}" "${PY_SENDER_DIR}" "${PY_SENDER_RNS_DIR}" "${HOOK_STATE_DIR}"
runner_milestone "topology-configured"

PY_CONTROL_IDENTITY_HASH="$("${PYTHON_BIN}" - <<'PY' "${PY_DIR}/identity"
import sys
import RNS

path = sys.argv[1]
identity = RNS.Identity()
identity.to_file(path)
print(RNS.hexrep(identity.hash, delimit=False).lower())
PY
)"

RUST_DB="${RUST_DIR}/messages.db"
RUST_IDENTITY="${RUST_DIR}/identity"

ROUTED=false
case "${SCENARIO}" in
  routed_direct|routed_direct_resource) ROUTED=true ;;
esac

RUST_ROLE="full_node"
case "${SCENARIO}" in
  propagated_resource_lxm|propagated_retrieval|propagated_capacity|propagated_expiry) RUST_ROLE="hub" ;;
esac

cat > "${RUST_DIR}/config.toml" <<EOF
role = "${RUST_ROLE}"
EOF

RUST_CONTROL_IDENTITY_HASH=""

cat > "${PY_RNS_DIR}/config" <<EOF
[reticulum]
  enable_transport = true
  share_instance = yes
  shared_instance_port = ${PY_SHARED_INSTANCE_PORT}
  instance_control_port = ${PY_INSTANCE_CONTROL_PORT}
  discover_interfaces = false
  autoconnect_discovered_interfaces = 0

[logging]
  loglevel = 4

[interfaces]
  [[Rust LXMD]]
    type = TCPClientInterface
    enabled = yes
    target_host = ${RUST_TRANSPORT_HOST}
    target_port = ${RUST_TRANSPORT_PORT}
EOF
if [[ "${ROUTED}" == "true" ]]; then
  cat >> "${PY_RNS_DIR}/config" <<EOF

  [[Transport Hop]]
    type = TCPServerInterface
    enabled = yes
    listen_ip = 127.0.0.1
    listen_port = ${PY_HOP_PORT}
EOF
fi

SENDER_TARGET_HOST="${RUST_TRANSPORT_HOST}"
SENDER_TARGET_PORT="${RUST_TRANSPORT_PORT}"
if [[ "${ROUTED}" == "true" ]]; then
  SENDER_TARGET_HOST="127.0.0.1"
  SENDER_TARGET_PORT="${PY_HOP_PORT}"
fi

cat > "${PY_SENDER_RNS_DIR}/config" <<EOF
[reticulum]
  enable_transport = true
  share_instance = no
  discover_interfaces = false
  autoconnect_discovered_interfaces = 0

[logging]
  loglevel = 4

[interfaces]
  [[Rust LXMD Sender]]
    type = TCPClientInterface
    enabled = yes
    target_host = ${SENDER_TARGET_HOST}
    target_port = ${SENDER_TARGET_PORT}
EOF

cargo build --manifest-path "${REPO_ROOT}/Cargo.toml" -p styrened -p styrene --bin styrened --bin styrene --quiet

# Start the Rust daemon with its log at $1. The same database, identity, and
# addresses are reused, so a restart proves persisted propagation state.
start_rust_daemon() {
  local log_path="$1"
  # Policy scenarios bound the Rust queue through the daemon's documented
  # environment overrides so the policy outcome is observable in one run.
  local -a policy_env=()
  if [[ "${SCENARIO}" == "propagated_capacity" ]]; then
    policy_env+=("STYRENE_PROPAGATION_QUEUE_MAX_BYTES=${CAPACITY_QUEUE_MAX_BYTES}")
  elif [[ "${SCENARIO}" == "propagated_expiry" ]]; then
    policy_env+=("STYRENE_PROPAGATION_EXPIRY_SECS=${EXPIRY_SECS}")
  fi
  (
    env ${policy_env[@]+"${policy_env[@]}"} \
    LXMF_DISPLAY_NAME="Rust Smoke Node" \
    STYRENE_PROPAGATION_CONTROL_ALLOWED_IDENTITIES="${PY_CONTROL_IDENTITY_HASH}" \
      "${STYRENED_BIN}" \
      --rpc "${RUST_RPC_ADDR}" \
      --db "${RUST_DB}" \
      --identity "${RUST_IDENTITY}" \
      --config "${RUST_DIR}/config.toml" \
      --socket "${RUST_SOCKET}" \
      --transport "${RUST_TRANSPORT_ADDR}" \
      --announce-interval-secs 1 > >(bounded_log "${log_path}" "${LOG_LIMIT_BYTES}") 2>&1
  ) &
  RUST_PID=$!
}

wait_rust_daemon_ready() {
  wait_for_file_pattern "$1" "listening on http://|delivery destination hash=" "${TIMEOUT_SECS}"
}

start_rust_daemon "${RUST_LOG}"

if ! wait_rust_daemon_ready "${RUST_LOG}"; then
  echo "Rust lxmd did not become ready" >&2
  exit 1
fi
runner_milestone "rust-ready"

RUST_DELIVERY_HASH="$(destination_hash_from_identity "${RUST_IDENTITY}" "lxmf" "delivery")"
RUST_PROPAGATION_HASH="$(destination_hash_from_identity "${RUST_IDENTITY}" "lxmf" "propagation")"
RUST_CONTROL_IDENTITY_HASH="$(identity_hash_from_file "${RUST_IDENTITY}")"

cat > "${PY_DIR}/config" <<EOF
[propagation]
enable_node = yes
announce_at_start = yes
announce_interval = 1
autopeer = yes
autopeer_maxdepth = 6
control_allowed = ${RUST_CONTROL_IDENTITY_HASH}

[lxmf]
display_name = Python Smoke Node
announce_at_start = yes
announce_interval = 1

[logging]
loglevel = 4
EOF

(
  "${PYTHON_BIN}" -m LXMF.Utilities.lxmd \
    --config "${PY_DIR}" \
    --rnsconfig "${PY_RNS_DIR}" \
    --propagation-node > >(bounded_log "${PY_LOG}" "${LOG_LIMIT_BYTES}") 2>&1
) &
PY_PID=$!

for _ in $(seq 1 "${TIMEOUT_SECS}"); do
  if [[ -f "${PY_DIR}/identity" ]] && kill -0 "${PY_PID}" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

if [[ ! -f "${PY_DIR}/identity" ]] || ! kill -0 "${PY_PID}" >/dev/null 2>&1; then
  echo "Python lxmd did not become ready" >&2
  exit 1
fi
runner_milestone "python-ready"

PY_DELIVERY_HASH="$(destination_hash_from_identity "${PY_DIR}/identity" "lxmf" "delivery")"
PY_PROPAGATION_HASH="$(destination_hash_from_identity "${PY_DIR}/identity" "lxmf" "propagation")"

# Wait until the Python lxmd control client can query the Rust propagation
# node's status; $1 bounds the wait in seconds.
wait_rust_propagation_ready() {
  local budget="$1"
  rust_propagation_ready=false
  status_deadline=$((SECONDS + budget))
  while (( SECONDS < status_deadline )); do
    status_remaining=$((status_deadline - SECONDS))
    status_attempt_timeout="${REMOTE_STATUS_TIMEOUT_SECS}"
    if (( status_attempt_timeout > status_remaining )); then
      status_attempt_timeout="${status_remaining}"
    fi
    if PYTHONUNBUFFERED=1 "${PYTHON_BIN}" -m LXMF.Utilities.lxmd \
        -v \
        --config "${PY_DIR}" \
        --rnsconfig "${PY_RNS_DIR}" \
        --identity "${PY_DIR}/identity" \
        --timeout "${status_attempt_timeout}" \
        --remote "${RUST_PROPAGATION_HASH}" \
        --status >"${PY_REMOTE_STATUS_LOG}" 2>&1 && \
        grep -q "LXMF Propagation Node running on" "${PY_REMOTE_STATUS_LOG}"; then
      rust_propagation_ready=true
      break
    fi
    sleep 1
  done
  [[ "${rust_propagation_ready}" == "true" ]]
}

# Fetch the Rust node's raw /pn/get/stats response through the pinned Python
# control client and store it as JSON at $1.
python_remote_stats() {
  local output="$1"
  PYTHONUNBUFFERED=1 "${PYTHON_BIN}" - <<'PY' "${PY_DIR}" "${PY_RNS_DIR}" "${PY_DIR}/identity" "${RUST_PROPAGATION_HASH}" "${REMOTE_STATUS_TIMEOUT_SECS}" "${output}"
import json
import sys

from LXMF.Utilities import lxmd

configdir, rnsconfigdir, identity_path, remote, timeout, output = sys.argv[1:7]
lxmd._remote_init(configdir, rnsconfigdir, 0, 0, identity_path)
target = lxmd._get_target_identity(remote)
response = lxmd.query_status(
    lxmd.identity, remote_identity=target, timeout=float(timeout), exit_on_fail=True
)
if not isinstance(response, dict) or "messagestore" not in response:
    raise SystemExit(f"unexpected propagation status response: {response!r}")
store = response["messagestore"]
with open(output, "w", encoding="utf-8") as handle:
    json.dump(
        {
            "messagestore_count": int(store["count"]),
            "messagestore_bytes": int(store["bytes"]),
            "messagestore_limit": int(store["limit"]),
        },
        handle,
        sort_keys=True,
    )
    handle.write("\n")
PY
}

# Capture the Rust daemon's standard propagation snapshot at $1.
rust_propagation_snapshot() {
  "${STYRENE_CLI_BIN}" --socket "${RUST_SOCKET}" propagation > "$1"
}

if [[ "${RUST_ROLE}" == "hub" ]]; then
  if ! wait_rust_propagation_ready "${TIMEOUT_SECS}"; then
    echo "Rust styrened does not expose a Python-compatible lxmf.propagation control destination" >&2
    echo "propagated parity remains unsupported; see ${PY_REMOTE_STATUS_LOG}" >&2
    exit 1
  fi
fi

PY_MESSAGE_CONTENT="python-smoke-message-$(date +%s)"
PY_MESSAGE_METHOD="opportunistic"
if [[ "${SCENARIO}" == "direct" || "${SCENARIO}" == "routed_direct" ]]; then
  PY_MESSAGE_METHOD="direct"
elif [[ "${SCENARIO}" == "direct_resource" || "${SCENARIO}" == "routed_direct_resource" ]]; then
  # Larger than one link packet, so LXMF must carry it as an RNS resource.
  PY_MESSAGE_METHOD="direct"
  PY_MESSAGE_CONTENT="python-smoke-resource-$(date +%s)-$(head -c 4096 /dev/zero | tr '\0' 'p')"
elif [[ "${SCENARIO}" == "propagated_resource_lxm" ]]; then
  PY_MESSAGE_METHOD="propagated"
  PY_MESSAGE_CONTENT="python-smoke-resource-lxm-$(date +%s)-$(head -c 8192 /dev/zero | tr '\0' 'r')"
elif [[ "${SCENARIO}" == "propagated_retrieval" ]]; then
  # Packet-sized content queued for a second Python identity and retrieved
  # from the Rust node after it restarts.
  PY_MESSAGE_METHOD="propagated"
  PY_MESSAGE_CONTENT="python-smoke-retrieval-$(date +%s)"
elif [[ "${SCENARIO}" == "propagated_capacity" ]]; then
  # Packet-sized content the bounded Rust node must refuse.
  PY_MESSAGE_METHOD="propagated"
  PY_MESSAGE_CONTENT="python-smoke-capacity-$(date +%s)"
elif [[ "${SCENARIO}" == "propagated_expiry" ]]; then
  # Packet-sized content queued for a second Python identity that the Rust
  # node must expire before the recipient asks for it.
  PY_MESSAGE_METHOD="propagated"
  PY_MESSAGE_CONTENT="python-smoke-expiry-$(date +%s)"
fi
# The Python peer stays alive after sending so the Rust daemon can deliver a
# message back to it. Send evidence lands in PY_SEND_LOG as soon as the
# outbound message is delivered or sent; receive evidence lands in
# PY_RECEIVE_LOG when the Rust message arrives.
(
"${PYTHON_BIN}" - <<'PY' \
  "${PY_SENDER_RNS_DIR}" \
  "${PY_SENDER_DIR}" \
  "${RUST_DELIVERY_HASH}" \
  "${RUST_PROPAGATION_HASH}" \
  "${PY_MESSAGE_CONTENT}" \
  "${PY_MESSAGE_METHOD}" \
  "${SENDER_WAIT_SECS}" \
  "${PROPAGATION_TARGET_COST}" \
  "${PY_RECEIVE_LOG}" \
  "${TIMEOUT_SECS}" \
  "${SCENARIO}" \
  "${RETRIEVE_SIGNAL}" \
  "${PY_RETRIEVE_LOG}" >"${PY_SEND_LOG}"
import json
import os
import sys
import threading
import time

import RNS
import LXMF

(
    rns_config,
    storage_dir,
    destination_hash_hex,
    propagation_hash_hex,
    content,
    message_method,
    sender_wait_secs,
    propagation_target_cost,
    receive_log_path,
    receive_wait_secs,
    scenario,
    retrieve_signal_path,
    retrieve_log_path,
) = sys.argv[1:14]
destination_hash = bytes.fromhex(destination_hash_hex)
propagation_hash = bytes.fromhex(propagation_hash_hex)
sender_wait_secs = int(sender_wait_secs)
receive_wait_secs = int(receive_wait_secs)

# PY_SENDER_LOGLEVEL raises Reticulum logging for harness debugging. Logs go to
# a file in the sender storage directory because stdout carries the JSON handoff.
sender_loglevel = int(os.environ.get("PY_SENDER_LOGLEVEL", "0"))
if sender_loglevel > 0:
    RNS.logdest = RNS.LOG_FILE
    RNS.logfile = os.path.join(storage_dir, "rns-sender.log")
RNS.Reticulum(configdir=rns_config, loglevel=sender_loglevel)
identity = RNS.Identity()
router = LXMF.LXMRouter(identity=identity, storagepath=storage_dir)
source = router.register_delivery_identity(identity, display_name="Python Smoke Sender")

# The retrieval scenario queues the message for a second local identity and
# later retrieves it from the Rust node with that identity. The pinned LXMF
# router supports one delivery identity, so the recipient gets its own router
# on the same Reticulum instance.
recipient_identity = None
recipient_router = None
if scenario in ("propagated_retrieval", "propagated_expiry"):
    recipient_identity = RNS.Identity()
    recipient_router = LXMF.LXMRouter(
        identity=recipient_identity, storagepath=os.path.join(storage_dir, "recipient")
    )
    recipient = recipient_router.register_delivery_identity(
        recipient_identity, display_name="Python Smoke Recipient"
    )
    if recipient is None:
        raise SystemExit("recipient router did not register a delivery identity")
    destination_hash = recipient.hash
    destination_hash_hex = RNS.hexrep(destination_hash, delimit=False).lower()

received = {}
received_event = threading.Event()


def on_delivery(message):
    if received_event.is_set():
        return
    received.update(
        {
            "content": message.content.decode("utf-8", errors="replace"),
            "title": message.title.decode("utf-8", errors="replace"),
            "source": RNS.hexrep(message.source_hash, delimit=False).lower(),
            "destination": RNS.hexrep(message.destination_hash, delimit=False).lower(),
            "message_id": RNS.hexrep(message.hash, delimit=False).lower(),
            "method": int(message.method) if message.method is not None else None,
            "signature_validated": bool(message.signature_validated),
        }
    )
    received_event.set()


router.register_delivery_callback(on_delivery)
if recipient_router is not None:
    recipient_router.register_delivery_callback(on_delivery)
    recipient_router.set_outbound_propagation_node(propagation_hash)
router.announce(source.hash)
desired_method = {
    "direct": LXMF.LXMessage.DIRECT,
    "opportunistic": LXMF.LXMessage.OPPORTUNISTIC,
    "propagated": LXMF.LXMessage.PROPAGATED,
}.get(message_method)
if desired_method is None:
    raise SystemExit(f"unknown message method {message_method}")
if desired_method == LXMF.LXMessage.PROPAGATED:
    router.set_outbound_propagation_node(propagation_hash)

deadline = time.time() + sender_wait_secs
if desired_method == LXMF.LXMessage.PROPAGATED:
    while time.time() < deadline:
        if RNS.Transport.has_path(propagation_hash) and RNS.Identity.recall(propagation_hash):
            break
        RNS.Transport.request_path(propagation_hash)
        time.sleep(0.5)
    else:
        raise SystemExit("timed out waiting for the Rust propagation node path")

if recipient_identity is not None:
    remote_identity = recipient_identity
else:
    while time.time() < deadline:
        if RNS.Transport.has_path(destination_hash):
            break
        RNS.Transport.request_path(destination_hash)
        time.sleep(0.5)
    else:
        raise SystemExit("timed out waiting for Rust delivery path")

    remote_identity = None
    while time.time() < deadline:
        remote_identity = RNS.Identity.recall(destination_hash)
        if remote_identity is not None:
            break
        time.sleep(0.2)

    if remote_identity is None:
        raise SystemExit("timed out recalling Rust delivery identity")

destination = RNS.Destination(
    remote_identity,
    RNS.Destination.OUT,
    RNS.Destination.SINGLE,
    LXMF.APP_NAME,
    "delivery",
)
message = LXMF.LXMessage(
    destination,
    source,
    content=content,
    desired_method=desired_method,
)
if desired_method == LXMF.LXMessage.PROPAGATED:
    if message.get_propagation_stamp(int(propagation_target_cost)) is None:
        raise SystemExit("failed to generate propagation stamp")
    message.defer_propagation_stamp = False
    message.packed = None
    message.pack()
router.handle_outbound(message)

def send_evidence(state, extra=None):
    payload = {
        "state": int(state),
        "destination": destination_hash_hex,
        "source": RNS.hexrep(source.hash, delimit=False).lower(),
        "message_id": RNS.hexrep(message.hash, delimit=False).lower(),
        "transient_id": (
            RNS.hexrep(message.transient_id, delimit=False).lower()
            if message.transient_id is not None
            else None
        ),
        "method": message_method,
        "representation": (
            int(message.representation) if message.representation is not None else None
        ),
        "hops_to_destination": RNS.Transport.hops_to(destination_hash),
        "next_hop": (
            RNS.hexrep(RNS.Transport.next_hop(destination_hash), delimit=False).lower()
            if RNS.Transport.next_hop(destination_hash) is not None
            else None
        ),
        "next_hop_interface": str(RNS.Transport.next_hop_interface(destination_hash)),
    }
    if extra:
        payload.update(extra)
    return json.dumps(payload)


sent = False
if scenario == "propagated_capacity":
    # The bounded Rust node must refuse the upload. LXMF marks the message
    # SENDING while a transfer runs and returns it to OUTBOUND when the node
    # cancels the resource, so each SENDING-to-OUTBOUND round trip is one
    # rejected attempt. Reaching SENT would mean the bound was not enforced.
    rejected_attempts = 0
    was_sending = False
    capacity_deadline = time.time() + receive_wait_secs * 2
    while time.time() < capacity_deadline:
        state = message.state
        if state in (LXMF.LXMessage.DELIVERED, LXMF.LXMessage.SENT):
            raise SystemExit("bounded Rust node accepted a message beyond its queue capacity")
        if state == LXMF.LXMessage.SENDING:
            was_sending = True
        elif was_sending and state == LXMF.LXMessage.OUTBOUND:
            rejected_attempts += 1
            print(send_evidence(state, {"rejected_attempts": rejected_attempts}), flush=True)
            sent = True
            break
        time.sleep(0.02)
    if not sent:
        raise SystemExit(
            f"Rust node did not reject the upload within the window, state={message.state}"
        )
    raise SystemExit(0)

while time.time() < deadline:
    if message.state in (LXMF.LXMessage.DELIVERED, LXMF.LXMessage.SENT):
        print(send_evidence(message.state), flush=True)
        sent = True
        break
    time.sleep(0.2)

if not sent:
    raise SystemExit(f"timed out waiting for Python message delivery, state={message.state}")

if scenario in ("propagated_retrieval", "propagated_expiry"):
    # Wait for the harness signal, then retrieve with the recipient identity.
    # The retrieval scenario expects the queued message; the expiry scenario
    # expects the transfer to complete with nothing to deliver.
    signal_deadline = time.time() + receive_wait_secs * 4
    while time.time() < signal_deadline and not os.path.exists(retrieve_signal_path):
        time.sleep(0.2)
    if not os.path.exists(retrieve_signal_path):
        raise SystemExit("timed out waiting for the retrieval signal")

    retrieve_deadline = time.time() + receive_wait_secs * 3
    next_request = 0.0
    requested = False
    completed_empty = False
    while time.time() < retrieve_deadline and not received_event.is_set():
        state = recipient_router.propagation_transfer_state
        if (
            scenario == "propagated_expiry"
            and requested
            and state == LXMF.LXMRouter.PR_COMPLETE
            and recipient_router.propagation_transfer_last_result == 0
        ):
            completed_empty = True
            break
        idle = state == LXMF.LXMRouter.PR_IDLE or state >= LXMF.LXMRouter.PR_NO_PATH
        if idle and time.time() >= next_request:
            recipient_router.request_messages_from_propagation_node(recipient_identity)
            requested = True
            next_request = time.time() + 5.0
        received_event.wait(0.2)
    if scenario == "propagated_expiry":
        if received_event.is_set():
            raise SystemExit("Rust node delivered a message that should have expired")
        if not completed_empty:
            raise SystemExit(
                "recipient retrieval did not complete empty, "
                f"state={recipient_router.propagation_transfer_state}"
            )
        with open(retrieve_log_path, "w", encoding="utf-8") as handle:
            json.dump(
                {
                    "transfer_state": int(recipient_router.propagation_transfer_state),
                    "messages": int(recipient_router.propagation_transfer_last_result),
                    "delivered": False,
                    "requested": True,
                },
                handle,
                sort_keys=True,
                separators=(",", ":"),
            )
            handle.write("\n")
        raise SystemExit(0)
    if not received_event.is_set():
        raise SystemExit(
            "timed out retrieving from the Rust node, "
            f"state={recipient_router.propagation_transfer_state}"
        )
    ack_deadline = time.time() + 15.0
    while (
        time.time() < ack_deadline
        and recipient_router.propagation_transfer_state != LXMF.LXMRouter.PR_COMPLETE
    ):
        time.sleep(0.2)
    received["transfer_state"] = int(recipient_router.propagation_transfer_state)
    with open(retrieve_log_path, "w", encoding="utf-8") as handle:
        json.dump(received, handle, sort_keys=True, separators=(",", ":"))
        handle.write("\n")
    raise SystemExit(0)

if desired_method == LXMF.LXMessage.PROPAGATED:
    raise SystemExit(0)

# Rust-to-Python leg: keep announcing until the Rust daemon delivers a message
# to this peer's delivery destination, then retain the received evidence.
receive_deadline = time.time() + receive_wait_secs * 3
next_announce = 0.0
while time.time() < receive_deadline and not received_event.is_set():
    if time.time() >= next_announce:
        router.announce(source.hash)
        next_announce = time.time() + 2.0
    received_event.wait(0.2)

if not received_event.is_set():
    raise SystemExit("timed out waiting for the Rust message to reach the Python peer")

with open(receive_log_path, "w", encoding="utf-8") as handle:
    json.dump(received, handle, sort_keys=True, separators=(",", ":"))
    handle.write("\n")
raise SystemExit(0)
PY
) &
PY_SENDER_PID=$!

if ! wait_for_file_pattern "${PY_SEND_LOG}" '"message_id"' "${SENDER_WAIT_SECS}"; then
  echo "Python peer did not report a sent message" >&2
  exit 1
fi
runner_milestone "python-message-sent"

PY_SENDER_SOURCE_HASH="$("${PYTHON_BIN}" - <<'PY' "${PY_SEND_LOG}"
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
print(payload["source"])
PY
)"

PY_MESSAGE_ID="$("${PYTHON_BIN}" - <<'PY' "${PY_SEND_LOG}"
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
print(payload["message_id"])
PY
)"

PY_MESSAGE_DESTINATION="$("${PYTHON_BIN}" - <<'PY' "${PY_SEND_LOG}"
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
print(payload["destination"])
PY
)"

PY_MESSAGE_TRANSIENT_ID="$("${PYTHON_BIN}" - <<'PY' "${PY_SEND_LOG}"
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
print(payload.get("transient_id") or "")
PY
)"

# LXMF message state reported by the Python sender: 4 = SENT, 8 = DELIVERED.
PY_MESSAGE_STATE="$("${PYTHON_BIN}" - <<'PY' "${PY_SEND_LOG}"
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
print(payload.get("state") if payload.get("state") is not None else "")
PY
)"

# LXMF representations: 1 = PACKET, 2 = RESOURCE. The resource scenarios exist
# to prove resource-backed transfer, so the Python sender must have used it.
PY_MESSAGE_REPRESENTATION="$("${PYTHON_BIN}" - <<'PY' "${PY_SEND_LOG}"
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
print(payload.get("representation") if payload.get("representation") is not None else "")
PY
)"
EXPECTED_PY_REPRESENTATION=1
if [[ "${SCENARIO}" == "direct_resource" || "${SCENARIO}" == "routed_direct_resource" || "${SCENARIO}" == "propagated_resource_lxm" ]]; then
  EXPECTED_PY_REPRESENTATION=2
fi
if [[ "${PY_MESSAGE_REPRESENTATION}" != "${EXPECTED_PY_REPRESENTATION}" ]]; then
  echo "Python sender used representation '${PY_MESSAGE_REPRESENTATION}', expected ${EXPECTED_PY_REPRESENTATION} for ${SCENARIO}" >&2
  exit 1
fi

if [[ "${SCENARIO}" == "propagated_capacity" ]]; then
  # The bounded node must have refused the upload: no queued item may exist
  # for the transient id, the Rust snapshot must carry a recorded capacity
  # failure and an empty queue, and the pinned Python control client must
  # see the same empty store under the same limit.
  rust_propagation_snapshot "${RUST_PROPAGATION_SNAPSHOT_LOG}"
  python_remote_stats "${PY_REMOTE_STATS_LOG}"
  "${PYTHON_BIN}" - <<'PY' \
    "${RUST_DB}" \
    "${PY_MESSAGE_TRANSIENT_ID}" \
    "${PY_MESSAGE_DESTINATION}" \
    "${PY_SENDER_SOURCE_HASH}" \
    "${PY_SEND_LOG}" \
    "${RUST_PROPAGATION_SNAPSHOT_LOG}" \
    "${PY_REMOTE_STATS_LOG}" \
    "${CAPACITY_QUEUE_MAX_BYTES}" \
    "${RUST_CAPACITY_PROOF_PATH}" \
    "${SCENARIO}" \
    "${STYRENE_INTEROP_CORRELATION_ID}"
import json
import sqlite3
import sys
from pathlib import Path

(
    path,
    transient_id,
    destination,
    source,
    send_log_path,
    snapshot_path,
    remote_stats_path,
    queue_max_bytes,
    proof_path,
    scenario,
    correlation_id,
) = sys.argv[1:12]
with sqlite3.connect(path) as db:
    row = db.execute(
        "SELECT state FROM standard_lxmf_propagation_items WHERE transient_id = ? LIMIT 1",
        (bytes.fromhex(transient_id),),
    ).fetchone()
sent = json.loads(Path(send_log_path).read_text(encoding="utf-8"))
snapshot = json.loads(Path(snapshot_path).read_text(encoding="utf-8"))
remote = json.loads(Path(remote_stats_path).read_text(encoding="utf-8"))
policy = snapshot.get("policy") or {}
queue = snapshot.get("queue") or {}
capacity_failures = sum(
    1 for failure in snapshot.get("failures") or [] if failure.get("code") == "capacity"
)
problems = []
if row is not None and row[0] == "queued":
    problems.append("Rust node queued a message beyond its capacity bound")
if int(policy.get("queue_max_bytes", -1)) != int(queue_max_bytes):
    problems.append(f"Rust policy queue_max_bytes is {policy.get('queue_max_bytes')!r}")
if int(queue.get("queued_count", -1)) != 0 or int(queue.get("queued_bytes", -1)) != 0:
    problems.append(f"Rust queue is not empty: {queue!r}")
if capacity_failures < 1:
    problems.append("Rust node recorded no capacity failure")
if int(sent.get("rejected_attempts", 0)) < 1 or int(sent.get("state", -1)) != 1:
    problems.append(f"Python sender did not observe a rejected attempt: {sent!r}")
if remote["messagestore_count"] != 0 or remote["messagestore_limit"] != int(queue_max_bytes):
    problems.append(f"Python control view disagrees with the Rust bound: {remote!r}")
if problems:
    raise SystemExit("; ".join(problems))
proof = {
    "correlation_id": correlation_id,
    "expected_hashes": {
        "destination": destination,
        "source": source,
        "transient_id": transient_id,
    },
    "python_rejected_attempts": int(sent["rejected_attempts"]),
    "python_remote_status": remote,
    "rust_item_present": row is not None and row[0] == "queued",
    "rust_snapshot": {
        "capacity_failures": capacity_failures,
        "queue_max_bytes": int(policy["queue_max_bytes"]),
        "queued_bytes": int(queue["queued_bytes"]),
        "queued_count": int(queue["queued_count"]),
    },
    "scenario": scenario,
    "table": "standard_lxmf_propagation_items",
}
with open(proof_path, "w", encoding="utf-8") as handle:
    json.dump(proof, handle, sort_keys=True, separators=(",", ":"))
    handle.write("\n")
PY
  runner_assertion "python-to-rust-capacity-rejected"
  runner_milestone "rust-capacity-enforced"
elif [[ "${RUST_ROLE}" == "hub" ]]; then
  for _ in $(seq 1 "${TIMEOUT_SECS}"); do
    if "${PYTHON_BIN}" - <<'PY' "${RUST_DB}" "${PY_MESSAGE_TRANSIENT_ID}" "${PY_MESSAGE_DESTINATION}"; then
import sqlite3
import sys

path, transient_id, destination = sys.argv[1:4]
with sqlite3.connect(path) as db:
    row = db.execute(
        "SELECT 1 FROM standard_lxmf_propagation_items "
        "WHERE transient_id = ? AND destination = ? AND state = 'queued' "
        "AND stored_size > 32 LIMIT 1",
        (bytes.fromhex(transient_id), bytes.fromhex(destination)),
    ).fetchone()
raise SystemExit(0 if row else 1)
PY
      break
    fi
    sleep 1
  done

  "${PYTHON_BIN}" - <<'PY' \
    "${RUST_DB}" \
    "${PY_MESSAGE_TRANSIENT_ID}" \
    "${PY_MESSAGE_DESTINATION}" \
    "${PY_SENDER_SOURCE_HASH}" \
    "${DATASTORE_PROOF_PATH}" \
    "${SCENARIO}" \
    "${STYRENE_INTEROP_CORRELATION_ID}"
import json
import sqlite3
import sys

path, transient_id, destination, source, proof_path, scenario, correlation_id = sys.argv[1:8]
with sqlite3.connect(path) as db:
    row = db.execute(
        "SELECT hex(transient_id), hex(destination), state, stored_size "
        "FROM standard_lxmf_propagation_items "
        "WHERE transient_id = ? AND destination = ? AND state = 'queued' "
        "AND stored_size > 32 LIMIT 1",
        (bytes.fromhex(transient_id), bytes.fromhex(destination)),
    ).fetchone()
if row is None:
    raise SystemExit("Rust daemon did not persist the exact propagated LXMF transient item")
proof = {
    "correlation_id": correlation_id,
    "expected_hashes": {
        "destination": destination,
        "source": source,
        "transient_id": transient_id,
    },
    "scenario": scenario,
    "selected_row": {
        "destination": row[1].lower(),
        "state": row[2],
        "stored_size": row[3],
        "transient_id": row[0].lower(),
    },
    "table": "standard_lxmf_propagation_items",
}
with open(proof_path, "w", encoding="utf-8") as handle:
    json.dump(proof, handle, sort_keys=True, separators=(",", ":"))
    handle.write("\n")
PY
  runner_assertion "python-to-rust-propagation-item"
else
  for _ in $(seq 1 "${TIMEOUT_SECS}"); do
    if "${PYTHON_BIN}" - <<'PY' "${RUST_DB}" "${PY_MESSAGE_CONTENT}" "${PY_SENDER_SOURCE_HASH}" "${RUST_DELIVERY_HASH}" "${PY_MESSAGE_ID}"; then
import sqlite3
import sys

path, content, source, destination, message_id = sys.argv[1:6]
with sqlite3.connect(path) as db:
    row = db.execute(
        "SELECT 1 FROM messages WHERE content = ? AND source = ? AND destination = ? "
        "AND id = ? AND direction = 'in' LIMIT 1",
        (content, source, destination, message_id),
    ).fetchone()
raise SystemExit(0 if row else 1)
PY
      break
    fi
    sleep 1
  done

  "${PYTHON_BIN}" - <<'PY' \
    "${RUST_DB}" \
    "${PY_MESSAGE_CONTENT}" \
    "${PY_SENDER_SOURCE_HASH}" \
    "${RUST_DELIVERY_HASH}" \
    "${PY_MESSAGE_ID}" \
    "${DATASTORE_PROOF_PATH}" \
    "${SCENARIO}" \
    "${STYRENE_INTEROP_CORRELATION_ID}"
import json
import sqlite3
import sys

path, content, source, destination, message_id, proof_path, scenario, correlation_id = sys.argv[1:9]
with sqlite3.connect(path) as db:
    row = db.execute(
        "SELECT id, source, destination, content, direction FROM messages "
        "WHERE content = ? AND source = ? AND destination = ? AND id = ? "
        "AND direction = 'in' LIMIT 1",
        (content, source, destination, message_id),
    ).fetchone()
if row is None:
    raise SystemExit("Rust daemon did not persist the inbound Python LXMF message")
proof = {
    "correlation_id": correlation_id,
    "expected_content": content,
    "expected_hashes": {
        "destination": destination,
        "source": source,
        "message_id": message_id,
    },
    "scenario": scenario,
    "selected_row": {
        "content": row[3],
        "destination": row[2],
        "direction": row[4],
        "id": row[0],
        "source": row[1],
    },
    "table": "messages",
}
with open(proof_path, "w", encoding="utf-8") as handle:
    json.dump(proof, handle, sort_keys=True, separators=(",", ":"))
    handle.write("\n")
PY
  runner_assertion "python-to-rust-content"
fi
if [[ "${SCENARIO}" != "propagated_capacity" ]]; then
  runner_milestone "rust-message-persisted"
fi

BIDIRECTIONAL=false
case "${SCENARIO}" in
  direct|direct_resource|opportunistic|routed_direct|routed_direct_resource) BIDIRECTIONAL=true ;;
esac

if [[ "${ROUTED}" == "true" ]]; then
  # Both endpoints must see each other two hops away through the Python
  # transport hop, and both must name the same transport identity as the
  # next hop. The Rust route record comes from the daemon's path table.
  for _ in $(seq 1 "${TIMEOUT_SECS}"); do
    if "${STYRENE_CLI_BIN}" --socket "${RUST_SOCKET}" path "${PY_SENDER_SOURCE_HASH}" >"${RUST_PATH_LOG}" 2>&1 \
      && grep -q '^hops=' "${RUST_PATH_LOG}"; then
      break
    fi
    sleep 1
  done
  "${PYTHON_BIN}" - <<'PY' \
    "${RUST_PATH_LOG}" \
    "${PY_SEND_LOG}" \
    "${PY_SENDER_SOURCE_HASH}" \
    "${RUST_DELIVERY_HASH}" \
    "${ROUTED_PATH_PROOF_PATH}" \
    "${SCENARIO}" \
    "${STYRENE_INTEROP_CORRELATION_ID}"
import json
import sys
from pathlib import Path

path_log, send_log, sender_hash, rust_delivery, proof_path, scenario, correlation_id = sys.argv[1:8]
rust = {}
for line in Path(path_log).read_text(encoding="utf-8").splitlines():
    if "=" in line:
        key, value = line.split("=", 1)
        rust[key.strip()] = value.strip()
sent = json.loads(Path(send_log).read_text(encoding="utf-8"))
rust_hops = int(rust["hops"]) if rust.get("hops", "").isdigit() else None
python_hops = sent.get("hops_to_destination")
problems = []
if rust.get("found") != "true" or rust_hops != 2:
    problems.append(f"Rust route to the Python sender is not two hops: {rust!r}")
if python_hops != 2:
    problems.append(f"Python route to the Rust node is not two hops: {python_hops!r}")
if not rust.get("next_hop") or rust.get("next_hop") != sent.get("next_hop"):
    problems.append(
        f"next hops disagree: rust={rust.get('next_hop')!r} python={sent.get('next_hop')!r}"
    )
if problems:
    raise SystemExit("; ".join(problems))
proof = {
    "correlation_id": correlation_id,
    "expected_hops": 2,
    "expected_hashes": {"python_sender": sender_hash, "rust_delivery": rust_delivery},
    "python_route": {
        "hops": python_hops,
        "next_hop": sent.get("next_hop"),
        "next_hop_interface": sent.get("next_hop_interface"),
    },
    "rust_route": {
        "found": rust.get("found") == "true",
        "hops": rust_hops,
        "interface": rust.get("interface"),
        "next_hop": rust.get("next_hop"),
    },
    "scenario": scenario,
}
with open(proof_path, "w", encoding="utf-8") as handle:
    json.dump(proof, handle, sort_keys=True, separators=(",", ":"))
    handle.write("\n")
PY
  runner_assertion "routed-path-two-hops"
  runner_milestone "routed-path-verified"
fi

RUST_MESSAGE_CONTENT=""
RUST_MESSAGE_ID=""
RUST_OUTBOUND_STATE=""
if [[ "${BIDIRECTIONAL}" == "true" ]]; then
  RUST_MESSAGE_CONTENT="rust-smoke-message-$(date +%s)"
  if [[ "${SCENARIO}" == "direct_resource" || "${SCENARIO}" == "routed_direct_resource" ]]; then
    RUST_MESSAGE_CONTENT="rust-smoke-resource-$(date +%s)-$(head -c 4096 /dev/zero | tr '\0' 'q')"
  fi
  if ! "${STYRENE_CLI_BIN}" --socket "${RUST_SOCKET}" send \
      "${PY_SENDER_SOURCE_HASH}" \
      "${RUST_MESSAGE_CONTENT}" \
      --delivery-method "${PY_MESSAGE_METHOD}" >"${RUST_SEND_LOG}" 2>&1; then
    echo "Rust daemon rejected the outbound ${PY_MESSAGE_METHOD} message" >&2
    cat "${RUST_SEND_LOG}" >&2 || true
    exit 1
  fi
  RUST_MESSAGE_ID=""
  for _ in $(seq 1 "${TIMEOUT_SECS}"); do
    RUST_MESSAGE_ID="$("${PYTHON_BIN}" - <<'PY' "${RUST_DB}" "${RUST_MESSAGE_CONTENT}" "${RUST_DELIVERY_HASH}" "${PY_SENDER_SOURCE_HASH}"
import sqlite3
import sys

path, content, source, destination = sys.argv[1:5]
with sqlite3.connect(path) as db:
    row = db.execute(
        "SELECT id FROM messages WHERE content = ? AND source = ? AND destination = ? "
        "AND direction = 'out' LIMIT 1",
        (content, source, destination),
    ).fetchone()
print(row[0] if row else "")
PY
)"
    if [[ -n "${RUST_MESSAGE_ID}" ]]; then
      break
    fi
    sleep 1
  done
  if [[ -z "${RUST_MESSAGE_ID}" ]]; then
    echo "Rust daemon did not persist the outbound message" >&2
    exit 1
  fi
  runner_milestone "rust-message-sent"

  if ! wait_for_file_pattern "${PY_RECEIVE_LOG}" '"message_id"' "${TIMEOUT_SECS}"; then
    echo "Python peer did not receive the Rust ${PY_MESSAGE_METHOD} message" >&2
    exit 1
  fi
  if ! wait "${PY_SENDER_PID}"; then
    echo "Python peer exited with an error after receiving the Rust message" >&2
    exit 1
  fi
  PY_SENDER_PID=""

  # Both Direct and Opportunistic deliveries are proved by the Python peer, so
  # the Rust route must reach the delivered state in either scenario.
  EXPECTED_ROUTE_STATES="delivered"
  for _ in $(seq 1 "${TIMEOUT_SECS}"); do
    if "${PYTHON_BIN}" - <<'PY' "${RUST_DB}" "${RUST_MESSAGE_ID}" "${EXPECTED_ROUTE_STATES}"; then
import sqlite3
import sys

path, message_id, expected_states = sys.argv[1:4]
with sqlite3.connect(path) as db:
    row = db.execute(
        "SELECT state FROM outbound_routes WHERE message_id = ? LIMIT 1",
        (message_id,),
    ).fetchone()
raise SystemExit(0 if row is not None and row[0] in expected_states.split("|") else 1)
PY
      break
    fi
    sleep 1
  done

  RUST_OUTBOUND_STATE="$("${PYTHON_BIN}" - <<'PY' \
    "${RUST_DB}" \
    "${RUST_MESSAGE_ID}" \
    "${RUST_MESSAGE_CONTENT}" \
    "${RUST_DELIVERY_HASH}" \
    "${PY_SENDER_SOURCE_HASH}" \
    "${PY_MESSAGE_METHOD}" \
    "${EXPECTED_ROUTE_STATES}" \
    "${PY_RECEIVE_LOG}" \
    "${RUST_OUTBOUND_PROOF_PATH}" \
    "${SCENARIO}" \
    "${STYRENE_INTEROP_CORRELATION_ID}"
import json
import sqlite3
import sys
from pathlib import Path

(
    path,
    message_id,
    content,
    source,
    destination,
    method,
    expected_states,
    receive_log_path,
    proof_path,
    scenario,
    correlation_id,
) = sys.argv[1:12]
with sqlite3.connect(path) as db:
    row = db.execute(
        "SELECT m.id, m.source, m.destination, m.content, m.direction, "
        "r.requested_method, r.actual_method, r.state, r.representation "
        "FROM messages m JOIN outbound_routes r ON r.message_id = m.id "
        "WHERE m.id = ? AND m.direction = 'out' LIMIT 1",
        (message_id,),
    ).fetchone()
if row is None:
    raise SystemExit("Rust daemon did not persist the outbound message route")
if row[7] not in expected_states.split("|"):
    raise SystemExit(f"Rust outbound route state {row[7]!r} is not in {expected_states!r}")
received = json.loads(Path(receive_log_path).read_text(encoding="utf-8"))
mismatches = [
    name
    for name, expected, actual in (
        ("content", content, received.get("content")),
        ("source", source, received.get("source")),
        ("destination", destination, received.get("destination")),
        ("message_id", message_id, received.get("message_id")),
        ("signature_validated", True, received.get("signature_validated")),
    )
    if expected != actual
]
if mismatches:
    raise SystemExit(f"Python peer evidence disagrees with the Rust daemon on {mismatches}")
proof = {
    "correlation_id": correlation_id,
    "expected_content": content,
    "expected_hashes": {
        "destination": destination,
        "source": source,
        "message_id": message_id,
    },
    "expected_method": method,
    "python_receipt": received,
    "route": {
        "actual_method": row[6],
        "representation": row[8],
        "requested_method": row[5],
        "state": row[7],
    },
    "scenario": scenario,
    "selected_row": {
        "content": row[3],
        "destination": row[2],
        "direction": row[4],
        "id": row[0],
        "source": row[1],
    },
    "table": "messages",
}
with open(proof_path, "w", encoding="utf-8") as handle:
    json.dump(proof, handle, sort_keys=True, separators=(",", ":"))
    handle.write("\n")
print(row[7])
PY
)"
  runner_assertion "rust-to-python-content"
  runner_milestone "python-message-received"
fi

PY_RETRIEVAL_TRANSFER_STATE=""
RUST_ITEM_STATE=""
if [[ "${SCENARIO}" == "propagated_expiry" ]]; then
  # The queued item must expire on its own: poll the Rust snapshot until the
  # queue is empty and the expired count is up, then let the recipient ask.
  expiry_deadline=$((SECONDS + EXPIRY_SECS + TIMEOUT_SECS))
  expired=false
  while (( SECONDS < expiry_deadline )); do
    if rust_propagation_snapshot "${RUST_PROPAGATION_SNAPSHOT_LOG}" && \
      "${PYTHON_BIN}" - <<'PY' "${RUST_PROPAGATION_SNAPSHOT_LOG}"; then
import json
import sys
from pathlib import Path

queue = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8")).get("queue") or {}
raise SystemExit(0 if queue.get("queued_count") == 0 and queue.get("expired_count", 0) >= 1 else 1)
PY
      expired=true
      break
    fi
    sleep 1
  done
  if [[ "${expired}" != "true" ]]; then
    echo "Rust node did not expire the queued propagation item within ${EXPIRY_SECS}s + ${TIMEOUT_SECS}s" >&2
    exit 1
  fi
  runner_milestone "rust-item-expired"
  python_remote_stats "${PY_REMOTE_STATS_LOG}"
  : > "${RETRIEVE_SIGNAL}"
  runner_milestone "python-retrieval-requested"
  if ! wait_for_file_pattern "${PY_RETRIEVE_LOG}" '"transfer_state"' "$((TIMEOUT_SECS * 2))"; then
    echo "Python recipient did not complete an empty retrieval from the Rust node" >&2
    exit 1
  fi
  if ! wait "${PY_SENDER_PID}"; then
    echo "Python peer exited with an error after the empty retrieval" >&2
    exit 1
  fi
  PY_SENDER_PID=""
  RUST_ITEM_STATE="$("${PYTHON_BIN}" - <<'PY' \
    "${RUST_DB}" \
    "${PY_MESSAGE_TRANSIENT_ID}" \
    "${PY_MESSAGE_DESTINATION}" \
    "${PY_SENDER_SOURCE_HASH}" \
    "${RUST_PROPAGATION_SNAPSHOT_LOG}" \
    "${PY_REMOTE_STATS_LOG}" \
    "${PY_RETRIEVE_LOG}" \
    "${EXPIRY_SECS}" \
    "${RUST_EXPIRY_PROOF_PATH}" \
    "${SCENARIO}" \
    "${STYRENE_INTEROP_CORRELATION_ID}"
import json
import sqlite3
import sys
from pathlib import Path

(
    path,
    transient_id,
    destination,
    source,
    snapshot_path,
    remote_stats_path,
    retrieve_log_path,
    expiry_secs,
    proof_path,
    scenario,
    correlation_id,
) = sys.argv[1:12]
with sqlite3.connect(path) as db:
    row = db.execute(
        "SELECT hex(transient_id), hex(destination), state, stored_size "
        "FROM standard_lxmf_propagation_items WHERE transient_id = ? LIMIT 1",
        (bytes.fromhex(transient_id),),
    ).fetchone()
if row is None:
    raise SystemExit("Rust node lost the propagation item instead of expiring it")
snapshot = json.loads(Path(snapshot_path).read_text(encoding="utf-8"))
remote = json.loads(Path(remote_stats_path).read_text(encoding="utf-8"))
retrieval = json.loads(Path(retrieve_log_path).read_text(encoding="utf-8"))
policy = snapshot.get("policy") or {}
queue = snapshot.get("queue") or {}
problems = []
if row[2] != "expired" or int(row[3]) != 0:
    problems.append(f"Rust item state is {row[2]!r} with stored_size {row[3]!r}")
if int(policy.get("expiry_secs", -1)) != int(expiry_secs):
    problems.append(f"Rust policy expiry_secs is {policy.get('expiry_secs')!r}")
if int(queue.get("queued_count", -1)) != 0 or int(queue.get("expired_count", 0)) < 1:
    problems.append(f"Rust queue did not record the expiry: {queue!r}")
if remote["messagestore_count"] != 0:
    problems.append(f"Python control view still counts stored messages: {remote!r}")
if retrieval.get("transfer_state") != 7 or retrieval.get("messages") != 0 or retrieval.get("delivered"):
    problems.append(f"Python retrieval did not complete empty: {retrieval!r}")
if problems:
    raise SystemExit("; ".join(problems))
proof = {
    "correlation_id": correlation_id,
    "expected_hashes": {
        "destination": destination,
        "source": source,
        "transient_id": transient_id,
    },
    "item": {
        "destination": row[1].lower(),
        "state": row[2],
        "stored_size": int(row[3]),
        "transient_id": row[0].lower(),
    },
    "python_remote_status": remote,
    "python_retrieval": retrieval,
    "rust_snapshot": {
        "expired_count": int(queue["expired_count"]),
        "expiry_secs": int(policy["expiry_secs"]),
        "queued_count": int(queue["queued_count"]),
    },
    "scenario": scenario,
    "table": "standard_lxmf_propagation_items",
}
with open(proof_path, "w", encoding="utf-8") as handle:
    json.dump(proof, handle, sort_keys=True, separators=(",", ":"))
    handle.write("\n")
print(row[2])
PY
)"
  PY_RETRIEVAL_TRANSFER_STATE="$("${PYTHON_BIN}" - <<'PY' "${PY_RETRIEVE_LOG}"
import json
import sys
from pathlib import Path

print(json.loads(Path(sys.argv[1]).read_text(encoding="utf-8")).get("transfer_state"))
PY
)"
  runner_assertion "rust-to-python-expired-retrieval"
  runner_milestone "python-retrieval-empty"
fi
if [[ "${SCENARIO}" == "propagated_retrieval" ]]; then
  # Offline delivery must survive a node restart: stop the Rust node while the
  # message is queued, start it again on the same database, and only then let
  # the recipient identity retrieve.
  runner_milestone "rust-restart-requested"
  cleanup_child "${RUST_PID}"
  RUST_PID=""
  start_rust_daemon "${RUST_RESTART_LOG}"
  if ! wait_rust_daemon_ready "${RUST_RESTART_LOG}"; then
    echo "Rust lxmd did not become ready after restart" >&2
    exit 1
  fi
  runner_milestone "rust-restarted"
  if ! wait_rust_propagation_ready "$((TIMEOUT_SECS * 2))"; then
    echo "Rust propagation node was not reachable after restart; see ${PY_REMOTE_STATUS_LOG}" >&2
    exit 1
  fi
  : > "${RETRIEVE_SIGNAL}"
  runner_milestone "python-retrieval-requested"
  if ! wait_for_file_pattern "${PY_RETRIEVE_LOG}" '"message_id"' "$((TIMEOUT_SECS * 2))"; then
    echo "Python recipient did not retrieve the queued message from the Rust node" >&2
    exit 1
  fi
  if ! wait "${PY_SENDER_PID}"; then
    echo "Python peer exited with an error after retrieval" >&2
    exit 1
  fi
  PY_SENDER_PID=""

  for _ in $(seq 1 "${TIMEOUT_SECS}"); do
    if "${PYTHON_BIN}" - <<'PY' "${RUST_DB}" "${PY_MESSAGE_TRANSIENT_ID}"; then
import sqlite3
import sys

path, transient_id = sys.argv[1:3]
with sqlite3.connect(path) as db:
    row = db.execute(
        "SELECT state FROM standard_lxmf_propagation_items WHERE transient_id = ? LIMIT 1",
        (bytes.fromhex(transient_id),),
    ).fetchone()
raise SystemExit(0 if row is not None and row[0] == "acknowledged" else 1)
PY
      break
    fi
    sleep 1
  done

  RUST_ITEM_STATE="$("${PYTHON_BIN}" - <<'PY' \
    "${RUST_DB}" \
    "${PY_MESSAGE_TRANSIENT_ID}" \
    "${PY_MESSAGE_CONTENT}" \
    "${PY_SENDER_SOURCE_HASH}" \
    "${PY_MESSAGE_DESTINATION}" \
    "${PY_MESSAGE_ID}" \
    "${PY_RETRIEVE_LOG}" \
    "${RUST_RETRIEVAL_PROOF_PATH}" \
    "${SCENARIO}" \
    "${STYRENE_INTEROP_CORRELATION_ID}"
import json
import sqlite3
import sys
from pathlib import Path

(
    path,
    transient_id,
    content,
    source,
    destination,
    message_id,
    retrieve_log_path,
    proof_path,
    scenario,
    correlation_id,
) = sys.argv[1:11]
with sqlite3.connect(path) as db:
    row = db.execute(
        "SELECT hex(transient_id), hex(destination), state FROM standard_lxmf_propagation_items "
        "WHERE transient_id = ? LIMIT 1",
        (bytes.fromhex(transient_id),),
    ).fetchone()
if row is None:
    raise SystemExit("Rust node lost the propagation item across restart")
if row[2] != "acknowledged":
    raise SystemExit(f"Rust propagation item state is {row[2]!r}, expected acknowledged")
received = json.loads(Path(retrieve_log_path).read_text(encoding="utf-8"))
mismatches = [
    name
    for name, expected, actual in (
        ("content", content, received.get("content")),
        ("source", source, received.get("source")),
        ("destination", destination, received.get("destination")),
        ("message_id", message_id, received.get("message_id")),
        ("signature_validated", True, received.get("signature_validated")),
        ("transfer_state", 7, received.get("transfer_state")),
    )
    if expected != actual
]
if mismatches:
    raise SystemExit(f"Python retrieval evidence disagrees with the Rust node on {mismatches}")
proof = {
    "correlation_id": correlation_id,
    "expected_content": content,
    "expected_hashes": {
        "destination": destination,
        "source": source,
        "message_id": message_id,
        "transient_id": transient_id,
    },
    "item": {
        "destination": row[1].lower(),
        "state": row[2],
        "transient_id": row[0].lower(),
    },
    "python_receipt": received,
    "scenario": scenario,
    "table": "standard_lxmf_propagation_items",
}
with open(proof_path, "w", encoding="utf-8") as handle:
    json.dump(proof, handle, sort_keys=True, separators=(",", ":"))
    handle.write("\n")
print(row[2])
PY
)"
  PY_RETRIEVAL_TRANSFER_STATE="$("${PYTHON_BIN}" - <<'PY' "${PY_RETRIEVE_LOG}"
import json
import sys
from pathlib import Path

print(json.loads(Path(sys.argv[1]).read_text(encoding="utf-8")).get("transfer_state"))
PY
)"
  runner_assertion "rust-to-python-retrieval"
  runner_milestone "python-message-retrieved"
fi

"${PYTHON_BIN}" - <<'PY' \
  "${REPORT_PATH}" \
  "${TMP_ROOT}" \
  "${RUST_LOG}" \
  "${PY_LOG}" \
  "${PY_REMOTE_STATUS_LOG}" \
  "${RUST_DELIVERY_HASH}" \
  "${RUST_PROPAGATION_HASH}" \
  "${PY_DELIVERY_HASH}" \
  "${PY_MESSAGE_CONTENT}" \
    "${PY_SENDER_SOURCE_HASH}" \
    "${PY_MESSAGE_ID}" \
    "${PY_MESSAGE_TRANSIENT_ID}" \
  "${PY_MESSAGE_REPRESENTATION}" \
  "${PY_MESSAGE_STATE}" \
  "${RUST_MESSAGE_CONTENT}" \
  "${RUST_MESSAGE_ID}" \
  "${RUST_OUTBOUND_STATE}" \
  "${PY_RETRIEVAL_TRANSFER_STATE}" \
  "${RUST_ITEM_STATE}" \
  "${SCENARIO}" \
  "${STYRENE_INTEROP_CORRELATION_ID}"
import json
import sys

(
    report_path,
    tmp_root,
    rust_log,
    py_log,
    py_remote_status_log,
    rust_delivery_hash,
    rust_propagation_hash,
    py_delivery_hash,
    py_message_content,
    py_sender_source_hash,
    py_message_id,
    py_message_transient_id,
    py_message_representation,
    py_message_state,
    rust_message_content,
    rust_message_id,
    rust_outbound_state,
    python_retrieval_transfer_state,
    rust_item_state,
    scenario,
    correlation_id,
) = sys.argv[1:22]

report = {
    "status": "pass",
    "scenario": scenario,
    "correlation_id": correlation_id,
    "proof": {
        "python_to_rust_inbound_content": py_message_content,
        "python_sender_source_hash": py_sender_source_hash,
        "python_message_id": py_message_id,
        "python_message_transient_id": py_message_transient_id,
        "python_message_representation": py_message_representation,
        "python_message_state": py_message_state,
        "rust_to_python_outbound_content": rust_message_content,
        "rust_message_id": rust_message_id,
        "rust_outbound_state": rust_outbound_state,
        "python_retrieval_transfer_state": python_retrieval_transfer_state,
        "rust_item_state": rust_item_state,
    },
    "hashes": {
        "rust_delivery": rust_delivery_hash,
        "rust_propagation": rust_propagation_hash,
        "python_delivery": py_delivery_hash,
    },
    "logs": {
        "tmp_root": tmp_root,
        "rust_lxmd": rust_log,
        "python_lxmd": py_log,
        "python_remote_status": py_remote_status_log,
    },
}

with open(report_path, "w", encoding="utf-8") as handle:
    json.dump(report, handle, indent=2)
    handle.write("\n")
PY

if [[ "${RUST_ROLE}" == "hub" ]]; then
  "${PYTHON_BIN}" - <<'PY' "${REPORT_PATH}" "${RUST_PROPAGATION_HASH}"
import json
import sys
from pathlib import Path

report_path, rust_prop = sys.argv[1:3]
report = json.loads(Path(report_path).read_text(encoding="utf-8"))
report["proof"]["python_remote_status_to_rust"] = rust_prop
Path(report_path).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
PY
fi

runner_artifact "scenario-report" "${REPORT_PATH}"
if [[ "${SCENARIO}" == "propagated_capacity" ]]; then
  runner_artifact "rust-capacity-proof" "${RUST_CAPACITY_PROOF_PATH}"
else
  runner_artifact "datastore-proof" "${DATASTORE_PROOF_PATH}"
fi
if [[ "${SCENARIO}" == "propagated_expiry" ]]; then
  runner_artifact "rust-expiry-proof" "${RUST_EXPIRY_PROOF_PATH}"
fi
if [[ "${BIDIRECTIONAL}" == "true" ]]; then
  runner_artifact "rust-outbound-proof" "${RUST_OUTBOUND_PROOF_PATH}"
fi
if [[ "${ROUTED}" == "true" ]]; then
  runner_artifact "routed-path-proof" "${ROUTED_PATH_PROOF_PATH}"
fi
if [[ "${SCENARIO}" == "propagated_retrieval" ]]; then
  runner_artifact "rust-retrieval-proof" "${RUST_RETRIEVAL_PROOF_PATH}"
  runner_artifact "rust-daemon-restart-log" "${RUST_RESTART_LOG}"
fi
runner_artifact "rust-daemon-log" "${RUST_LOG}"
runner_artifact "python-daemon-log" "${PY_LOG}"

echo "[python-lxmd-rust-lxmd-smoke] pass"
echo "[python-lxmd-rust-lxmd-smoke] scenario=${SCENARIO}"
echo "[python-lxmd-rust-lxmd-smoke] report=${REPORT_PATH}"
echo "[python-lxmd-rust-lxmd-smoke] logs=${TMP_ROOT}"
