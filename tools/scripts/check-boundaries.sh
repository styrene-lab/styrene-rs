#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "boundary check failed: $*" >&2
  exit 1
}

check_forbidden_dep() {
  local manifest="$1"
  local dep="$2"
  if rg -n "^\s*${dep}\s*=\s*" "$manifest" >/dev/null; then
    fail "${manifest} depends on forbidden crate '${dep}'"
  fi
}

# Core crates keep a minimal direct dependency surface.
check_forbidden_dep "crates/libs/lxmf-core/Cargo.toml" "tokio"
check_forbidden_dep "crates/libs/lxmf-core/Cargo.toml" "clap"
check_forbidden_dep "crates/libs/lxmf-core/Cargo.toml" "ureq"
check_forbidden_dep "crates/libs/lxmf-core/Cargo.toml" "serde_json"

check_forbidden_dep "crates/libs/rns-core/Cargo.toml" "tokio"
check_forbidden_dep "crates/libs/rns-core/Cargo.toml" "clap"

# Public libraries must not depend directly on app crates.
for manifest in crates/libs/*/Cargo.toml; do
  if rg -n "path\s*=\s*\"\.\./\.\./apps/" "$manifest" >/dev/null; then
    fail "${manifest} must not depend on crates/apps/*"
  fi
done

echo "boundary checks: ok"
