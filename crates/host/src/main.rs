//! `ssh-app-host`: the typed helper the app installs on the host (feature 24, phase P0).
//!
//! It runs no shell, opens no socket (except a loopback read of Ollama's `/api/ps`) and never
//! writes a file. Stdout is one JSON object; exit status 0 means "answered".

use serde_json::json;
use ssh_app_host::queue::{install_signal_handlers, Paths};
use ssh_app_host::{agent, ctl, hook, models, rpc, statusline, usage};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("hello") => {
            println!(
                "{}",
                json!({"v": 1, "name": "ssh-app-host", "version": VERSION, "caps": ["usage", "hook", "agent", "models"]})
            );
            0
        }
        Some("usage") => {
            // `--agent` asks the host agent (cached, started on demand) and falls back to scanning.
            let via_agent = args.get(1).map(String::as_str) == Some("--agent");
            let rest = &args[if via_agent { 2 } else { 1 }..];
            match usage::validate_dirs(rest) {
                Ok(dirs) => {
                    let cached = via_agent
                        .then(|| {
                            rpc::call(
                                &Paths::from_env(),
                                agent::DEFAULT_LEASE,
                                &json!({"m": "usage", "dirs": dirs}),
                            )
                        })
                        .flatten()
                        .filter(|v| v["ok"] == true)
                        .map(|v| v["report"].clone());
                    println!("{}", cached.unwrap_or_else(|| usage::report(&dirs)));
                    0
                }
                Err(e) => fail(&e),
            }
        }
        Some("models") => {
            let home = std::env::var("HOME").unwrap_or_default();
            let reply = models::reply(args.get(1).map_or("", String::as_str), &home);
            println!("{reply}");
            i32::from(reply["ok"] != true)
        }
        Some("hook") => guarded(|| {
            install_signal_handlers();
            hook::run(&Paths::from_env(), &args[1..])
        }),
        Some("statusline") => guarded(|| statusline::run(&Paths::from_env(), &args[1..])),
        Some("agent") => agent::run(Paths::from_env(), rpc::parse_lease(&args[1..]), &agent::NoopSink),
        Some("rpc") => rpc::run(&Paths::from_env(), rpc::parse_lease(&args[1..])),
        Some("ctl") => {
            let reply = std::panic::catch_unwind(|| ctl::run(&Paths::from_env(), &args[1..]))
                .unwrap_or_else(|_| json!({"ok": false, "error": "internal error"}));
            println!("{reply}");
            0
        }
        _ => fail("usage: ssh-app-host hello | usage [CONFIG_DIR...] | hook AGENT EVENT | rpc [--lease SECS] | agent | ctl SUB | statusline [PREV_HEX]"),
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
