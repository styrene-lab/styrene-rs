#!/usr/bin/env bash
# Rust native NomadNet client against a pinned Python NomadNet node.
#
# A Python NomadNet daemon serves a staged page and file tree. The Rust daemon
# browses it through the styrene CLI: a static page, a dynamic page with a
# submitted field, an allow-listed page that names the Rust identity, an
# allow-listed page that does not, and a file download with integrity.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

PYTHON_BIN="${PYTHON_BIN:-python3}"
LOG_DIR="${LOG_DIR:-${REPO_ROOT}/target/interop/rust-nomadnet-python-node}"
REPORT_PATH="${REPORT_PATH:-${LOG_DIR}/report.json}"
TIMEOUT_SECS="${TIMEOUT_SECS:-45}"
LOG_LIMIT_BYTES="${LOG_LIMIT_BYTES:-2097152}"
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

wait_for_file() {
  local file="$1"
  local timeout="$2"
  local start
  start="$(date +%s)"
  while [[ ! -s "${file}" ]]; do
    if (( "$(date +%s)" - start >= timeout )); then
      return 1
    fi
    sleep 1
  done
}

rns_identity_hash() {
  "${PYTHON_BIN}" - <<'PY' "$1"
import os
import sys
import tempfile

import RNS

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
identity = RNS.Identity.from_file(sys.argv[1])
if identity is None:
    raise SystemExit(f"failed to load identity from {sys.argv[1]}")
print(RNS.hexrep(identity.hash, delimit=False).lower())
PY
}

