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

# Run interop tests (requires Python RNS/LXMF installed)
test-interop:
    cargo test --workspace --features interop-tests

# Generate Python test fixtures
generate-fixtures:
    cd tests/interop/python && python3 generate_fixtures.py

# ─── Security ──────────────────────────────────────────────────────────────

# Run cargo-deny checks (licenses, advisories, bans)
deny:
    cargo deny check

# Run security audit
audit:
    cargo audit

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

# Publish crates in dependency order (live)
publish:
    cargo publish -p styrene-rns
    sleep 30
    cargo publish -p styrene-lxmf
    sleep 30
    cargo publish -p styrene-mesh
