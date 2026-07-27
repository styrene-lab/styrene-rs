#!/usr/bin/env bash
set -euo pipefail

action=${1:?usage: remote-rpi4-build-job.sh start|status|wait|cancel USER@HOST /nix/store/NAME.drv}
host=${2:?missing USER@HOST}
drv=${3:?missing derivation}
[[ $host =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9._:-]+$ ]] || { echo "invalid builder host" >&2; exit 2; }
[[ $drv =~ ^/nix/store/[a-z0-9]{32}-[^/]+\.drv$ ]] || { echo "invalid derivation" >&2; exit 2; }
job=${drv##*/}; job=${job%.drv}
ssh_args=(-o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=15 -o ServerAliveCountMax=3)
remote_root=".local/state/styrene/build-jobs"

case "$action" in
  start)
    ssh "${ssh_args[@]}" "$host" bash -s -- "$drv" "$job" "$remote_root" <<'REMOTE'
set -euo pipefail
drv=$1 job=$2 root=$3
dir="$HOME/$root/$job"
mkdir -p "$dir"
if [[ -f $dir/status ]] && grep -qxE 'running|succeeded' "$dir/status"; then
  printf 'job=%s\nstatus=%s\n' "$job" "$(cat "$dir/status")"
  exit 0
fi
rm -f "$dir/exit-code" "$dir/output-paths" "$dir/pid" "$dir/run.sh"
printf 'running\n' > "$dir/status"
nohup setsid bash -c '
  set +e
  drv=$1; dir=$2
  if nix-store -r "$drv" > "$dir/output-paths.tmp" 2>> "$dir/build.log"; then rc=0; else rc=$?; fi
  if ((rc == 0)); then
    mv "$dir/output-paths.tmp" "$dir/output-paths"
    printf "succeeded\\n" > "$dir/status"
  else
    rm -f "$dir/output-paths.tmp"
    printf "failed\\n" > "$dir/status"
  fi
  printf "%s\\n" "$rc" > "$dir/exit-code"
  exit "$rc"
' styrene-detached-build "$drv" "$dir" </dev/null >>"$dir/build.log" 2>&1 &
pid=$!
printf '%s\n' "$pid" > "$dir/pid"
printf 'job=%s\nstatus=running\npid=%s\n' "$job" "$pid"
REMOTE
    ;;
  status)
    ssh "${ssh_args[@]}" "$host" bash -s -- "$job" "$remote_root" <<'REMOTE'
set -euo pipefail
job=$1 root=$2; dir="$HOME/$root/$job"
[[ -f $dir/status ]] || { echo "status=missing"; exit 3; }
printf 'job=%s\nstatus=%s\n' "$job" "$(cat "$dir/status")"
[[ ! -f $dir/pid ]] || printf 'pid=%s\n' "$(cat "$dir/pid")"
[[ ! -f $dir/exit-code ]] || printf 'exit_code=%s\n' "$(cat "$dir/exit-code")"
[[ ! -f $dir/output-paths ]] || sed 's/^/output=/' "$dir/output-paths"
REMOTE
    ;;
  wait)
    delay=${STYRENE_BUILDER_POLL_SECONDS:-15}
    while true; do
      set +e
      report=$($0 status "$host" "$drv" 2>&1); rc=$?
      set -e
      if ((rc != 0)); then
        echo "builder unreachable; detached job remains on $host; retrying in ${delay}s" >&2
        sleep "$delay"
        continue
      fi
      printf '%s\n' "$report"
      status=$(sed -n 's/^status=//p' <<<"$report")
      case "$status" in
        succeeded) exit 0 ;;
        failed) exit 1 ;;
        running) sleep "$delay" ;;
        *) echo "unexpected remote status: $status" >&2; exit 1 ;;
      esac
    done
    ;;
  cancel)
    ssh "${ssh_args[@]}" "$host" bash -s -- "$job" "$remote_root" <<'REMOTE'
set -euo pipefail
job=$1 root=$2; dir="$HOME/$root/$job"
[[ -f $dir/pid ]] || { echo "job has no pid" >&2; exit 1; }
pid=$(cat "$dir/pid")
kill -- "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
printf 'cancelled\n' > "$dir/status"
echo "status=cancelled"
REMOTE
    ;;
  *) echo "unknown action: $action" >&2; exit 2 ;;
esac
