#!/usr/bin/env bash
set -euo pipefail

run_dir=/tmp/styrene-resilience-contract
rm -rf "$run_dir"
mkdir -p "$run_dir/bin" "$run_dir/result"

cat > "$run_dir/bin/kubectl" <<'SH'
#!/usr/bin/env bash
if [[ " $* " == *" exec "* ]] && [[ " $* " == *"wait_for_peer"* ]]; then
  echo "simulated readiness failure" >&2
  exit 101
fi
exit 0
SH
chmod +x "$run_dir/bin/kubectl"

export KUBECTL="$run_dir/bin/kubectl"
export NAMESPACE=test
export RESULT_DIR="$run_dir/result"
export operator_pod=operator-test
export ALPHA_SOCK=/run/alpha/daemon.sock
export HUB_SOCK=/run/hub/daemon.sock
export resilience_rc=1
k() { "$KUBECTL" -n "$NAMESPACE" "$@"; }
export -f k

# Run the production resilience block through the extraction hook. It must
# preserve the non-zero readiness result rather than printing a false PASS.
set +e
RUN_K3S_SOURCE=tests/mesh/run_k3s_scenarios.sh bash tests/mesh/test_collect_artifacts.sh \
  >"$run_dir/output.log" 2>&1
rc=$?
set -e

if [[ $rc -eq 0 ]]; then
  echo "expected resilience block to fail" >&2
  cat "$run_dir/output.log" >&2
  exit 1
fi
grep -q 'FAIL: T24 mesh did not recover' "$run_dir/result/resilience.log"
if grep -q 'PASS: T24' "$run_dir/result/resilience.log"; then
  echo "false T24 pass was emitted" >&2
  exit 1
fi
printf 'resilience-failure-propagation=passed\n'
