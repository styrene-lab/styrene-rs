# Justfile for styrene-rs development and build automation
#
# Just is a command runner - install via: brew install just / cargo install just
# Run `just` or `just --list` to see available recipes

# ─── Configuration ──────────────────────────────────────────────────────────

project_root := justfile_directory()

# ─── Help ───────────────────────────────────────────────────────────────────

# Show available recipes (default)
@default:
    just --list --unsorted

# ─── Development ────────────────────────────────────────────────────────────

# Run all tests
test:
    cargo test --workspace

# Run tests with output
test-verbose:
    cargo test --workspace -- --nocapture

# Run clippy linter
lint:
    cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings

# Format code
format:
    cargo fmt --all

# Check formatting (CI mode)
format-check:
    cargo fmt --all -- --check

# Build all crates
build:
    cargo build --workspace --all-targets

# Build in release mode
build-release:
    cargo build --workspace --release

# Run all validation checks (format + lint + test)
validate: format-check lint test

# Check all crates compile (fast, no codegen)
check:
    cargo check --workspace --all-targets

# ─── Documentation ──────────────────────────────────────────────────────────

# Generate documentation
docs:
    cargo doc --workspace --no-deps

# Generate and open documentation
docs-open:
    cargo doc --workspace --no-deps --open

# ─── Interop Testing ───────────────────────────────────────────────────────

# Run interop tests against committed fixtures (includes HDLC via transport)
test-interop:
    cargo test --package styrene-rns --features interop-tests,transport

# Generate Python test fixtures (requires Python RNS/LXMF)
generate-fixtures:
    cd tests/interop/python && python3 generate_fixtures.py

# Generate fresh fixtures then run interop tests
test-interop-full: generate-fixtures test-interop

# Run full nightly pipeline locally (validate + interop + upstream review)
nightly: validate test-interop-full upstream-review

# ─── PQC Tunnel ───────────────────────────────────────────────────────────

# Run PQC tunnel tests (crypto, session, wire types)
test-pqc:
    cargo test --package styrene-tunnel
    cargo test --package styrene-mesh --features pqc

# Build with tunnel backends enabled
build-tunnel:
    cargo build --package styrene-tunnel --features tunnel

# Check PQC tunnel compiles with all features
check-tunnel:
    cargo check --package styrene-tunnel --features tunnel

# ─── Security ──────────────────────────────────────────────────────────────

# Run cargo-deny checks (licenses, advisories, bans)
deny:
    cargo deny check

# Run security audit
audit:
    cargo audit

# ─── Upstream Tracking ────────────────────────────────────────────────────

# Review pending upstream changes (beechat + freetakteam)
upstream-review *args='':
    ./scripts/upstream-review.sh {{ args }}

# Show upstream tracking status
upstream-status:
    ./scripts/upstream-review.sh --status

# Mark current upstream HEADs as reviewed (updates .upstream-tracking.json)
upstream-advance *args='':
    ./scripts/upstream-review.sh --advance {{ args }}

# Generate upstream sync report (same as weekly CI PR body)
upstream-sync-report:
    ./scripts/upstream-sync-pr.sh --report

# ─── Cleanup ───────────────────────────────────────────────────────────────

# Clean build artifacts
clean:
    cargo clean

# ─── Release ───────────────────────────────────────────────────────────────

# Publish crates in dependency order (dry run)
publish-dry-run:
    cargo publish -p styrene-rns --dry-run
    cargo publish -p styrene-lxmf --dry-run
    cargo publish -p styrene-mesh --dry-run
    cargo publish -p styrene-ipc --dry-run
    cargo publish -p styrene-tunnel --dry-run

# Publish crates in dependency order (live)
publish:
    cargo publish -p styrene-rns
    sleep 30
    cargo publish -p styrene-lxmf
    sleep 30
    cargo publish -p styrene-mesh
    sleep 30
    cargo publish -p styrene-ipc
    sleep 30
    cargo publish -p styrene-tunnel
