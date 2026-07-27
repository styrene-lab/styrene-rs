#!/usr/bin/env bash
set -euo pipefail

# Narrow validation target for the crates that form the A2A integration boundary.
exec cargo test \
  -p styrene-a2a \
  -p styrene-ipc \
  -p styrene-ipc-server \
  -p styrene-services \
  -p styrened \
  "$@"
