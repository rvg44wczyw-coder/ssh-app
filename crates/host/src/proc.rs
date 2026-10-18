//! Child processes with a fixed argv (never through a shell, except the explicit `sh -c` of the
//! user's own previous status line) and a hard timeout.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_OUT: u64 = 1 << 20;

/// Runs `cmd`, feeding it `input`, and returns its exit code and stdout, or `None` when it could
/// not start, wrote too much, or did not finish within `timeout` (it is killed then).
pub fn run_with_input(
    mut cmd: Command,
    input: &[u8],
    timeout: Duration,
) -> Option<(Option<i32>, Vec<u8>)> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let data = input.to_vec();
    // A separate thread so a child that never reads cannot block us before the timeout starts.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&data);
    });
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut stdout).take(MAX_OUT).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let status = status?;
    let _ = writer.join();
    // A grandchild may keep the pipe open after the child exited; do not wait for it for long.
    let out = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    Some((status.code(), out))
}
