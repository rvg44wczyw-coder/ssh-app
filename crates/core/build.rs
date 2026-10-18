//! Embeds the typed host helper (`ssh-app-host`, macOS arm64) into the core when the build script
//! of the app has produced it at `crates/core/embedded/ssh-app-host`; otherwise an empty slice is
//! embedded and the app reports "helper not bundled" (usage then uses the Python probe).

use std::path::PathBuf;

fn main() {
    let src =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("embedded/ssh-app-host");
    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ssh-app-host");
    println!("cargo:rerun-if-changed=embedded/ssh-app-host");
    println!("cargo:rerun-if-changed=build.rs");
    let bytes = std::fs::read(&src).unwrap_or_default();
    std::fs::write(&dest, bytes).expect("write embedded helper");
}
