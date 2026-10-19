//! Embeds the typed host helper (`ssh-app-host`) builds into the core when the build script of the
//! app has produced them in `crates/core/embedded/`:
//!
//! * `ssh-app-host-<target triple>` for every host system the app supports (feature 33), and
//! * the legacy `ssh-app-host`, which is the `aarch64-apple-darwin` build.
//!
//! A build that is missing is simply not in the table (the app then reports "helper not bundled"
//! for that host system and the older fallbacks run).

use std::path::PathBuf;

const TRIPLES: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
];

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("embedded");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=embedded");
    println!("cargo:rerun-if-changed=build.rs");

    // The legacy single-blob API (macOS arm64).
    let legacy = std::fs::read(dir.join("ssh-app-host")).unwrap_or_default();
    std::fs::write(out.join("ssh-app-host"), &legacy).expect("write embedded helper");

    let mut table = String::from("pub static HELPERS: &[(&str, &[u8])] = &[\n");
    for triple in TRIPLES {
        let mut bytes =
            std::fs::read(dir.join(format!("ssh-app-host-{triple}"))).unwrap_or_default();
        if bytes.is_empty() && triple == "aarch64-apple-darwin" {
            bytes = legacy.clone();
        }
        if bytes.is_empty() {
            continue;
        }
        let name = format!("helper-{triple}");
        std::fs::write(out.join(&name), bytes).expect("write embedded helper");
        table.push_str(&format!(
            "    (\"{triple}\", include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{name}\"))),\n"
        ));
    }
    table.push_str("];\n");
    std::fs::write(out.join("helpers.rs"), table).expect("write helper table");
}
