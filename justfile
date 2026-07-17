# Justfile for styrene-rs development and build automation
#
# Just is a command runner - install via: brew install just / cargo install just
# Run `just` or `just --list` to see available recipes

# ─── Configuration ──────────────────────────────────────────────────────────

project_root := justfile_directory()
install_dir := env_var_or_default("STYRENE_INSTALL_DIR", env_var("HOME") + "/.cargo/bin")

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

# Build and transactionally replace local Styrene binaries (defaults to ~/.cargo/bin)
install destination=install_dir:
    cargo build --release --locked -p styrene --features tui -p styrened -p styrene-tui
    sh scripts/install-local.sh "{{ destination }}" \
        target/release/styrene \
        target/release/styrened \
        target/release/styrene-tui
    @"{{ destination }}/styrene" --version
    @"{{ destination }}/styrened" --version
    @echo "installed {{ destination }}/styrene-tui (interactive compatibility launcher)"

# Verify the local installer (preflight, success, rollback, and state preservation)
test-install:
    sh scripts/test-install-local.sh

# Package the current native target into a versioned release archive
package target=`rustc -vV | sed -n 's/^host: //p'`:
    cargo build --release --locked -p styrene --features tui -p styrened -p styrene-tui
    python3 scripts/release_artifact.py package \
        --version "$(sed -n 's/^version = \"\([^\"]*\)\"/\1/p' crates/apps/styrene/Cargo.toml | head -1)" \
        --target "{{ target }}" \
        --commit "$(git rev-parse HEAD)" \
        --rust-version "$(sed -n 's/^channel = \"\([^\"]*\)\"/\1/p' rust-toolchain.toml)" \
        --binary-dir target/release \
        --output-dir target/artifacts

# Safely validate and execute a packaged release archive
verify-artifact archive:
    python3 scripts/release_artifact.py verify "{{ archive }}"

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
    cargo clippy --workspace --all-targets --no-deps --exclude styrene-dx
    cargo test --workspace --exclude styrene-dx --exclude styrene-e2e

# ─── Hub Deployment ───────────────────────────────────────────────────────

hub_image := "ghcr.io/styrene-lab/styrened-hub"
hub_tag := `git rev-parse --short HEAD`

# Build hub image via podman (cross-compile in container — slow on ARM hosts)
hub-build:
    podman build --platform linux/amd64 -f deploy/Dockerfile.hub -t {{hub_image}}:{{hub_tag}} -t {{hub_image}}:latest .

# Build hub image fast: static musl binary via zigbuild, alpine runtime
hub-build-fast:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Cross-compiling static musl binary for x86_64-linux..."
    rustup target add x86_64-unknown-linux-musl 2>/dev/null || true
    cargo zigbuild --release --target x86_64-unknown-linux-musl -p styrened -p styrene
    echo "Packaging into alpine container..."
    CTX=$(mktemp -d)
    cp target/x86_64-unknown-linux-musl/release/styrened "$CTX/"
    cp target/x86_64-unknown-linux-musl/release/styrene "$CTX/"
    cp -r deploy/pages "$CTX/pages"
    cat > "$CTX/Dockerfile" << 'DOCKERFILE'
    FROM alpine:3.21
    RUN addgroup -S styrene && adduser -S -G styrene -h /var/lib/styrene styrene
    COPY styrened /usr/local/bin/styrened
    COPY styrene /usr/local/bin/styrene
    COPY pages/ /etc/styrene/pages/
    ENV STYRENE_CONFIG_DIR=/etc/styrene
    ENV STYRENE_DATA_DIR=/var/lib/styrene
    RUN mkdir -p /var/lib/styrene /run/styrene \
        && chown -R styrene:styrene /var/lib/styrene /run/styrene /etc/styrene/pages
    USER styrene
    EXPOSE 4242
    ENTRYPOINT ["styrened"]
    CMD ["--transport", "0.0.0.0:4242", "--rpc", "0.0.0.0:4243"]
    DOCKERFILE
    podman build --platform linux/amd64 -t {{hub_image}}:{{hub_tag}} -t {{hub_image}}:latest "$CTX"
    rm -rf "$CTX"
    echo "Done: {{hub_image}}:latest"

# Push hub image to container registry
hub-push:
    gh auth token | podman login ghcr.io -u styrene-lab --password-stdin
    podman push {{hub_image}}:{{hub_tag}}
    podman push {{hub_image}}:latest

# Build and push hub in one step (fast path)
hub-ship: hub-build-fast hub-push hub-deploy

# Deploy hub to k3s cluster (applies all manifests)
hub-deploy:
    KUBECONFIG=~/.kube/brutus.yaml kubectl apply -k deploy/k3s/
    KUBECONFIG=~/.kube/brutus.yaml kubectl -n styrene rollout restart deployment/styrene-hub

# Show hub status on the cluster
hub-status:
    KUBECONFIG=~/.kube/brutus.yaml kubectl -n styrene get pods,svc,pvc

# Stream hub logs
hub-logs:
    KUBECONFIG=~/.kube/brutus.yaml kubectl -n styrene logs -f deployment/styrene-hub

# Tear down hub deployment
hub-destroy:
    KUBECONFIG=~/.kube/brutus.yaml kubectl delete -k deploy/k3s/

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

# ─── Constrained-device simulation ────────────────────────────────────────

# Build an arm64 Linux image approximating the R36S userspace envelope
sim-r36s-build:
    ./scripts/smoke-r36s-sim.sh build

# Smoke-test version, persistent setup, and Ghost lifecycle under bounded resources
sim-r36s-smoke:
    ./scripts/smoke-r36s-sim.sh smoke

# Open an interactive shell in the bounded arm64 simulation image
sim-r36s-shell:
    ./scripts/smoke-r36s-sim.sh shell

