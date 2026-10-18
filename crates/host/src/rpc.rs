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

/// Stops a running agent of another build (the helper was updated underneath it).
fn retire_outdated(p: &Paths) {
    use std::io::{BufRead, BufReader};
    let sock = socket_path(p);
    let Ok(mut s) = UnixStream::connect(&sock) else {
        return;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    if s.write_all(b"{\"id\":1,\"m\":\"hello\"}\n").is_err() {
        return;
    }
    let mut line = String::new();
    if BufReader::new(&s).read_line(&mut line).is_err() {
        return;
    }
    let build = serde_json::from_str::<serde_json::Value>(line.trim())
        .ok()
        .and_then(|v| v["build"].as_str().map(str::to_string));
    if build.as_deref() == Some(crate::agent::self_digest().as_str()) {
        return;
    }
    let _ = s.write_all(b"{\"id\":2,\"m\":\"quit\"}\n");
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if UnixStream::connect(&sock).is_err() {
            return;
        }
    }
}

fn connect(p: &Paths, lease: i64) -> Option<UnixStream> {
    retire_outdated(p);
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

/// One request/reply through the agent (started if needed, lease renewed); `None` on any failure.
pub fn call(p: &Paths, lease: i64, request: &serde_json::Value) -> Option<serde_json::Value> {
    use std::io::{BufRead, BufReader};
    let mut stream = connect(p, lease)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(40)))
        .ok()?;
    let lease_req = format!("{{\"id\":0,\"m\":\"lease\",\"secs\":{lease}}}\n");
    let mut req = request.clone();
    req["id"] = serde_json::json!(1);
    stream.write_all(lease_req.as_bytes()).ok()?;
    stream.write_all(format!("{req}\n").as_bytes()).ok()?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        if v["id"] == 1 {
            return Some(v);
        }
    }
}

pub fn parse_lease(args: &[String]) -> i64 {
    args.windows(2)
        .find(|w| w[0] == "--lease")
        .and_then(|w| w[1].parse::<i64>().ok())
        .unwrap_or(DEFAULT_LEASE)
}
