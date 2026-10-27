//! What the push notification extension does, measured: one SSH exec for a sandbox's refused
//! hosts, then the process's peak memory. iOS gives a notification extension roughly 24 MB.
//! Run by `first_contact`-style loopback tests, or by hand:
//! `cargo run --release -p ssh-core --example nse_probe -- HOST PORT USER KEY_PEM_FILE ENGINE SANDBOX_ID`
use ssh_core::sandbox::SandboxEngine;
use ssh_core::{key_store, RemoteServerConfig};

/// Peak resident size of this process in bytes (macOS reports bytes, Linux kilobytes).
fn peak_rss() -> u64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `u` is a valid, writable rusage for the call.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    let n = u64::try_from(u.ru_maxrss).unwrap_or(0);
    if cfg!(target_os = "linux") {
        n * 1024
    } else {
        n
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 7 {
        eprintln!("usage: nse_probe HOST PORT USER KEY_PEM_FILE ENGINE SANDBOX_ID");
        std::process::exit(2);
    }
    let pem = std::fs::read_to_string(&a[4]).expect("key file");
    let before = peak_rss();
    let cfg = RemoteServerConfig {
        host: a[1].clone(),
        port: a[2].parse().expect("port"),
        username: a[3].clone(),
        key: key_store::test_put(&pem),
        pinned_host_key: None,
    };
    let engine = if a[5] == "podman" {
        SandboxEngine::Podman
    } else {
        SandboxEngine::Docker
    };
    let started = std::time::Instant::now();
    let hosts = ssh_core::sandbox_blocked_hosts_for(cfg, engine, a[6].clone())
        .await
        .expect("blocked hosts");
    println!("hosts={}", hosts.join(","));
    println!("ms={}", started.elapsed().as_millis());
    println!("peak_rss_before={before}");
    println!("peak_rss_after={}", peak_rss());
}
