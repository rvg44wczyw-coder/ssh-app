//! `ssh-app-host`: the typed helper the app installs on the host (feature 24, phase P0).
//!
//! It runs no shell, opens no socket (except a loopback read of Ollama's `/api/ps`) and never
//! writes a file. Stdout is one JSON object; exit status 0 means "answered".

use serde_json::json;
use ssh_app_host::queue::{install_signal_handlers, Paths};
use ssh_app_host::{ctl, hook, statusline, usage};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("hello") => {
            println!(
                "{}",
                json!({"v": 1, "name": "ssh-app-host", "version": VERSION, "caps": ["usage", "hook"]})
            );
            0
        }
        Some("usage") => match usage::validate_dirs(&args[1..]) {
            Ok(dirs) => {
                println!("{}", usage::report(&dirs));
                0
            }
            Err(e) => fail(&e),
        },
        Some("hook") => guarded(|| {
            install_signal_handlers();
            hook::run(&Paths::from_env(), &args[1..])
        }),
        Some("statusline") => guarded(|| statusline::run(&Paths::from_env(), &args[1..])),
        Some("ctl") => {
            let reply = std::panic::catch_unwind(|| ctl::run(&Paths::from_env(), &args[1..]))
                .unwrap_or_else(|_| json!({"ok": false, "error": "internal error"}));
            println!("{reply}");
            0
        }
        _ => fail("usage: ssh-app-host hello | usage [CONFIG_DIR...] | hook AGENT EVENT | ctl SUB | statusline [PREV_HEX]"),
    };
    std::process::exit(code);
}

/// Any unexpected error means "no opinion" (exit 0, nothing printed), never a crash message.
fn guarded(f: impl FnOnce() -> i32 + std::panic::UnwindSafe) -> i32 {
    std::panic::catch_unwind(f).unwrap_or(0)
}

fn fail(message: &str) -> i32 {
    eprintln!("{message}");
    println!("{}", json!({"error": message}));
    2
}
