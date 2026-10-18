//! `rpc`: the stdio bridge the app runs over SSH. Makes sure the agent is up (starting it
//! detached when needed), renews its lease and copies bytes both ways until either side closes.

use crate::agent::{socket_path, DEFAULT_LEASE};
use crate::queue::Paths;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

fn start_agent(lease: i64) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = Command::new(exe);
    cmd.args(["agent", "--lease", &lease.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe; it detaches the agent from the SSH session so it
    // survives the connection closing (its lifetime is the lease, not the channel).
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = cmd.spawn();
}

fn connect(p: &Paths, lease: i64) -> Option<UnixStream> {
    let sock = socket_path(p);
    if let Ok(s) = UnixStream::connect(&sock) {
        return Some(s);
    }
    start_agent(lease);
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(s) = UnixStream::connect(&sock) {
            return Some(s);
        }
    }
    None
}

pub fn run(p: &Paths, lease: i64) -> i32 {
    let Some(mut stream) = connect(p, lease) else {
        eprintln!("the host agent did not start");
        return 1;
    };
    let lease_req = format!("{{\"id\":0,\"m\":\"lease\",\"secs\":{lease}}}\n");
    if stream.write_all(lease_req.as_bytes()).is_err() {
        return 1;
    }
    let Ok(mut upstream) = stream.try_clone() else {
        return 1;
    };
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 || upstream.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        let _ = upstream.shutdown(std::net::Shutdown::Write);
    });
    let mut stdout = std::io::stdout().lock();
    let mut buf = [0u8; 4096];
    while let Ok(n) = stream.read(&mut buf) {
        if n == 0
            || stdout
                .write_all(&buf[..n])
                .and_then(|()| stdout.flush())
                .is_err()
        {
            break;
        }
    }
    0
}

pub fn parse_lease(args: &[String]) -> i64 {
    args.windows(2)
        .find(|w| w[0] == "--lease")
        .and_then(|w| w[1].parse::<i64>().ok())
        .unwrap_or(DEFAULT_LEASE)
}
