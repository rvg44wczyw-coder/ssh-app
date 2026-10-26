//! `ssh-app-host`: the typed helper the app installs on the host (feature 24, phase P0).
//!
//! It runs no shell, opens no socket (except a loopback read of Ollama's `/api/ps` and a loopback
//! connect by `doctor`) and writes no file, except the queue directories of the hook and the one
//! line `pair` appends to `~/.ssh/authorized_keys`. Stdout is one JSON object; exit status 0 means "answered".

use serde_json::json;
use ssh_app_host::queue::{install_signal_handlers, Paths};
use ssh_app_host::{
    agent, agyfeed, agyquota, ctl, doctor, files, grokquota, hook, hooksview, models, pair, procs,
    push, rpc, skills, statusline, telemetry, usage,
};
use std::path::PathBuf;
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("hello") => {
            println!(
                "{}",
                json!({"v": 1, "name": "ssh-app-host", "version": VERSION, "caps": ["usage", "hook", "agent", "push", "models", "telemetry", "fs", "read", "kill", "pair", "doctor", "skills", "agy-events"]})
            );
            0
        }
        Some("agy-quota-refresh") => i32::from(
            !args
                .get(1)
                .is_some_and(|cwd| agyquota::refresh(cwd)),
        ),
        Some("grok-quota-refresh") => i32::from(
            !args
                .get(1)
                .is_some_and(|cwd| grokquota::refresh(cwd)),
        ),
        Some("hooks-overview") => {
            let folders: Vec<String> = args[1..]
                .windows(2)
                .filter(|w| w[0] == "--folder")
                .map(|w| w[1].clone())
                .collect();
            let claude_dir = args[1..]
                .windows(2)
                .find(|w| w[0] == "--claude-dir")
                .map(|w| w[1].as_str());
            println!("{}", hooksview::overview(&folders, claude_dir));
            0
        }
        Some("agy-jsonl") if args.last().map(String::as_str) == Some("--raw") => {
            match agyfeed::jsonl_raw(&args[1..args.len() - 1]) {
                Some(out) => {
                    print!("{out}");
                    0
                }
                None => 1,
            }
        }
        Some("agy-conv") if args.last().map(String::as_str) == Some("--raw") => {
            match agyfeed::conv_raw(&args[1..args.len() - 1]) {
                Some(id) => {
                    println!("{id}");
                    0
                }
                None => 1,
            }
        }
        Some("agy-jsonl") => {
            let reply = agyfeed::jsonl_reply(&args[1..]);
            println!("{reply}");
            i32::from(reply["ok"] != true)
        }
        Some("agy-children") => {
            println!("{}", agyfeed::children_reply(&args[1..]));
            0
        }
        Some("agy-conv") => {
            let reply = agyfeed::conv_reply(&args[1..]);
            println!("{reply}");
            i32::from(reply["ok"] != true)
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
        Some("ls") => {
            let reply = ls_reply(&args[1..]);
            println!("{reply}");
            i32::from(reply["ok"] != true)
        }
        Some("read") => {
            let reply = read_reply(&args[1..]);
            println!("{reply}");
            i32::from(reply["ok"] != true)
        }
        Some("kill") => {
            let reply = kill_reply(&args[1..]);
            println!("{reply}");
            i32::from(reply["ok"] != true)
        }
        Some("doctor") => {
            println!("{}", doctor::doctor(VERSION));
            0
        }
        Some("pair") => {
            let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
            let reply = pair::pair(&home, &mut std::io::stdin());
            println!("{reply}");
            i32::from(reply["ok"] != true)
        }
        Some("telemetry") => {
            println!("{}", telemetry::snapshot());
            0
        }
        Some("models") => {
            let home = std::env::var("HOME").unwrap_or_default();
            let reply = models::reply(args.get(1).map_or("", String::as_str), &home);
            println!("{reply}");
            i32::from(reply["ok"] != true)
        }
        Some("skills") => {
            // One JSON request on stdin (at most a skill and a little more).
            let mut input = Vec::new();
            let _ = std::io::Read::read_to_end(
                &mut std::io::Read::take(std::io::stdin(), 2 * skills::MAX_SKILL_BYTES as u64),
                &mut input,
            );
            let reply = match serde_json::from_slice::<serde_json::Value>(&input) {
                Ok(req) => skills::handle(&req, &std::env::var("HOME").unwrap_or_default()),
                Err(_) => json!({"ok": false, "error": "the request is not JSON"}),
            };
            println!("{reply}");
            i32::from(reply["ok"] != true)
        }
        Some("hook") => guarded(|| {
            install_signal_handlers();
            hook::run(&Paths::from_env(), &args[1..])
        }),
        Some("statusline") => guarded(|| statusline::run(&Paths::from_env(), &args[1..])),
        Some("agent") => {
            let paths = Paths::from_env();
            let sink = push::PushSink::new(Paths::from_env(), Arc::new(push::CurlSender));
            agent::run(paths, rpc::parse_lease(&args[1..]), &sink)
        }
        Some(cmd @ ("push-register" | "push-unregister" | "push-status" | "push-test")) => {
            let paths = Paths::from_env();
            let reply = match cmd {
                "push-register" => push::register(&paths, &mut std::io::stdin()),
                "push-unregister" => push::unregister(&paths),
                "push-status" => push::status(&paths),
                _ => push::test(&paths, &push::CurlSender),
            };
            println!("{reply}");
            i32::from(reply["ok"] != true)
        }
        Some("rpc") => rpc::run(&Paths::from_env(), rpc::parse_lease(&args[1..])),
        Some("ctl") => {
            let reply = std::panic::catch_unwind(|| ctl::run(&Paths::from_env(), &args[1..]))
                .unwrap_or_else(|_| json!({"ok": false, "error": "internal error"}));
            println!("{reply}");
            0
        }
        _ => fail("usage: ssh-app-host hello | pair | doctor | usage [CONFIG_DIR...] | hook AGENT EVENT | rpc [--lease SECS] | agent | ctl SUB | statusline [PREV_HEX]"),
    };
    std::process::exit(code);
}

