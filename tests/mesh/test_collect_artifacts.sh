#!/usr/bin/env bash
set -euo pipefail

run_dir=/tmp/styrene-collect-contract
rm -rf /tmp/styrene-collect-contract
mkdir -p "$run_dir/bin" "$run_dir/result"
cat > "$run_dir/bin/kubectl" <<'SH'
#!/usr/bin/env bash
if [[ " $* " == *" logs deployment/operator "* ]]; then echo container-diagnostic; exit 0; fi
if [[ " $* " == *" logs deployment/"* ]]; then echo daemon-diagnostic; exit 0; fi
if [[ " $* " == *" logs job/build "* ]]; then echo build-diagnostic; exit 0; fi
if [[ " $* " == *" get pods,deployments,jobs "* ]]; then echo cluster-state; exit 0; fi
exit 0
SH
chmod +x "$run_dir/bin/kubectl"
echo scenario-transcript > "$run_dir/result/operator.log"
KUBECTL="$run_dir/bin/kubectl"
NAMESPACE=test
RESULT_DIR="$run_dir/result"
k() { "$KUBECTL" -n "$NAMESPACE" "$@"; }
k get pods,deployments,jobs -o wide >"$RESULT_DIR/kubernetes.txt"
for component in hub alpha beta gamma; do
  k logs "deployment/$component" --all-containers >"$RESULT_DIR/$component.log"
done
k logs deployment/operator --all-containers >"$RESULT_DIR/operator-container.log"
test "$(cat "$RESULT_DIR/operator.log")" = scenario-transcript
test "$(cat "$RESULT_DIR/operator-container.log")" = container-diagnostic
printf 'preservation-contract=passed\n'
