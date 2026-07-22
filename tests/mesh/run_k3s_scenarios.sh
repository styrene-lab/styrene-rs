#!/usr/bin/env bash
# Execute the mesh integration suite on a remote K3s/containerd node.
#
# The controller is the only component with Kubernetes lifecycle authority.
# The operator pod has no service-account token and exercises only styrene's
# CLI over Unix sockets shared from the node.

set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
KUBECTL=${STYRENE_MESH_KUBECTL:-kubectl}
NAMESPACE=${STYRENE_MESH_NAMESPACE:-styrene-mesh-test}
NODE=${STYRENE_MESH_NODE:-}
REMOTE_ROOT=${STYRENE_MESH_REMOTE_ROOT:-/var/lib/styrene-mesh-test}
RUN_ID=${STYRENE_MESH_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)-$$}
export STYRENE_MESH_RUN_ID="$RUN_ID"
RESULT_DIR=${STYRENE_MESH_RESULT_DIR:-$ROOT/target/mesh-scenarios/$RUN_ID}
KEEP=${STYRENE_MESH_KEEP:-0}
RUNTIME_IMAGE=${STYRENE_MESH_RUNTIME_IMAGE:-rust:1.85-slim}
BUILD_TIMEOUT=${STYRENE_MESH_BUILD_TIMEOUT:-45m}
BUILD_STALL_TIMEOUT_SECONDS=${STYRENE_MESH_BUILD_STALL_TIMEOUT_SECONDS:-600}
BUILD_POLL_SECONDS=${STYRENE_MESH_BUILD_POLL_SECONDS:-10}

mkdir -p "$RESULT_DIR"
started=$(date -u +%Y-%m-%dT%H:%M:%SZ)
status=failed
operator_rc=1
resilience_rc=1

k() { "$KUBECTL" -n "$NAMESPACE" "$@"; }

collect() {
  k get pods,deployments,jobs -o wide >"$RESULT_DIR/kubernetes.txt" 2>&1 || true
  for component in hub alpha beta gamma; do
    k logs "deployment/$component" --all-containers >"$RESULT_DIR/$component.log" 2>&1 || true
  done
  k logs deployment/operator --all-containers >"$RESULT_DIR/operator-container.log" 2>&1 || true
  k logs job/build --all-containers >"$RESULT_DIR/build.log" 2>&1 || true
  cat >"$RESULT_DIR/result.json" <<EOF
{
  "schema_version": 1,
  "run_id": "$RUN_ID",
  "started_at": "$started",
  "finished_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "result": "$status",
  "operator_exit_code": $operator_rc,
  "resilience_exit_code": $resilience_rc,
  "artifacts": ["kubernetes.txt", "operator.log", "operator-container.log", "resilience.log", "hub.log", "alpha.log", "beta.log", "gamma.log", "build.log"]
}
EOF
  if [[ $KEEP != 1 ]]; then "$KUBECTL" delete namespace "$NAMESPACE" --wait=false >/dev/null 2>&1 || true; fi
}
trap collect EXIT

if [[ -z $NODE ]]; then
  NODE=$($KUBECTL get nodes --no-headers | awk '$2 == "Ready" {print $1; exit}')
fi
if [[ -z $NODE ]]; then
  echo "no Ready Kubernetes node found; set STYRENE_MESH_NODE" >&2
  exit 2
fi

$KUBECTL create namespace "$NAMESPACE" --dry-run=client -o yaml | $KUBECTL apply -f - >/dev/null

# The source tree is staged by the caller on the node. Building inside K3s
# keeps the host free of a Rust toolchain and imports no second container daemon.
cat <<EOF | $KUBECTL apply -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata: {name: build, namespace: $NAMESPACE}
spec:
  backoffLimit: 0
  template:
    spec:
      nodeName: $NODE
      restartPolicy: Never
      automountServiceAccountToken: false
      containers:
        - name: build
          image: $RUNTIME_IMAGE
          command: [bash, -ceu]
          args:
            - |
              apt-get update
              apt-get install -y --no-install-recommends pkg-config libssl-dev
              cd /source
              cargo build --release -p styrened -p styrene
              install -Dm755 target/release/styrened /artifacts/bin/styrened
              install -Dm755 target/release/styrene /artifacts/bin/styrene
          volumeMounts:
            - {name: source, mountPath: /source}
            - {name: artifacts, mountPath: /artifacts}
      volumes:
        - name: source
          hostPath: {path: $REMOTE_ROOT/source, type: Directory}
        - name: artifacts
          hostPath: {path: $REMOTE_ROOT/artifacts, type: DirectoryOrCreate}
EOF

