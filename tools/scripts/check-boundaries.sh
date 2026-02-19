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

if rg -n "crates/internal/lxmf-legacy|crates/internal/reticulum-legacy" Cargo.toml >/dev/null; then
  fail "Workspace membership must not include crates/internal legacy crates"
fi

# Core crates keep a minimal direct dependency surface.
check_forbidden_dep "crates/libs/lxmf-core/Cargo.toml" "tokio"
check_forbidden_dep "crates/libs/lxmf-core/Cargo.toml" "clap"
check_forbidden_dep "crates/libs/lxmf-core/Cargo.toml" "ureq"
check_forbidden_dep "crates/libs/lxmf-core/Cargo.toml" "serde_json"
check_forbidden_dep "crates/libs/lxmf-core/Cargo.toml" "lxmf_legacy"
if rg -n "internal/lxmf-legacy" "crates/libs/lxmf-core/Cargo.toml" >/dev/null; then
  fail "crates/libs/lxmf-core/Cargo.toml must not reference internal/lxmf-legacy"
fi

check_forbidden_dep "crates/libs/rns-core/Cargo.toml" "tokio"
check_forbidden_dep "crates/libs/rns-core/Cargo.toml" "clap"

for manifest in crates/libs/*/Cargo.toml; do
  if rg -n "lxmf_legacy|reticulum_legacy|crates/internal/" "$manifest" >/dev/null; then
    fail "${manifest} must not reference legacy shim crates"
  fi
done

# Keep core crates free of direct runtime/CLI imports at source level.
if rg -n "\b(tokio|clap)::" crates/libs/lxmf-core/src >/dev/null; then
  fail "crates/libs/lxmf-core/src imports tokio/clap symbols"
fi
if rg -n "\b(tokio|clap)::" crates/libs/rns-core/src >/dev/null; then
  fail "crates/libs/rns-core/src imports tokio/clap symbols"
fi

# Public libraries must not depend directly on app crates.
for manifest in crates/libs/*/Cargo.toml; do
  if rg -n "path\s*=\s*\"\.\./\.\./apps/" "$manifest" >/dev/null; then
    fail "${manifest} must not depend on crates/apps/*"
  fi
done

echo "boundary checks: ok"
