#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

MODE="${1:-ghost}"
case "$MODE" in
  ghost) ARGS=(--ghost) ;;
  standard) ARGS=() ;;
  portable)
    ROOT="${2:-$PWD/.local/styrene-portable}"
    ARGS=(--portable "$ROOT")
    ;;
  *)
    echo "usage: $0 [ghost|standard|portable [DIR]]" >&2
    exit 2
    ;;
esac

if [[ ! -t 0 || ! -t 1 ]]; then
  echo "Styrene TUI requires an interactive terminal." >&2
  exit 1
fi

export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"
exec cargo run --release -p styrene --features tui -- "${ARGS[@]}"
