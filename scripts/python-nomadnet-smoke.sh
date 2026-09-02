#!/usr/bin/env bash
# Pinned Python NomadNet client against the Rust native NomadNet host.
#
# The Rust daemon serves a staged page and file tree. A Python Reticulum client
# speaking the NomadNet request protocol fetches a static page, a dynamic page
# with submitted fields, an allow-listed page before and after identifying,
# and a file. Evidence is emitted as runner events and a retained proof.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

PYTHON_BIN="${PYTHON_BIN:-python3}"
LOG_DIR="${LOG_DIR:-${REPO_ROOT}/target/interop/python-nomadnet-rust-host}"
REPORT_PATH="${REPORT_PATH:-${LOG_DIR}/report.json}"
TIMEOUT_SECS="${TIMEOUT_SECS:-45}"
LOG_LIMIT_BYTES="${LOG_LIMIT_BYTES:-2097152}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
export CARGO_TARGET_DIR
STYRENED_BIN="${STYRENED_BIN:-${CARGO_TARGET_DIR}/debug/styrened}"

PORT_SEED="${PORT_SEED:-$$}"
SCENARIO="${SCENARIO:-nomadnet_pages}"
RUST_RPC_PORT="${RUST_RPC_PORT:-$((4243 + (PORT_SEED % 2000)))}"
RUST_TRANSPORT_PORT="${RUST_TRANSPORT_PORT:-$((37429 + (PORT_SEED % 2000)))}"
RUST_RPC_ADDR="${RUST_RPC_ADDR:-127.0.0.1:${RUST_RPC_PORT}}"
RUST_TRANSPORT_ADDR="${RUST_TRANSPORT_ADDR:-127.0.0.1:${RUST_TRANSPORT_PORT}}"
RUST_TRANSPORT_HOST="${RUST_TRANSPORT_ADDR%:*}"
RUST_TRANSPORT_PORT="${RUST_TRANSPORT_ADDR##*:}"
# The routed scenario places a Python transport hop between the client and
# the Rust host; the hop listens here.
PY_HOP_PORT="${PY_HOP_PORT:-$((39528 + (PORT_SEED % 2000)))}"

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
        echo "Usage: python-nomadnet-smoke.sh [--scenario nomadnet_pages|routed_nomadnet_pages] [--timeout SECONDS]" >&2
        exit 2
        ;;
    esac
  done
  case "${SCENARIO}" in
    nomadnet_pages|routed_nomadnet_pages) ;;
    *)
      echo "unsupported scenario: ${SCENARIO}" >&2
      exit 2
      ;;
  esac
}
parse_args "$@"
ROUTED=false
RUST_ANNOUNCE_INTERVAL_SECS="${RUST_ANNOUNCE_INTERVAL_SECS:-1}"
if [[ "${SCENARIO}" == "routed_nomadnet_pages" ]]; then
  ROUTED=true
  # A pinned Python transport hop permits about one announce per hour per
  # destination and cancels a pending path response whenever another announce
  # for that destination arrives, so the routed host announces rarely and is
  # triggered once the hop is up.
  RUST_ANNOUNCE_INTERVAL_SECS=120
fi
STYRENE_CLI_BIN="${STYRENE_CLI_BIN:-${CARGO_TARGET_DIR:-${REPO_ROOT}/target}/debug/styrene}"

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

destination_hash_from_identity() {
  local identity_path="$1"
  local aspect_one="$2"
  local aspect_two="$3"
  "${PYTHON_BIN}" - <<'PY' "${identity_path}" "${aspect_one}" "${aspect_two}"
import os
import sys
import tempfile

import RNS

identity_path, aspect_one, aspect_two = sys.argv[1:4]
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
destination = RNS.Destination(identity, RNS.Destination.IN, RNS.Destination.SINGLE, aspect_one, aspect_two)
print(RNS.hexrep(destination.hash, delimit=False).lower())
PY
}

