//! `ssh-app-host`: the typed helper the app installs on the host (feature 24, phase P0).
//!
//! It runs no shell, opens no socket (except a loopback read of Ollama's `/api/ps`) and never
//! writes a file. Stdout is one JSON object; exit status 0 means "answered".

use serde_json::json;
use ssh_app_host::usage;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("hello") => {
            println!(
                "{}",
                json!({"v": 1, "name": "ssh-app-host", "version": VERSION, "caps": ["usage"]})
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
        _ => fail("usage: ssh-app-host hello | usage [CONFIG_DIR...]"),
    };
    std::process::exit(code);
}

fn fail(message: &str) -> i32 {
    eprintln!("{message}");
    println!("{}", json!({"error": message}));
    2
}