build_started=$(date +%s)
build_last_progress=$build_started
build_last_size=-1
while :; do
  if k get job/build -o jsonpath='{.status.conditions[?(@.type=="Complete")].status}' | grep -q True; then
    break
  fi
  if k get job/build -o jsonpath='{.status.conditions[?(@.type=="Failed")].status}' | grep -q True; then
    k logs job/build >"$RESULT_DIR/build.log" 2>&1 || true
    echo "build job failed; see $RESULT_DIR/build.log" >&2
    exit 1
  fi

  now=$(date +%s)
  if [ "$((now - build_started))" -ge "$((${BUILD_TIMEOUT%m} * 60))" ]; then
    k logs job/build >"$RESULT_DIR/build.log" 2>&1 || true
    echo "build job exceeded $BUILD_TIMEOUT; see $RESULT_DIR/build.log" >&2
    exit 1
  fi

  build_log=$(k logs job/build 2>&1 || true)
  build_size=${#build_log}
  if [ "$build_size" -gt "$build_last_size" ]; then
    build_last_size=$build_size
    build_last_progress=$now
    printf '%s\n' "$build_log" >"$RESULT_DIR/build.log"
  elif [ "$((now - build_last_progress))" -ge "$BUILD_STALL_TIMEOUT_SECONDS" ]; then
    printf '%s\n' "$build_log" >"$RESULT_DIR/build.log"
    echo "build job made no log progress for ${BUILD_STALL_TIMEOUT_SECONDS}s; see $RESULT_DIR/build.log" >&2
    exit 1
  fi
  sleep "$BUILD_POLL_SECONDS"
done
k logs job/build >"$RESULT_DIR/build.log" 2>&1 || true

$KUBECTL create configmap styrene-mesh-configs -n "$NAMESPACE" \
  --from-file="$ROOT/tests/mesh/configs" --dry-run=client -o yaml | $KUBECTL apply -f - >/dev/null
$KUBECTL create configmap styrene-mesh-harness -n "$NAMESPACE" \
  --from-file="$ROOT/tests/mesh/harness.sh" \
  --from-file="$ROOT/tests/mesh/run_tests.sh" --dry-run=client -o yaml | $KUBECTL apply -f - >/dev/null
$KUBECTL create configmap styrene-mesh-scenarios -n "$NAMESPACE" \
  --from-file="$ROOT/tests/mesh/scenarios" --dry-run=client -o yaml | $KUBECTL apply -f - >/dev/null

apply_daemon() {
  local name=$1 port=$2
  cat <<EOF | $KUBECTL apply -f - >/dev/null
apiVersion: apps/v1
kind: Deployment
metadata: {name: $name, namespace: $NAMESPACE}
spec:
  replicas: 1
  selector: {matchLabels: {app: styrene-mesh-$name}}
  template:
    metadata: {labels: {app: styrene-mesh-$name}}
    spec:
      nodeName: $NODE
      automountServiceAccountToken: false
      containers:
        - name: $name
          image: $RUNTIME_IMAGE
          command: [/artifacts/bin/styrened]
          args: [--socket, /run/styrene/daemon.sock, --config, /configs/$name.toml, --transport, 0.0.0.0:$port, --announce-interval-secs, "15"]
          env:
            - {name: STYRENED_DIAGNOSTICS, value: "1"}
            - {name: LXMF_DISPLAY_NAME, value: $name}
          readinessProbe:
            exec: {command: [test, -S, /run/styrene/daemon.sock]}
            periodSeconds: 3
            failureThreshold: 20
          volumeMounts:
            - {name: artifacts, mountPath: /artifacts, readOnly: true}
            - {name: configs, mountPath: /configs, readOnly: true}
            - {name: socket, mountPath: /run/styrene}
      volumes:
        - name: artifacts
          hostPath: {path: $REMOTE_ROOT/artifacts, type: Directory}
        - name: configs
          configMap: {name: styrene-mesh-configs}
        - name: socket
          hostPath: {path: $REMOTE_ROOT/sockets/$name, type: DirectoryOrCreate}
---
apiVersion: v1
kind: Service
metadata: {name: $name, namespace: $NAMESPACE}
spec:
  selector: {app: styrene-mesh-$name}
  ports: [{name: transport, port: 4242, targetPort: $port}]
EOF
}

apply_daemon hub 4242
apply_daemon alpha 4242
apply_daemon beta 4242
apply_daemon gamma 4242

cat <<EOF | $KUBECTL apply -f - >/dev/null
apiVersion: apps/v1
kind: Deployment
metadata: {name: operator, namespace: $NAMESPACE}
spec:
  replicas: 1
  selector: {matchLabels: {app: styrene-mesh-operator}}
  template:
    metadata: {labels: {app: styrene-mesh-operator}}
    spec:
      nodeName: $NODE
      automountServiceAccountToken: false
      containers:
        - name: operator
          image: $RUNTIME_IMAGE
          command: [sleep, infinity]
          env:
            - {name: HUB_SOCK, value: /run/hub/daemon.sock}
            - {name: ALPHA_SOCK, value: /run/alpha/daemon.sock}
            - {name: BETA_SOCK, value: /run/beta/daemon.sock}
            - {name: GAMMA_SOCK, value: /run/gamma/daemon.sock}
            - {name: STYRENE_MESH_RUN_ID, value: "$RUN_ID"}
          volumeMounts:
            - {name: artifacts, mountPath: /artifacts, readOnly: true}
            - {name: harness, mountPath: /harness, readOnly: true}
            - {name: scenarios, mountPath: /harness/scenarios, readOnly: true}
            - {name: hub-sock, mountPath: /run/hub, readOnly: true}
            - {name: alpha-sock, mountPath: /run/alpha, readOnly: true}
            - {name: beta-sock, mountPath: /run/beta, readOnly: true}
            - {name: gamma-sock, mountPath: /run/gamma, readOnly: true}
      volumes:
        - name: artifacts
          hostPath: {path: $REMOTE_ROOT/artifacts, type: Directory}
        - name: harness
          configMap: {name: styrene-mesh-harness, defaultMode: 0555}
        - name: scenarios
          configMap: {name: styrene-mesh-scenarios, defaultMode: 0555}
        - name: hub-sock
          hostPath: {path: $REMOTE_ROOT/sockets/hub, type: Directory}
        - name: alpha-sock
          hostPath: {path: $REMOTE_ROOT/sockets/alpha, type: Directory}
        - name: beta-sock
          hostPath: {path: $REMOTE_ROOT/sockets/beta, type: Directory}
        - name: gamma-sock
          hostPath: {path: $REMOTE_ROOT/sockets/gamma, type: Directory}
EOF

for deployment in hub alpha beta gamma operator; do
  k rollout status "deployment/$deployment" --timeout=3m
done

operator_pod=$(k get pod -l app=styrene-mesh-operator -o jsonpath='{.items[0].metadata.name}')
k exec "$operator_pod" -- env PATH="/artifacts/bin:$PATH" bash /harness/run_tests.sh \
  >"$RESULT_DIR/operator.log.tmp" 2>&1 && operator_rc=0 || operator_rc=$?
mv "$RESULT_DIR/operator.log.tmp" "$RESULT_DIR/operator.log"
if ((operator_rc != 0)); then
  echo "operator scenarios reported failures; continuing to exercise resilience" >&2
fi

# Kubernetes owns fault injection: the operator has no API credentials and no
# container-runtime socket. Scale-to-zero/redeploy exercises process loss and
# recreation without granting test code cluster authority.
{
  echo "Suite: K3s resilience"
  k scale deployment/alpha --replicas=0
  k wait --for=delete pod -l app=styrene-mesh-alpha --timeout=90s
  echo "PASS: T21 stopped alpha workload"
  k scale deployment/alpha --replicas=1
  k rollout status deployment/alpha --timeout=3m
  echo "PASS: T22 recreated alpha workload"
  k scale deployment/hub --replicas=0
  k wait --for=delete pod -l app=styrene-mesh-hub --timeout=90s
  k exec "$operator_pod" -- env PATH="/artifacts/bin:$PATH" styrene --socket /run/alpha/daemon.sock status
  echo "PASS: T23 alpha remains healthy without hub"
  k scale deployment/hub --replicas=1
  k rollout status deployment/hub --timeout=3m
  sleep 15
  k exec "$operator_pod" -- env PATH="/artifacts/bin:$PATH" bash -ceu \
    'source /harness/harness.sh; wait_for_peer "$HUB_SOCK" alpha 60; wait_for_peer "$HUB_SOCK" beta 60'
  echo "PASS: T24 mesh recovered after hub recreation"
} >"$RESULT_DIR/resilience.log" 2>&1 && resilience_rc=0 || resilience_rc=$?
if ((resilience_rc != 0)); then
  echo "resilience scenario failed; see $RESULT_DIR/resilience.log" >&2
  exit "$resilience_rc"
fi
if ((operator_rc != 0)); then
  echo "operator scenarios failed; see $RESULT_DIR/operator.log" >&2
  exit "$operator_rc"
fi

status=passed
echo "K3s mesh scenarios passed: $RESULT_DIR"
