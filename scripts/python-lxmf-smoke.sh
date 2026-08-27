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
LOG_LIMIT_BYTES="${LOG_LIMIT_BYTES:-2097152}"
SCENARIO="${SCENARIO:-direct}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
export CARGO_TARGET_DIR
STYRENED_BIN="${STYRENED_BIN:-${CARGO_TARGET_DIR}/debug/styrened}"

PORT_SEED="${PORT_SEED:-$$}"
RUST_RPC_PORT="${RUST_RPC_PORT:-$((4243 + (PORT_SEED % 2000)))}"
RUST_TRANSPORT_PORT="${RUST_TRANSPORT_PORT:-$((37429 + (PORT_SEED % 2000)))}"
RUST_RPC_ADDR="${RUST_RPC_ADDR:-127.0.0.1:${RUST_RPC_PORT}}"
RUST_TRANSPORT_ADDR="${RUST_TRANSPORT_ADDR:-127.0.0.1:${RUST_TRANSPORT_PORT}}"
RUST_TRANSPORT_HOST="${RUST_TRANSPORT_ADDR%:*}"
RUST_TRANSPORT_PORT="${RUST_TRANSPORT_ADDR##*:}"

PY_SHARED_INSTANCE_PORT="${PY_SHARED_INSTANCE_PORT:-$((39428 + (PORT_SEED % 2000)))}"
PY_INSTANCE_CONTROL_PORT="${PY_INSTANCE_CONTROL_PORT:-$((PY_SHARED_INSTANCE_PORT + 1))}"

usage() {
  cat <<'EOF'
Usage: python-lxmd-rust-lxmd-smoke.sh [--scenario direct|opportunistic|propagated_resource_lxm] [--timeout SECONDS]
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
    direct|opportunistic|propagated_resource_lxm) ;;
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
  if [[ -n "${PY_PID:-}" ]]; then
    cleanup_child "${PY_PID}"
  fi
  if [[ -n "${RUST_PID:-}" ]]; then
    cleanup_child "${RUST_PID}"
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

RUST_ROLE="full_node"
if [[ "${SCENARIO}" == "propagated_resource_lxm" ]]; then
  RUST_ROLE="hub"
fi

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
    target_host = ${RUST_TRANSPORT_HOST}
    target_port = ${RUST_TRANSPORT_PORT}
EOF

cargo build --manifest-path "${REPO_ROOT}/Cargo.toml" --bin styrened --quiet

(
  LXMF_DISPLAY_NAME="Rust Smoke Node" \
  STYRENE_PROPAGATION_CONTROL_ALLOWED_IDENTITIES="${PY_CONTROL_IDENTITY_HASH}" \
    "${STYRENED_BIN}" \
    --rpc "${RUST_RPC_ADDR}" \
    --db "${RUST_DB}" \
    --identity "${RUST_IDENTITY}" \
    --config "${RUST_DIR}/config.toml" \
    --transport "${RUST_TRANSPORT_ADDR}" \
    --announce-interval-secs 1 > >(bounded_log "${RUST_LOG}" "${LOG_LIMIT_BYTES}") 2>&1
) &
RUST_PID=$!

if ! wait_for_file_pattern "${RUST_LOG}" "listening on http://|delivery destination hash=" "${TIMEOUT_SECS}"; then
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

if [[ "${SCENARIO}" == "propagated_resource_lxm" ]]; then
  rust_propagation_ready=false
  status_deadline=$((SECONDS + TIMEOUT_SECS))
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

  if [[ "${rust_propagation_ready}" != "true" ]]; then
    echo "Rust styrened does not expose a Python-compatible lxmf.propagation control destination" >&2
    echo "propagated resource/.lxm parity remains unsupported; see ${PY_REMOTE_STATUS_LOG}" >&2
    exit 1
  fi
fi

PY_MESSAGE_CONTENT="python-smoke-message-$(date +%s)"
PY_MESSAGE_METHOD="opportunistic"
if [[ "${SCENARIO}" == "direct" ]]; then
  PY_MESSAGE_METHOD="direct"