mkdir -p "${LOG_DIR}"
TMP_ROOT="$(mktemp -d "${LOG_DIR}/run.XXXXXX")"
RUST_DIR="${TMP_ROOT}/rust-host"
PY_DIR="${TMP_ROOT}/python-client"
PY_RNS_DIR="${TMP_ROOT}/python-rns"
PY_HOP_DIR="${TMP_ROOT}/python-hop"
PY_HOP_LOG="${TMP_ROOT}/python-hop.log"
RUST_LOG="${TMP_ROOT}/rust-host.log"
PY_LOG="${TMP_ROOT}/python-client.log"
PY_RESULTS="${TMP_ROOT}/python-results.json"
PROOF_PATH="${TMP_ROOT}/nomadnet-proof.json"
RUST_SOCKET_DIR="$(mktemp -d /tmp/styrene-nn.XXXXXX)"
RUST_SOCKET="${RUST_SOCKET_DIR}/rust.sock"

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
  if [[ -n "${HOP_PID:-}" ]]; then
    cleanup_child "${HOP_PID}"
  fi
  if [[ -n "${PY_PID:-}" ]]; then
    cleanup_child "${PY_PID}"
  fi
  if [[ -n "${RUST_PID:-}" ]]; then
    cleanup_child "${RUST_PID}"
  fi
  rm -rf "${RUST_SOCKET_DIR}"
  if [[ ${status} -ne 0 ]]; then
    echo "[python-nomadnet-rust-host-smoke] failed" >&2
    echo "[python-nomadnet-rust-host-smoke] logs=${TMP_ROOT}" >&2
  fi
  runner_milestone "child-cleanup-complete"
}
trap cleanup EXIT

"${PYTHON_BIN}" - <<'PY' >/dev/null
import importlib.util
for module in ("RNS", "nomadnet"):
    if importlib.util.find_spec(module) is None:
        raise SystemExit(f"missing Python module: {module}")
PY

mkdir -p "${RUST_DIR}/pages" "${RUST_DIR}/files" "${PY_DIR}" "${PY_RNS_DIR}"
runner_milestone "topology-configured"

# The Python client identity is created first so the allow list can name it.
PY_IDENTITY_HASH="$("${PYTHON_BIN}" - <<'PY' "${PY_DIR}/identity"
import sys
import RNS

identity = RNS.Identity()
identity.to_file(sys.argv[1])
print(RNS.hexrep(identity.hash, delimit=False).lower())
PY
)"

STATIC_CONTENT=">Styrene Static Page
Served natively by the Rust host. Correlation ${STYRENE_INTEROP_CORRELATION_ID}."
PRIVATE_CONTENT=">Styrene Private Page
Only allow-listed identities may read this."
printf '%s\n' "${STATIC_CONTENT}" > "${RUST_DIR}/pages/index.mu"
printf '%s\n' "${PRIVATE_CONTENT}" > "${RUST_DIR}/pages/private.mu"
printf '%s\n' "${PY_IDENTITY_HASH}" > "${RUST_DIR}/pages/private.mu.allowed"
cat > "${RUST_DIR}/pages/dynamic.mu" <<'EOF'
#!/bin/sh
printf '>Styrene Dynamic Page\nremote=%s\nfield=%s\n' "${remote_identity:-none}" "${field_name:-none}"
EOF
chmod 0755 "${RUST_DIR}/pages/dynamic.mu"
head -c 3000 /dev/urandom > "${RUST_DIR}/files/manual.bin"
FILE_SHA256="$("${PYTHON_BIN}" -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "${RUST_DIR}/files/manual.bin")"

cat > "${RUST_DIR}/config.toml" <<EOF
role = "full_node"
EOF

CLIENT_TARGET_HOST="${RUST_TRANSPORT_HOST}"
CLIENT_TARGET_PORT="${RUST_TRANSPORT_PORT}"
if [[ "${ROUTED}" == "true" ]]; then
  CLIENT_TARGET_HOST="127.0.0.1"
  CLIENT_TARGET_PORT="${PY_HOP_PORT}"
  mkdir -p "${PY_HOP_DIR}"
  cat > "${PY_HOP_DIR}/config" <<EOF
[reticulum]
  enable_transport = true
  share_instance = no
  discover_interfaces = false
  autoconnect_discovered_interfaces = 0

[logging]
  loglevel = 4

[interfaces]
  [[Rust Host]]
    type = TCPClientInterface
    enabled = yes
    target_host = ${RUST_TRANSPORT_HOST}
    target_port = ${RUST_TRANSPORT_PORT}

  [[Client Side]]
    type = TCPServerInterface
    enabled = yes
    listen_ip = 127.0.0.1
    listen_port = ${PY_HOP_PORT}
EOF
fi