rns_destination_hash() {
  "${PYTHON_BIN}" - <<'PY' "$1" "$2" "$3"
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
RUST_DIR="${TMP_ROOT}/rust-client"
PY_NN_DIR="${TMP_ROOT}/python-nomadnet"
PY_RNS_DIR="${TMP_ROOT}/python-rns"
RUST_LOG="${TMP_ROOT}/rust-client.log"
PY_LOG="${TMP_ROOT}/python-node.log"
PROOF_PATH="${TMP_ROOT}/nomadnet-client-proof.json"
RESULTS_DIR="${TMP_ROOT}/results"
RUST_SOCKET_DIR="$(mktemp -d /tmp/styrene-nnc.XXXXXX)"
RUST_SOCKET="${RUST_SOCKET_DIR}/rust.sock"

cleanup_child() {
  local pid="$1"
  local deadline
  kill -TERM "${pid}" >/dev/null 2>&1 || true
  deadline=$((SECONDS + 3))
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
  rm -rf "${RUST_SOCKET_DIR}"
  if [[ ${status} -ne 0 ]]; then
    echo "[rust-nomadnet-python-node-smoke] failed" >&2
    echo "[rust-nomadnet-python-node-smoke] logs=${TMP_ROOT}" >&2
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

mkdir -p "${RUST_DIR}" "${PY_NN_DIR}/storage/pages" "${PY_NN_DIR}/storage/files" "${PY_RNS_DIR}" "${RESULTS_DIR}"
runner_milestone "topology-configured"

cat > "${RUST_DIR}/config.toml" <<EOF
role = "full_node"
EOF

cat > "${PY_RNS_DIR}/config" <<EOF
[reticulum]
  enable_transport = false
  share_instance = no
  discover_interfaces = false
  autoconnect_discovered_interfaces = 0

[logging]
  loglevel = 4

[interfaces]
  [[Rust Client]]
    type = TCPClientInterface
    enabled = yes
    target_host = ${RUST_TRANSPORT_HOST}
    target_port = ${RUST_TRANSPORT_PORT}
EOF

cargo build --manifest-path "${REPO_ROOT}/Cargo.toml" -p styrened -p styrene --bin styrened --bin styrene --quiet

RUST_IDENTITY="${RUST_DIR}/identity"
(
  STYRENE_CONFIG_DIR="${RUST_DIR}" \
  LXMF_DISPLAY_NAME="Rust NomadNet Client" \
    "${STYRENED_BIN}" \
    --rpc "${RUST_RPC_ADDR}" \
    --db "${RUST_DIR}/messages.db" \
    --identity "${RUST_IDENTITY}" \
    --config "${RUST_DIR}/config.toml" \
    --socket "${RUST_SOCKET}" \
    --transport "${RUST_TRANSPORT_ADDR}" \
    --announce-interval-secs 1 > >(bounded_log "${RUST_LOG}" "${LOG_LIMIT_BYTES}") 2>&1
) &
RUST_PID=$!

if ! wait_for_file_pattern "${RUST_LOG}" "listening on http://|delivery destination hash=" "${TIMEOUT_SECS}"; then
  echo "Rust client daemon did not become ready" >&2
  exit 1
fi
runner_milestone "rust-ready"

RUST_IDENTITY_HASH="$(rns_identity_hash "${RUST_IDENTITY}")"

# Stage the Python node's pages after the Rust identity exists so the allow
# list can name it. A second allow list names nobody the Rust node knows.
STATIC_CONTENT=">Python Static Page
Served by the pinned NomadNet node. Correlation ${STYRENE_INTEROP_CORRELATION_ID}."
PRIVATE_CONTENT=">Python Private Page
Only the allow-listed Rust identity may read this."
SECRET_CONTENT=">Python Secret Page
Nobody in this run is allowed here."
printf '%s\n' "${STATIC_CONTENT}" > "${PY_NN_DIR}/storage/pages/index.mu"
printf '%s\n' "${PRIVATE_CONTENT}" > "${PY_NN_DIR}/storage/pages/private.mu"
printf '%s\n' "${RUST_IDENTITY_HASH}" > "${PY_NN_DIR}/storage/pages/private.mu.allowed"
printf '%s\n' "${SECRET_CONTENT}" > "${PY_NN_DIR}/storage/pages/secret.mu"
printf '%s\n' "00000000000000000000000000000000" > "${PY_NN_DIR}/storage/pages/secret.mu.allowed"
cat > "${PY_NN_DIR}/storage/pages/dynamic.mu" <<'EOF'
#!/bin/sh
printf '>Python Dynamic Page\nremote=%s\nfield=%s\n' "${remote_identity:-none}" "${field_name:-none}"
EOF
chmod 0755 "${PY_NN_DIR}/storage/pages/dynamic.mu"
head -c 3000 /dev/urandom > "${PY_NN_DIR}/storage/files/manual.bin"
FILE_SHA256="$("${PYTHON_BIN}" -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "${PY_NN_DIR}/storage/files/manual.bin")"

cat > "${PY_NN_DIR}/config" <<EOF
[logging]
loglevel = 5

[client]
enable_client = yes
announce_at_start = no

[node]
enable_node = yes
node_name = Python Smoke Node
announce_at_start = yes
announce_interval = 1
disable_propagation = yes
pages_path = ${PY_NN_DIR}/storage/pages
files_path = ${PY_NN_DIR}/storage/files
EOF

# NomadNet exposes its entry point as nomadnet.nomadnet:main rather than a
# __main__ module, so invoke it the way the console script does.
(
  PYTHONUNBUFFERED=1 "${PYTHON_BIN}" -c 'import sys; from nomadnet.nomadnet import main; sys.argv[0] = "nomadnet"; main()' \
    --daemon --console \
    --config "${PY_NN_DIR}" \
    --rnsconfig "${PY_RNS_DIR}" > >(bounded_log "${PY_LOG}" "${LOG_LIMIT_BYTES}") 2>&1
) &
PY_PID=$!

if ! wait_for_file "${PY_NN_DIR}/storage/identity" "${TIMEOUT_SECS}"; then
  echo "Python NomadNet node did not create its identity" >&2
  exit 1
fi
runner_milestone "python-ready"

PY_NODE_HASH="$(rns_destination_hash "${PY_NN_DIR}/storage/identity" "nomadnetwork" "node")"

browse() {
  local name="$1"
  local path="$2"
  shift 2
  for _ in 1 2 3 4 5 6; do
    if "${STYRENE_CLI_BIN}" --socket "${RUST_SOCKET}" page "${PY_NODE_HASH}" "${path}" \
        --timeout "${TIMEOUT_SECS}" "$@" > "${RESULTS_DIR}/${name}.json" 2> "${RESULTS_DIR}/${name}.stderr"; then
      if "${PYTHON_BIN}" -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if d.get("outcome")=="succeeded" else 1)' "${RESULTS_DIR}/${name}.json"; then
        return 0
      fi
    fi
    sleep 2
  done
  return 1
}

if ! browse static "/page/index.mu"; then
  echo "Rust client could not fetch the static page from the Python node" >&2
  cat "${RESULTS_DIR}/static.stderr" >&2 || true
  exit 1
fi
runner_milestone "nomadnet-client-static"
if ! browse dynamic "/page/dynamic.mu" --field "name=rust"; then
  echo "Rust client could not fetch the dynamic page from the Python node" >&2
  exit 1
fi
runner_milestone "nomadnet-client-dynamic"
if ! browse allowed "/page/private.mu"; then
  echo "Rust client could not fetch the allow-listed page from the Python node" >&2
  exit 1
fi
runner_milestone "nomadnet-client-allowed"
if ! browse denied "/page/secret.mu"; then
  echo "Rust client did not receive the Python node's denial page" >&2
  exit 1
fi
runner_milestone "nomadnet-client-denied"
if ! "${STYRENE_CLI_BIN}" --socket "${RUST_SOCKET}" file "${PY_NODE_HASH}:/file/manual.bin" \
    --timeout "${TIMEOUT_SECS}" --expected-sha256 "${FILE_SHA256}" > "${RESULTS_DIR}/file.json" 2> "${RESULTS_DIR}/file.stderr"; then
  echo "Rust client could not download the file from the Python node" >&2
  cat "${RESULTS_DIR}/file.stderr" >&2 || true
  exit 1
fi
runner_milestone "nomadnet-client-file"

"${PYTHON_BIN}" - <<'PY' \
  "${RESULTS_DIR}" \
  "${STATIC_CONTENT}" \
  "${PRIVATE_CONTENT}" \
  "${RUST_IDENTITY_HASH}" \
  "${PY_NODE_HASH}" \
  "${FILE_SHA256}" \
  "${PROOF_PATH}" \
  "${STYRENE_INTEROP_CORRELATION_ID}"
import hashlib
import json
import sys
from pathlib import Path

(
    results_dir,
    static_content,
    private_content,
    rust_identity,
    node_hash,
    file_sha256,
    proof_path,
    correlation_id,
) = sys.argv[1:9]
results = {name: json.loads(Path(results_dir, f"{name}.json").read_text(encoding="utf-8")) for name in ("static", "dynamic", "allowed", "denied", "file")}
static_expected = hashlib.sha256((static_content + "\n").encode("utf-8")).hexdigest()
private_expected = hashlib.sha256((private_content + "\n").encode("utf-8")).hexdigest()


def check(name, condition):
    if not condition:
        raise SystemExit(f"Rust NomadNet client evidence failed: {name}: {json.dumps(results[name.split('.')[0]])[:600]}")


check("static", results["static"]["outcome"] == "succeeded" and results["static"]["source_sha256"] == static_expected)
check("static.host", results["static"]["host_hash"] == node_hash)
check("dynamic", results["dynamic"]["outcome"] == "succeeded")
check("dynamic.field", "field=rust" in results["dynamic"]["source_text"])
check("dynamic.identity", f"remote={rust_identity}" in results["dynamic"]["source_text"])
check("allowed", results["allowed"]["outcome"] == "succeeded" and results["allowed"]["source_sha256"] == private_expected)
check("denied", results["denied"]["outcome"] == "succeeded" and "Request Not Allowed" in results["denied"]["source_text"])
check("file", results["file"]["state"] in ("completed", "saved") and results["file"]["sha256"] == file_sha256)
check("file.integrity", results["file"]["integrity_verified"] is True)

proof = {
    "correlation_id": correlation_id,
    "rust_identity": rust_identity,
    "python_node": node_hash,
    "expected": {
        "static_sha256": static_expected,
        "private_sha256": private_expected,
        "file_sha256": file_sha256,
    },
    "results": results,
    "scenario": "nomadnet_client",
}
with open(proof_path, "w", encoding="utf-8") as handle:
    json.dump(proof, handle, sort_keys=True, separators=(",", ":"))
    handle.write("\n")
PY
runner_assertion "rust-to-python-nomadnet-pages"

"${PYTHON_BIN}" - <<'PY' "${REPORT_PATH}" "${TMP_ROOT}" "${RUST_LOG}" "${PY_LOG}" "${PY_NODE_HASH}" "${RUST_IDENTITY_HASH}" "${STYRENE_INTEROP_CORRELATION_ID}"
import json
import sys

report_path, tmp_root, rust_log, py_log, node_hash, rust_identity, correlation_id = sys.argv[1:8]
report = {
    "status": "pass",
    "scenario": "nomadnet_client",
    "correlation_id": correlation_id,
    "proof": {"python_node_destination": node_hash, "rust_client_identity": rust_identity},
    "logs": {"tmp_root": tmp_root, "rust_client": rust_log, "python_node": py_log},
}
with open(report_path, "w", encoding="utf-8") as handle:
    json.dump(report, handle, indent=2)
    handle.write("\n")
PY

runner_artifact "scenario-report" "${REPORT_PATH}"
runner_artifact "nomadnet-client-proof" "${PROOF_PATH}"
runner_artifact "rust-daemon-log" "${RUST_LOG}"
runner_artifact "python-node-log" "${PY_LOG}"

echo "[rust-nomadnet-python-node-smoke] pass"
echo "[rust-nomadnet-python-node-smoke] report=${REPORT_PATH}"
echo "[rust-nomadnet-python-node-smoke] logs=${TMP_ROOT}"