# Characterize the simulated R36S userspace across descending memory ceilings
sim-r36s-characterize:
    ./scripts/characterize-r36s-sim.sh

# ─── Raspberry Pi materialization ─────────────────────────────────────────

# Validate the machine-readable repeatable-flasher contract
nix-rpi4-builder-contract:
    python3 scripts/validate_flashers.py product/flashers/rpi4b-builder-v1.toml

# Validate all declarative flasher contracts
nix-flasher-contracts:
    python3 scripts/validate_flashers.py product/flashers/rpi4b-builder-v1.toml product/flashers/rg35xxsp-bringup-v1.toml
    python3 scripts/test_validate_flashers.py

# Discover the accepted RPi4 native ARM64 builder
nix-rpi4-builder-discover:
    ./scripts/discover-rpi4-builder.sh

# Build a flake output natively on the accepted RPi4 builder
nix-rpi4-remote-build attr out="result-rpi-build":
    ./scripts/build-on-rpi4-builder.sh "{{attr}}" "{{out}}"

# Validate bounded OEM evidence; full preservation is still required for delivery approval
rg35xxsp-oem-evidence bundle="target/device-evidence/operator-rg35xxsp-a/oem-tf1-bounded":
    ./scripts/validate-rg35xxsp-oem-evidence.py "{{bundle}}"

# Deliberately fails until a complete OEM image and checksum exist
rg35xxsp-oem-evidence-full bundle="target/device-evidence/operator-rg35xxsp-a/oem-tf1-bounded":
    ./scripts/validate-rg35xxsp-oem-evidence.py --require-full "{{bundle}}"

# RG35XXSP source-first image build remains unavailable until the flake output exists
nix-rg35xxsp-bringup-build:
    python3 scripts/validate_flashers.py product/flashers/rg35xxsp-bringup-v1.toml
    ./scripts/build-on-rpi4-builder.sh .#packages.aarch64-linux.rg35xxsp-bringup-image result-rg35xxsp-bringup

# Re-run physical builder acceptance; optionally pass a native derivation
nix-rpi4-builder-accept host="nix-builder@styrene-builder-a.local" derivation="":
    ./scripts/verify-rpi4-builder-host.sh --host "{{host}}" {{ if derivation != "" { "--derivation \"" + derivation + "\"" } else { "" } }}

# Evaluate the Raspberry Pi 4B builder SD-image composition
nix-rpi4-builder-eval: nix-rpi4-builder-contract
    STYRENE_BUILDER_SSH_KEY="${STYRENE_BUILDER_SSH_KEY:?set operator public key}" nix eval --impure .#nixosConfigurations.rpi4-builder.config.system.build.sdImage.drvPath

# Build the Raspberry Pi 4B builder SD image in the persistent Linux builder
nix-rpi4-builder-build: nix-rpi4-builder-contract
    STYRENE_BUILDER_SSH_KEY="${STYRENE_BUILDER_SSH_KEY:?set operator public key}" ./scripts/build-nix-linux.sh .#nixosConfigurations.rpi4-builder.config.system.build.sdImage result-rpi4-builder

# Verify partition filesystems and all registered Nix store contents offline
nix-rpi4-builder-verify image="result-rpi4-builder/sd-image/nixos-image-sd-card-26.11.20260713.6cdc7fc-aarch64-linux.img.zst":
    ./scripts/verify-rpi4-image.sh "{{image}}"

# Exercise all non-destructive RPi 4 flash guards
nix-rpi4-flash-test:
    ./scripts/test-flash-rpi4-image.sh

# Validate a specific removable device without writing it
nix-rpi4-flash-dry-run device image="result-rpi4-builder/sd-image/nixos-image-sd-card-26.11.20260713.6cdc7fc-aarch64-linux.img.zst":
    ./scripts/flash-rpi4-image.sh --image "{{image}}" --device "{{device}}" --confirm ERASE --dry-run

# Evaluate the Raspberry Pi 4B Styrene appliance SD-image composition
nix-rpi4-appliance-eval:
    STYRENE_APPLIANCE_SSH_KEY="${STYRENE_APPLIANCE_SSH_KEY:?set operator public key}" nix eval --impure .#nixosConfigurations.rpi4-appliance.config.system.build.sdImage.drvPath

# Build the Raspberry Pi 4B Styrene appliance SD image (requires an aarch64-linux builder)
nix-rpi4-appliance-build:
    STYRENE_APPLIANCE_SSH_KEY="${STYRENE_APPLIANCE_SSH_KEY:?set operator public key}" nix build --impure .#nixosConfigurations.rpi4-appliance.config.system.build.sdImage --out-link result-rpi4-appliance

# Bootstrap the RPi4 builder image inside the native arm64 Podman Linux VM
nix-rpi4-builder-bootstrap:
    ./scripts/build-nix-linux.sh .#nixosConfigurations.rpi4-builder.config.system.build.sdImage result-rpi4-builder

# Inspect the completed RPi4 builder image and print its digest/size metadata
nix-rpi4-builder-inspect:
    ./scripts/rpi4_image.py inspect "$(find -L result-rpi4-builder/sd-image -type f -name '*.img.zst' -print -quit)"

# Print guarded flash metadata and required confirmation token; does not write
nix-rpi4-builder-flash-plan device:
    ./scripts/rpi4_image.py flash "$(find -L result-rpi4-builder/sd-image -type f -name '*.img.zst' -print -quit)" --device "{{device}}"

# Flash only after explicit whole-disk target and exact digest-bound token
nix-rpi4-builder-flash device confirm:
    ./scripts/rpi4_image.py flash "$(find -L result-rpi4-builder/sd-image -type f -name '*.img.zst' -print -quit)" --device "{{device}}" --confirm "{{confirm}}" --execute
