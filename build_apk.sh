#!/usr/bin/env bash
set -euo pipefail

# TRUSWAP Pure Native Rust APK Builder (cargo-apk)
export ANDROID_HOME="${ANDROID_HOME:-/home/user/Android/Sdk}"
export ANDROID_NDK_ROOT="${ANDROID_NDK_ROOT:-$ANDROID_HOME/ndk/26.2.11394342}"
export NDK_HOME="$ANDROID_NDK_ROOT"

echo "=== Building TRUSWAP Native Android APK (cargo-apk) ==="
cargo apk build --release --lib

OUTPUT_APK="target/release/apk/truswap.apk"
if [ -f "$OUTPUT_APK" ]; then
    echo "=== APK Successfully Built ==="
    ls -lh "$OUTPUT_APK"
    "$ANDROID_HOME/build-tools/34.0.0/apksigner" verify -v "$OUTPUT_APK"
    echo "Path: $(realpath "$OUTPUT_APK")"
else
    echo "Error: Output APK not found!" >&2
    exit 1
fi
