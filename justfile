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

# ─── Mobile ────────────────────────────────────────────────────────────────

# Check mobile library compiles (no desktop deps)
check-mobile:
    cargo check -p styrened --no-default-features

# Check mobile with keychain identity (iOS/macOS)
check-mobile-keychain:
    cargo check -p styrened --no-default-features --features mobile-keychain

# Check mobile with encrypted file identity (Android)
check-mobile-identity:
    cargo check -p styrened --no-default-features --features mobile-identity

# Check UniFFI bridge compiles
check-ffi:
    cargo check -p styrene-mobile-ffi

# Build iOS static library (requires Xcode + iOS SDK)
build-ios:
    cargo build -p styrened --no-default-features --features mobile-keychain \
        --target aarch64-apple-ios --release

# Build iOS simulator library
build-ios-sim:
    cargo build -p styrened --no-default-features --features mobile-keychain \
        --target aarch64-apple-ios-sim --release

# Build iOS FFI bridge (static library for Swift)
build-ios-ffi:
    cargo build -p styrene-mobile-ffi \
        --target aarch64-apple-ios --release

# Build Android library (requires cargo-ndk + NDK)
build-android:
    cargo ndk -t arm64-v8a -t armeabi-v7a \
        build -p styrened --no-default-features --features mobile-identity,bundled-sqlite --release

# Build Android FFI bridge (shared library for Kotlin)
build-android-ffi:
    cargo ndk -t arm64-v8a -t armeabi-v7a \
        build -p styrene-mobile-ffi --no-default-features --features android --release

# Generate Swift bindings from UniFFI
gen-swift: build-ios-ffi
    cargo run -p uniffi-bindgen -- generate \
        --library target/aarch64-apple-ios/release/libstyrene_mobile_ffi.a \
        --language swift \
        --out-dir bindings/swift/Sources/StyreneMobile/

# Generate Kotlin bindings from UniFFI
gen-kotlin: build-android-ffi
    cargo run -p uniffi-bindgen -- generate \
        --library target/aarch64-linux-android/release/libstyrene_mobile_ffi.so \
        --language kotlin \
        --out-dir bindings/kotlin/src/main/kotlin/io/styrene/mobile/

# Screenshot the desktop app for visual feedback
screenshot-dx:
    @./scripts/screenshot-dx.sh /tmp/styrene-dx-screenshot.png
    @echo "View: open /tmp/styrene-dx-screenshot.png"

# Full iOS build: compile + generate Swift bindings
mobile-ios: build-ios-ffi gen-swift
    @echo "iOS build complete — Swift bindings in bindings/swift/"

# Full Android build: compile + generate Kotlin bindings
mobile-android: build-android-ffi gen-kotlin
    @echo "Android build complete — Kotlin bindings in bindings/kotlin/"

# Copy .so to Android project jniLibs and build APK
android-deploy: build-android-ffi
    @mkdir -p android/app/src/main/jniLibs/arm64-v8a
    cp target/aarch64-linux-android/release/libstyrene_mobile_ffi.so \
        android/app/src/main/jniLibs/arm64-v8a/
    @echo "Native library copied to android/app/src/main/jniLibs/arm64-v8a/"
    cd android && ./gradlew assembleDebug
    @echo "APK built: android/app/build/outputs/apk/debug/app-debug.apk"

# Install debug APK on connected device
android-install: android-deploy
    adb install -r android/app/build/outputs/apk/debug/app-debug.apk
    @echo "Installed on device. Launch: adb shell am start -n io.styrene.mesh/.MainActivity"

# Validate all mobile profiles compile
check-mobile-all: check-mobile check-mobile-keychain check-mobile-identity check-ffi
    @echo "All mobile profiles compile ✓"

# ─── Feature Matrix ───────────────────────────────────────────────────────

# Verify all feature combinations compile (CI matrix)
check-all-features:
    @echo "Checking default features..."
    cargo check -p styrened
    @echo "Checking no-default-features..."
    cargo check -p styrened --no-default-features
    @echo "Checking mobile-keychain..."
    cargo check -p styrened --no-default-features --features mobile-keychain
    @echo "Checking mobile-identity..."
    cargo check -p styrened --no-default-features --features mobile-identity
    @echo "Checking terminal only..."
    cargo check -p styrened --no-default-features --features terminal
    @echo "Checking ipc-server only..."
    cargo check -p styrened --no-default-features --features ipc-server
    @echo "Checking FFI bridge..."
    cargo check -p styrene-mobile-ffi
    @echo "Checking TUI..."
    cargo check -p styrene-tui
    @echo "All feature combinations compile ✓"

# ─── E2E Testing ──────────────────────────────────────────────────────────

# Run e2e integration tests
test-e2e:
    cargo test -p styrene-e2e

# Run e2e tests with output
test-e2e-verbose:
    cargo test -p styrene-e2e -- --nocapture

# Run specific e2e test file
test-e2e-file file:
    cargo test -p styrene-e2e --test {{ file }}

# ─── Release Preflight ────────────────────────────────────────────────────

# Run the exact CI checks locally before tagging a release
preflight:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --no-deps --exclude styrene-dx --exclude styrene-native
    cargo test --workspace --exclude styrene-dx --exclude styrene-native

# ─── Hub Deployment ───────────────────────────────────────────────────────

hub_image := "ghcr.io/styrene-lab/styrened-hub"
hub_tag := `git rev-parse --short HEAD`

# Build the community hub container image
hub-build:
    docker build -f deploy/Dockerfile.hub -t {{hub_image}}:{{hub_tag}} -t {{hub_image}}:latest .

# Push hub image to container registry
hub-push: hub-build
    docker push {{hub_image}}:{{hub_tag}}
    docker push {{hub_image}}:latest

# Deploy hub to k3s cluster (applies all manifests)
hub-deploy:
    kubectl apply -k deploy/k3s/

# Show hub status on the cluster
hub-status:
    kubectl -n styrene get pods,svc,pvc

# Stream hub logs
hub-logs:
    kubectl -n styrene logs -f deployment/styrene-hub

# Tear down hub deployment
hub-destroy:
    kubectl delete -k deploy/k3s/

# ─── Cleanup ───────────────────────────────────────────────────────────────

# Clean build artifacts
clean:
    cargo clean

# Clean mobile binding outputs
clean-bindings:
    rm -rf bindings/swift/Sources bindings/kotlin/src

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