/// `ls (--path P | --home | --pane SESSION) [--hidden] [--max N]`.
fn ls_reply(args: &[String]) -> serde_json::Value {
    let (mut target, mut hidden, mut max) = (None, false, files::MAX_ENTRIES);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--path" => target = it.next().map(|p| files::Target::Path(p.clone())),
            "--pane" => target = it.next().map(|p| files::Target::Pane(p.clone())),
            "--home" => target = Some(files::Target::Home),
            "--hidden" => hidden = true,
            "--max" => max = it.next().and_then(|n| n.parse().ok()).unwrap_or(max),
            _ => {
                return json!({"ok": false, "error": "usage: ls (--path P | --home | --pane S) [--hidden] [--max N]"})
            }
        }
    }
    match target {
        // A link into a stalled mount can block `canonicalize`; answer without `ok` after 8 s so the
        // app falls back to the plain listing instead of waiting for its own watchdog.
        Some(t) => {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(files::list(&t, hidden, max));
            });
            rx.recv_timeout(std::time::Duration::from_secs(8))
                .unwrap_or_else(|_| json!({"error": "timeout"}))
        }
        None => json!({"ok": false, "error": "ls needs --path, --home or --pane"}),
    }
}

/// `read --path P [--max N]`; like `ls`, a file on a stalled mount is cut off after 8 s with a reply
/// that has no `ok`, so the app falls back to the plain read.
fn read_reply(args: &[String]) -> serde_json::Value {
    let (mut path, mut max) = (None, 0usize);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--path" => path = it.next().cloned(),
            "--max" => max = it.next().and_then(|n| n.parse().ok()).unwrap_or(0),
            _ => return json!({"ok": false, "error": "usage: read --path P [--max N]"}),
        }
    }
    let Some(path) = path else {
        return json!({"ok": false, "error": "read needs --path"});
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(files::read(&path, max));
    });
    rx.recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|_| json!({"error": "timeout"}))
}

/// `kill --pid N [--force] [--started EPOCH]`.
fn kill_reply(args: &[String]) -> serde_json::Value {
    let (mut pid, mut force, mut started) = (None, false, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pid" => {
                pid = it
                    .next()
                    .and_then(|n| n.parse::<i32>().ok())
                    .and_then(|n| u32::try_from(n).ok())
            }
            "--force" => force = true,
            "--started" => started = it.next().and_then(|n| n.parse::<u64>().ok()),
            _ => {
                return json!({"ok": false, "error": "usage: kill --pid N [--force] [--started EPOCH]"})
            }
        }
    }
    match pid {
        Some(p) => procs::kill(p, force, started),
        None => json!({"ok": false, "error": "kill needs --pid"}),
    }
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