cat > "${PY_RNS_DIR}/config" <<EOF
[reticulum]
  enable_transport = false
  share_instance = no
  discover_interfaces = false
  autoconnect_discovered_interfaces = 0

[logging]
  loglevel = 4

[interfaces]
  [[Upstream]]
    type = TCPClientInterface
    enabled = yes
    target_host = ${CLIENT_TARGET_HOST}
    target_port = ${CLIENT_TARGET_PORT}
EOF

cargo build --manifest-path "${REPO_ROOT}/Cargo.toml" -p styrened -p styrene --bin styrened --bin styrene --quiet

RUST_IDENTITY="${RUST_DIR}/identity"
(
  STYRENE_CONFIG_DIR="${RUST_DIR}" \
  LXMF_DISPLAY_NAME="Rust NomadNet Host" \
    "${STYRENED_BIN}" \
    --rpc "${RUST_RPC_ADDR}" \
    --db "${RUST_DIR}/messages.db" \
    --identity "${RUST_IDENTITY}" \
    --config "${RUST_DIR}/config.toml" \
    --socket "${RUST_SOCKET}" \
    --transport "${RUST_TRANSPORT_ADDR}" \
    --announce-interval-secs "${RUST_ANNOUNCE_INTERVAL_SECS}" > >(bounded_log "${RUST_LOG}" "${LOG_LIMIT_BYTES}") 2>&1
) &
RUST_PID=$!

if ! wait_for_file_pattern "${RUST_LOG}" "listening on http://|delivery destination hash=" "${TIMEOUT_SECS}"; then
  echo "Rust host did not become ready" >&2
  exit 1
fi
runner_milestone "rust-ready"

HOP_PID=""
if [[ "${ROUTED}" == "true" ]]; then
  # A pinned Python Reticulum instance with transport enabled forwards
  # between the client and the Rust host. It has no destinations of its own.
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
  # The hop connected after the host's startup announce; announce again so
  # the hop learns the node destination and can answer client path requests.
  for _ in 1 2; do
    "${STYRENE_CLI_BIN}" --socket "${RUST_SOCKET}" announce >/dev/null 2>&1 || true
    sleep 3
  done
fi

NODE_HASH="$(destination_hash_from_identity "${RUST_IDENTITY}" "nomadnetwork" "node")"

