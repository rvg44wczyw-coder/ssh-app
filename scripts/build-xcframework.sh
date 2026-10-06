#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

echo "=== 1. Building cdylib for UniFFI bindgen ==="
cargo build -p ssh-core --lib

echo "=== 2. Generating Swift bindings and C headers ==="
mkdir -p "${ROOT_DIR}/target/uniffi-out"
cargo run -p ssh-core --bin uniffi-bindgen --features cli -- \
    generate "${ROOT_DIR}/target/debug/libssh_core.dylib" \
    --language swift \
    --out-dir "${ROOT_DIR}/target/uniffi-out"

echo "=== 3. Compiling Rust static libraries for Apple targets ==="
cargo build -p ssh-core --release --target aarch64-apple-ios
cargo build -p ssh-core --release --target aarch64-apple-ios-sim
cargo build -p ssh-core --release --target aarch64-apple-darwin

echo "=== 4. Preparing headers directory for XCFramework ==="
mkdir -p "${ROOT_DIR}/target/uniffi-headers"
cp "${ROOT_DIR}/target/uniffi-out/ssh_coreFFI.h" "${ROOT_DIR}/target/uniffi-headers/"
cp "${ROOT_DIR}/target/uniffi-out/ssh_coreFFI.modulemap" "${ROOT_DIR}/target/uniffi-headers/module.modulemap"

echo "=== 5. Assembling ssh_coreFFI.xcframework ==="
OUTPUT_FRAMEWORK="${ROOT_DIR}/packages/SshCoreBridge/Frameworks/ssh_coreFFI.xcframework"
rm -rf "${OUTPUT_FRAMEWORK}"

xcodebuild -create-xcframework \
    -library "${ROOT_DIR}/target/aarch64-apple-ios/release/libssh_core.a" \
    -headers "${ROOT_DIR}/target/uniffi-headers" \
    -library "${ROOT_DIR}/target/aarch64-apple-ios-sim/release/libssh_core.a" \
    -headers "${ROOT_DIR}/target/uniffi-headers" \
    -library "${ROOT_DIR}/target/aarch64-apple-darwin/release/libssh_core.a" \
    -headers "${ROOT_DIR}/target/uniffi-headers" \
    -output "${OUTPUT_FRAMEWORK}"

echo "=== 6. Updating Swift Package sources ==="
mkdir -p "${ROOT_DIR}/packages/SshCoreBridge/Sources/SshCoreBridge"
cp "${ROOT_DIR}/target/uniffi-out/ssh_core.swift" \
   "${ROOT_DIR}/packages/SshCoreBridge/Sources/SshCoreBridge/SshCore.swift"

echo "=== UniFFI XCFramework and Swift Package built successfully! ==="