elif [[ "${SCENARIO}" == "propagated_resource_lxm" ]]; then
  PY_MESSAGE_METHOD="propagated"
  PY_MESSAGE_CONTENT="python-smoke-resource-lxm-$(date +%s)-$(head -c 8192 /dev/zero | tr '\0' 'r')"
fi
"${PYTHON_BIN}" - <<'PY' \
  "${PY_SENDER_RNS_DIR}" \
  "${PY_SENDER_DIR}" \
  "${RUST_DELIVERY_HASH}" \
  "${RUST_PROPAGATION_HASH}" \
  "${PY_MESSAGE_CONTENT}" \
  "${PY_MESSAGE_METHOD}" \
  "${SENDER_WAIT_SECS}" \
  "${PROPAGATION_TARGET_COST}" >"${PY_SEND_LOG}"
import json
import os
import sys
import time

import RNS
import LXMF

rns_config, storage_dir, destination_hash_hex, propagation_hash_hex, content, message_method, sender_wait_secs, propagation_target_cost = sys.argv[1:9]
destination_hash = bytes.fromhex(destination_hash_hex)
propagation_hash = bytes.fromhex(propagation_hash_hex)
sender_wait_secs = int(sender_wait_secs)

RNS.Reticulum(configdir=rns_config, loglevel=0)
identity = RNS.Identity()
router = LXMF.LXMRouter(identity=identity, storagepath=storage_dir)
source = router.register_delivery_identity(identity, display_name="Python Smoke Sender")
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

while time.time() < deadline:
    if message.state in (LXMF.LXMessage.DELIVERED, LXMF.LXMessage.SENT):
        print(
            json.dumps(
                {
                    "state": int(message.state),
                    "destination": destination_hash_hex,
                    "source": RNS.hexrep(source.hash, delimit=False).lower(),
                    "message_id": RNS.hexrep(message.hash, delimit=False).lower(),
                    "transient_id": (
                        RNS.hexrep(message.transient_id, delimit=False).lower()
                        if message.transient_id is not None
                        else None
                    ),
                    "method": message_method,
                }
            )
        )
        raise SystemExit(0)
    time.sleep(0.2)

raise SystemExit(f"timed out waiting for Python message delivery, state={message.state}")
PY
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

PY_MESSAGE_TRANSIENT_ID="$("${PYTHON_BIN}" - <<'PY' "${PY_SEND_LOG}"
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
print(payload.get("transient_id") or "")
PY
)"

if [[ "${SCENARIO}" == "propagated_resource_lxm" ]]; then
  for _ in $(seq 1 "${TIMEOUT_SECS}"); do
    if "${PYTHON_BIN}" - <<'PY' "${RUST_DB}" "${PY_MESSAGE_TRANSIENT_ID}" "${RUST_DELIVERY_HASH}"; then
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
    "${RUST_DELIVERY_HASH}" \
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
runner_milestone "rust-message-persisted"

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
    scenario,
    correlation_id,
) = sys.argv[1:15]

report = {
    "status": "pass",
    "scenario": scenario,
    "correlation_id": correlation_id,
    "proof": {
        "python_to_rust_inbound_content": py_message_content,
        "python_sender_source_hash": py_sender_source_hash,
        "python_message_id": py_message_id,
        "python_message_transient_id": py_message_transient_id,
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

if [[ "${SCENARIO}" == "propagated_resource_lxm" ]]; then
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
runner_artifact "datastore-proof" "${DATASTORE_PROOF_PATH}"
runner_artifact "rust-daemon-log" "${RUST_LOG}"
runner_artifact "python-daemon-log" "${PY_LOG}"

echo "[python-lxmd-rust-lxmd-smoke] pass"
echo "[python-lxmd-rust-lxmd-smoke] scenario=${SCENARIO}"
echo "[python-lxmd-rust-lxmd-smoke] report=${REPORT_PATH}"
echo "[python-lxmd-rust-lxmd-smoke] logs=${TMP_ROOT}"
