#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGET_DIR="$ROOT/target"
BINDINGS_DIR="$TARGET_DIR/ios-bindings"
HEADERS_DIR="$TARGET_DIR/ios-headers"
SIMULATOR_DIR="$TARGET_DIR/ios-simulator"
GENERATED_DIR="$ROOT/ios/Generated"
FRAMEWORK_DIR="$ROOT/ios/Frameworks/StyreneMobileFFI.xcframework"
LIBRARY=libstyrene_mobile_ffi.a
SIMULATOR_LIBRARY=libstyrene_mobile_ffi_sim.a

export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-17.0}"

cd "$ROOT"

cargo build --locked -p styrene-mobile-ffi \
    --no-default-features --features ios \
    --target aarch64-apple-ios --release
cargo build --locked -p styrene-mobile-ffi \
    --no-default-features --features ios \
    --target aarch64-apple-ios-sim --release
cargo build --locked -p styrene-mobile-ffi \
    --no-default-features --features ios \
    --target x86_64-apple-ios --release

rm -rf "$BINDINGS_DIR" "$HEADERS_DIR" "$SIMULATOR_DIR" "$GENERATED_DIR" "$FRAMEWORK_DIR"
mkdir -p "$BINDINGS_DIR" "$HEADERS_DIR" "$SIMULATOR_DIR" "$GENERATED_DIR"

cargo run --locked -p styrene-mobile-ffi \
    --no-default-features --features ios,bindgen \
    --bin styrene-uniffi-bindgen -- generate \
    --library "$TARGET_DIR/aarch64-apple-ios/release/$LIBRARY" \
    --language swift \
    --out-dir "$BINDINGS_DIR"

cp "$BINDINGS_DIR/styrene_mobile_ffi.swift" "$GENERATED_DIR/"
cp "$BINDINGS_DIR/styrene_mobile_ffiFFI.h" "$HEADERS_DIR/"
cp "$BINDINGS_DIR/styrene_mobile_ffiFFI.modulemap" "$HEADERS_DIR/module.modulemap"

xcrun lipo -create \
    "$TARGET_DIR/aarch64-apple-ios-sim/release/$LIBRARY" \
    "$TARGET_DIR/x86_64-apple-ios/release/$LIBRARY" \
    -output "$SIMULATOR_DIR/$SIMULATOR_LIBRARY"

xcodebuild -create-xcframework \
    -library "$TARGET_DIR/aarch64-apple-ios/release/$LIBRARY" \
    -headers "$HEADERS_DIR" \
    -library "$SIMULATOR_DIR/$SIMULATOR_LIBRARY" \
    -headers "$HEADERS_DIR" \
    -output "$FRAMEWORK_DIR"

echo "Created $FRAMEWORK_DIR"