(
exec "${PYTHON_BIN}" - <<'PY' \
  "${PY_RNS_DIR}" \
  "${PY_DIR}/identity" \
  "${NODE_HASH}" \
  "${TIMEOUT_SECS}" \
  "${PY_RESULTS}" \
  "${PY_LOG}" >"${PY_LOG}.stdout" 2>&1
import hashlib
import json
import os
import sys
import threading
import time

import RNS

rns_config, identity_path, node_hash_hex, timeout_secs, results_path, log_path = sys.argv[1:7]
timeout_secs = int(timeout_secs)
node_hash = bytes.fromhex(node_hash_hex)

RNS.logdest = RNS.LOG_FILE
RNS.logfile = log_path
RNS.Reticulum(configdir=rns_config, loglevel=int(os.environ.get("PY_CLIENT_LOGLEVEL", "6")))
identity = RNS.Identity.from_file(identity_path)
if identity is None:
    raise SystemExit("client identity is unavailable")

deadline = time.time() + timeout_secs
while time.time() < deadline:
    if RNS.Transport.has_path(node_hash) and RNS.Identity.recall(node_hash) is not None:
        break
    RNS.Transport.request_path(node_hash)
    # A transport node answers a path request after a short grace period and
    # re-arms that timer on every repeated request, so ask at a client's pace.
    time.sleep(2.5)
else:
    raise SystemExit("timed out waiting for the Rust NomadNet node path")

node_identity = RNS.Identity.recall(node_hash)
destination = RNS.Destination(
    node_identity, RNS.Destination.OUT, RNS.Destination.SINGLE, "nomadnetwork", "node"
)


def open_link():
    link = RNS.Link(destination)
    link_deadline = time.time() + timeout_secs
    while time.time() < link_deadline and link.status != RNS.Link.ACTIVE:
        time.sleep(0.1)
    if link.status != RNS.Link.ACTIVE:
        raise SystemExit("timed out establishing a link to the Rust NomadNet node")
    return link


def request(link, path, data=None, wait=None):
    outcome = {"path": path, "status": None, "response": None}
    done = threading.Event()

    def on_response(receipt):
        outcome["status"] = "ready"
        outcome["response"] = receipt.response
        done.set()

    def on_failed(receipt):
        outcome["status"] = "failed"
        done.set()

    receipt = link.request(
        path,
        data=data,
        response_callback=on_response,
        failed_callback=on_failed,
        timeout=wait or timeout_secs,
    )
    if receipt is False or receipt is None:
        outcome["status"] = "rejected"
        return outcome
    done.wait((wait or timeout_secs) + 2)
    if outcome["status"] is None:
        outcome["status"] = "timeout"
    return outcome


def text_result(outcome):
    response = outcome["response"]
    if isinstance(response, (bytes, bytearray)):
        return {
            "status": outcome["status"],
            "text": bytes(response).decode("utf-8", errors="replace"),
            "sha256": hashlib.sha256(bytes(response)).hexdigest(),
        }
    return {"status": outcome["status"], "text": None, "sha256": None}


results = {
    "client_identity": RNS.hexrep(identity.hash, delimit=False).lower(),
    "hops_to_node": RNS.Transport.hops_to(node_hash),
    "next_hop": (
        RNS.hexrep(RNS.Transport.next_hop(node_hash), delimit=False).lower()
        if RNS.Transport.next_hop(node_hash) is not None
        else None
    ),
    "next_hop_interface": str(RNS.Transport.next_hop_interface(node_hash)),
}

# Unidentified link: static, dynamic, and a denied allow-listed page.
public_link = open_link()
results["static"] = text_result(request(public_link, "/page/index.mu"))
results["dynamic_public"] = text_result(
    request(public_link, "/page/dynamic.mu", data={"field_name": "python"})
)
denied = request(public_link, "/page/private.mu", wait=8)
results["denied"] = {"status": denied["status"], "response_present": denied["response"] is not None}
# The denied request leaves an unproved packet receipt timing out inside the
# pinned client. Opening the next link in the same instant has stalled the
# client in rare local runs, so let that machinery settle first.
time.sleep(1.0)

# Identified link: the allow-listed page and the dynamic page seeing the identity.
identified_link = open_link()
identified_link.identify(identity)
time.sleep(0.5)
results["allowed"] = text_result(request(identified_link, "/page/private.mu"))
results["dynamic_identified"] = text_result(request(identified_link, "/page/dynamic.mu"))

# File download: NomadNet accepts a [name, data] pair when no metadata is sent.
file_outcome = request(identified_link, "/file/manual.bin")
file_result = {"status": file_outcome["status"], "name": None, "sha256": None, "shape": None}
response = file_outcome["response"]
if isinstance(response, (list, tuple)) and len(response) == 2:
    name, data = response
    file_result["shape"] = "pair"
    file_result["name"] = name.decode("utf-8") if isinstance(name, (bytes, bytearray)) else name
    if isinstance(data, (bytes, bytearray)):
        file_result["sha256"] = hashlib.sha256(bytes(data)).hexdigest()
elif isinstance(response, (bytes, bytearray)):
    file_result["shape"] = "bytes"
    file_result["sha256"] = hashlib.sha256(bytes(response)).hexdigest()
results["file"] = file_result

public_link.teardown()
identified_link.teardown()
with open(results_path, "w", encoding="utf-8") as handle:
    json.dump(results, handle, sort_keys=True, separators=(",", ":"))
    handle.write("\n")
PY
) &
PY_PID=$!
runner_milestone "python-ready"

if ! wait_for_file_pattern "${PY_RESULTS}" '"file"' "$((TIMEOUT_SECS * 4))"; then
  echo "Python NomadNet client did not finish its requests" >&2
  cat "${PY_LOG}.stdout" >&2 || true
  exit 1
fi
if ! wait "${PY_PID}"; then
  echo "Python NomadNet client exited with an error" >&2
  cat "${PY_LOG}.stdout" >&2 || true
  exit 1
fi
PY_PID=""

"${PYTHON_BIN}" - <<'PY' \
  "${PY_RESULTS}" \
  "${STATIC_CONTENT}" \
  "${PRIVATE_CONTENT}" \
  "${PY_IDENTITY_HASH}" \
  "${FILE_SHA256}" \
  "${PROOF_PATH}" \
  "${STYRENE_INTEROP_CORRELATION_ID}" \
  "${SCENARIO}" \
  "${ROUTED}"
