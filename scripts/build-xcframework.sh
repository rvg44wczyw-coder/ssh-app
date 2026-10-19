#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

echo "=== 0. Building the host helper (embedded into the core) ==="
cargo build -p ssh-app-host --release --target aarch64-apple-darwin
mkdir -p "${ROOT_DIR}/crates/core/embedded"
cp "${ROOT_DIR}/target/aarch64-apple-darwin/release/ssh-app-host" "${ROOT_DIR}/crates/core/embedded/ssh-app-host"
strip -x "${ROOT_DIR}/crates/core/embedded/ssh-app-host"
cp "${ROOT_DIR}/crates/core/embedded/ssh-app-host" "${ROOT_DIR}/crates/core/embedded/ssh-app-host-aarch64-apple-darwin"

# Helper builds for the other host systems (feature 33). They need `cargo install cargo-zigbuild`,
# `zig` and `rustup target add <triple>`; a target that cannot be built is skipped (the app then has
# no helper for that host system and the older fallbacks run), it never fails the app build.
for triple in x86_64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
    dest="${ROOT_DIR}/crates/core/embedded/ssh-app-host-${triple}"
    rm -f "${dest}"
    if command -v cargo-zigbuild >/dev/null 2>&1 && command -v zig >/dev/null 2>&1 \
        && rustup target list --installed 2>/dev/null | grep -qx "${triple}" \
        && cargo zigbuild -q -p ssh-app-host --release --locked --target "${triple}" 2>/dev/null; then
        cp "${ROOT_DIR}/target/${triple}/release/ssh-app-host" "${dest}"
        case "${triple}" in *apple-darwin) strip -x "${dest}" ;; esac
        echo "    helper for ${triple}: $(wc -c < "${dest}") bytes"
    else
        echo "    helper for ${triple}: skipped"
    fi
done

# Native Windows helper (feature 34): the ConPTY multiplexer binary, stored under the helper's name.
win="x86_64-pc-windows-gnu"
dest="${ROOT_DIR}/crates/core/embedded/ssh-app-host-${win}"
rm -f "${dest}"
if command -v cargo-zigbuild >/dev/null 2>&1 && command -v zig >/dev/null 2>&1 \
    && rustup target list --installed 2>/dev/null | grep -qx "${win}" \
    && cargo zigbuild -q -p ssh-app-host-win --release --locked --target "${win}" 2>/dev/null; then
    cp "${ROOT_DIR}/target/${win}/release/ssh-app-host-win.exe" "${dest}"
    echo "    helper for ${win}: $(wc -c < "${dest}") bytes"
else
    echo "    helper for ${win}: skipped"
fi

echo "=== 1. Building cdylib for UniFFI bindgen ==="
cargo build -p ssh-core --lib

echo "=== 2. Generating Swift bindings and C headers ==="
mkdir -p "${ROOT_DIR}/target/uniffi-out"
cargo run -p ssh-core --bin uniffi-bindgen --features cli -- \
    generate "${ROOT_DIR}/target/debug/libssh_core.dylib" \
    --language swift \
    --out-dir "${ROOT_DIR}/target/uniffi-out"

echo "=== 3. Compiling Rust static libraries for Apple targets ==="
# Same minimum as the app (IPHONEOS_DEPLOYMENT_TARGET in the Xcode project); otherwise the C
# objects are built for the SDK's newest iOS and the linker warns about every one of them.
export IPHONEOS_DEPLOYMENT_TARGET=17.0
cargo build -p ssh-core --release --target aarch64-apple-ios
cargo build -p ssh-core --release --target aarch64-apple-ios-sim
unset IPHONEOS_DEPLOYMENT_TARGET
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
