#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Detect Android NDK
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
  if [ -d "${HOME}/Library/Android/sdk/ndk/26.3.11579264" ]; then
    export ANDROID_NDK_HOME="${HOME}/Library/Android/sdk/ndk/26.3.11579264"
  elif [ -d "${ANDROID_HOME:-}/ndk" ]; then
    LATEST_NDK=$(ls -1 "${ANDROID_HOME}/ndk" | sort -V | tail -n 1)
    export ANDROID_NDK_HOME="${ANDROID_HOME}/ndk/${LATEST_NDK}"
  else
    echo "Error: ANDROID_NDK_HOME is not set and could not be detected automatically." >&2
    exit 1
  fi
fi

echo "=== Android NDK: ${ANDROID_NDK_HOME} ==="

JNI_LIBS_DIR="${ROOT_DIR}/android/app/src/main/jniLibs"
KOTLIN_SRC_DIR="${ROOT_DIR}/android/app/src/main/java"
mkdir -p "${JNI_LIBS_DIR}"
mkdir -p "${KOTLIN_SRC_DIR}"

echo "=== 1. Compiling Rust Core for Android architectures (arm64-v8a, x86_64) ==="
cargo ndk -t arm64-v8a -t x86_64 -o "${JNI_LIBS_DIR}" build --release -p ssh-core

echo "=== 2. Generating UniFFI Kotlin bindings ==="
cargo run -p ssh-core --features cli --bin uniffi-bindgen -- \
  generate "${JNI_LIBS_DIR}/arm64-v8a/libssh_core.so" \
  --language kotlin \
  --out-dir "${KOTLIN_SRC_DIR}"

echo "=== Android native libraries and Kotlin bindings generated successfully! ==="