import hashlib
import json
import sys
from pathlib import Path

(
    results_path,
    static_content,
    private_content,
    client_hash,
    file_sha256,
    proof_path,
    correlation_id,
    scenario,
    routed,
) = sys.argv[1:10]
expected_hops = 2 if routed == "true" else 1
results = json.loads(Path(results_path).read_text(encoding="utf-8"))
static_expected = (static_content + "\n").encode("utf-8")
private_expected = (private_content + "\n").encode("utf-8")


def check(name, condition):
    if not condition:
        raise SystemExit(f"NomadNet evidence failed: {name}: {json.dumps(results.get(name.split('.')[0]))}")


check("static", results["static"]["status"] == "ready")
check("static.content", results["static"]["sha256"] == hashlib.sha256(static_expected).hexdigest())
check("dynamic_public", results["dynamic_public"]["status"] == "ready")
check("dynamic_public.field", "field=python" in (results["dynamic_public"]["text"] or ""))
check("dynamic_public.anonymous", "remote=none" in (results["dynamic_public"]["text"] or ""))
check("denied", results["denied"]["status"] in ("failed", "timeout", "rejected"))
check("denied.no_response", results["denied"]["response_present"] is False)
check("allowed", results["allowed"]["status"] == "ready")
check("allowed.content", results["allowed"]["sha256"] == hashlib.sha256(private_expected).hexdigest())
check("dynamic_identified", results["dynamic_identified"]["status"] == "ready")
check(
    "dynamic_identified.remote",
    f"remote={client_hash}" in (results["dynamic_identified"]["text"] or ""),
)
check("file", results["file"]["status"] == "ready")
check("file.shape", results["file"]["shape"] == "pair")
check("file.name", results["file"]["name"] == "manual.bin")
check("file.sha256", results["file"]["sha256"] == file_sha256)
check("hops_to_node", results.get("hops_to_node") == expected_hops)

proof = {
    "correlation_id": correlation_id,
    "client_identity": client_hash,
    "expected": {
        "static_sha256": hashlib.sha256(static_expected).hexdigest(),
        "private_sha256": hashlib.sha256(private_expected).hexdigest(),
        "file_sha256": file_sha256,
        "file_name": "manual.bin",
        "hops_to_node": expected_hops,
    },
    "results": results,
    "scenario": scenario,
}
with open(proof_path, "w", encoding="utf-8") as handle:
    json.dump(proof, handle, sort_keys=True, separators=(",", ":"))
    handle.write("\n")
PY
runner_milestone "nomadnet-static-served"
runner_milestone "nomadnet-dynamic-served"
runner_milestone "nomadnet-denied-enforced"
runner_milestone "nomadnet-allowed-served"
runner_milestone "nomadnet-file-served"
runner_assertion "python-to-rust-nomadnet-pages"

"${PYTHON_BIN}" - <<'PY' "${REPORT_PATH}" "${TMP_ROOT}" "${RUST_LOG}" "${PY_LOG}" "${NODE_HASH}" "${PY_IDENTITY_HASH}" "${STYRENE_INTEROP_CORRELATION_ID}" "${SCENARIO}"
import json
import sys

report_path, tmp_root, rust_log, py_log, node_hash, client_hash, correlation_id, scenario = sys.argv[1:9]
report = {
    "status": "pass",
    "scenario": scenario,
    "correlation_id": correlation_id,
    "proof": {
        "rust_node_destination": node_hash,
        "python_client_identity": client_hash,
    },
    "logs": {"tmp_root": tmp_root, "rust_host": rust_log, "python_client": py_log},
}
with open(report_path, "w", encoding="utf-8") as handle:
    json.dump(report, handle, indent=2)
    handle.write("\n")
PY

runner_artifact "scenario-report" "${REPORT_PATH}"
runner_artifact "nomadnet-proof" "${PROOF_PATH}"
runner_artifact "rust-daemon-log" "${RUST_LOG}"
runner_artifact "python-client-log" "${PY_LOG}"

echo "[python-nomadnet-rust-host-smoke] pass"
echo "[python-nomadnet-rust-host-smoke] report=${REPORT_PATH}"
echo "[python-nomadnet-rust-host-smoke] logs=${TMP_ROOT}"
