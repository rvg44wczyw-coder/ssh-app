//! The host agent against a private `SSH_APP_HOME` (under /tmp: Unix socket paths are short). No
//! tmux, no network; every agent is killed when its test ends.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_ssh-app-host");

struct Home {
    dir: tempfile::TempDir,
    pids: std::cell::RefCell<Vec<i32>>,
}

impl Home {
    fn new() -> Home {
        let dir = tempfile::Builder::new()
            .prefix("hd")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        Home {
            dir,
            pids: Default::default(),
        }
    }
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.args(args)
            .env("SSH_APP_HOME", self.dir.path())
            // A bare `tmux` in the agent must never find the developer's real server.
            .env("TMUX_TMPDIR", self.dir.path())
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stderr(Stdio::null());
        c
    }
    fn rpc(&self, lease: &str) -> Rpc {
        let mut child = self
            .cmd(&["rpc", "--lease", lease])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        Rpc {
            stdin: child.stdin.take().unwrap(),
            stdout: BufReader::new(child.stdout.take().unwrap()),
            child,
        }
    }
    fn hook(&self, event: &str, input: &str, timeout: &str) -> Child {
        let mut child = self
            .cmd(&["hook", "claude", event])
            .env("SSH_APP_HOOK_TIMEOUT", timeout)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child
    }
    fn sock(&self) -> std::path::PathBuf {
        self.dir.path().join("run/hostd.sock")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        for pid in self.pids.borrow().iter() {
            // SAFETY: plain kill(2) of an agent this test started.
            unsafe { libc::kill(*pid, libc::SIGKILL) };
        }
    }
}

struct Rpc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Rpc {
    fn send(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").unwrap();
    }
    fn line(&mut self) -> Value {
        let mut s = String::new();
        self.stdout.read_line(&mut s).unwrap();
        serde_json::from_str(s.trim()).unwrap_or_else(|e| panic!("{e}: {s:?}"))
    }
    /// Sends a request and returns its reply (skipping the lease reply and stray events).
    fn call(&mut self, id: u32, m: &str) -> Value {
        self.send(json!({"id": id, "m": m}));
        loop {
            let v = self.line();
            if v["id"] == id {
                return v;
            }
        }
    }
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(10), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[test]
fn agent_starts_once_reports_state_and_streams_changes() {
    let h = Home::new();
    let mut a = h.rpc("60");
    let hello = a.call(1, "hello");
    let pid = hello["pid"].as_i64().unwrap() as i32;
    h.pids.borrow_mut().push(pid);
    assert_eq!(hello["name"], "ssh-app-host-agent");
    assert_eq!(
        std::fs::metadata(h.sock()).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // A second phone connection reuses the same agent.
    let mut b = h.rpc("60");
    assert_eq!(b.call(1, "hello")["pid"].as_i64().unwrap() as i32, pid);

    assert_eq!(a.call(2, "subscribe")["ok"], true);
    // The agent being a child of rpc must not matter: it survives its first rpc closing.
    drop(b);

    h.hook("Stop", r#"{"session_id":"s1","cwd":"/w"}"#, "5")
        .wait()
        .unwrap();
    let ev = a.line();
    assert_eq!(ev["event"], "state");
    assert_eq!(
        (ev["session"].as_str(), ev["to"].as_str()),
        (Some("s1"), Some("finished"))
    );

    let state = a.call(3, "state");
    assert_eq!(state["sessions"][0]["state"], "finished");

    // A pending approval wins over the notification.
    let pending = h.hook(
        "PermissionRequest",
        r#"{"tool_name":"Bash","tool_input":{"command":"ls"},"session_id":"s1","cwd":"/w"}"#,
        "30",
    );
    let ev = a.line();
    assert_eq!(
        (ev["from"].as_str(), ev["to"].as_str()),
        (Some("finished"), Some("approval"))
    );
    assert_eq!(a.call(4, "state")["pending"], 1);
    let mut pending = pending;
    let _ = pending.kill();
    let _ = pending.wait();
}

#[test]
fn closed_method_list_and_input_caps() {
    let h = Home::new();
    let mut a = h.rpc("60");
    let pid = a.call(1, "hello")["pid"].as_i64().unwrap() as i32;
    h.pids.borrow_mut().push(pid);
    assert_eq!(a.call(2, "exec")["ok"], false);
    assert_eq!(a.call(3, "ctl")["error"], "unknown method");
    a.send(json!("not an object"));
    assert_eq!(a.line()["ok"], false);
    // An oversized line is refused and the connection closed.
    let big = "x".repeat(5000);
    let mut stream = std::os::unix::net::UnixStream::connect(h.sock()).unwrap();
    writeln!(stream, "{big}").unwrap();
    let mut r = BufReader::new(stream);
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    assert!(line.contains("too long"), "{line}");
}

#[test]
fn the_agent_exits_when_its_lease_runs_out_and_a_stale_socket_is_recovered() {
    let h = Home::new();
    let mut a = h.rpc("2");
    let pid = a.call(1, "hello")["pid"].as_i64().unwrap() as i32;
    h.pids.borrow_mut().push(pid);
    // The rpc renews the lease with its own `--lease 2`; no further contact follows.
    drop(a);
    wait_for("agent exit", || !alive(pid));
    assert!(!h.sock().exists(), "socket removed on exit");

    // Killed hard: the stale socket file stays and the next rpc still gets a working agent.
    let mut a = h.rpc("60");
    let pid = a.call(1, "hello")["pid"].as_i64().unwrap() as i32;
    h.pids.borrow_mut().push(pid);
    // SAFETY: kill of the agent this test started.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    wait_for("agent killed", || !alive(pid));
    assert!(h.sock().exists());
    let mut b = h.rpc("60");
    let new_pid = b.call(1, "hello")["pid"].as_i64().unwrap() as i32;
    h.pids.borrow_mut().push(new_pid);
    assert_ne!(new_pid, pid);
    let _ = a.child.kill();
}

#[test]
fn one_subscription_per_connection() {
    let h = Home::new();
    let mut a = h.rpc("60");
    let pid = a.call(1, "hello")["pid"].as_i64().unwrap() as i32;
    h.pids.borrow_mut().push(pid);
    assert_eq!(a.call(2, "subscribe")["ok"], true);
    assert_eq!(a.call(3, "subscribe")["ok"], false);
}

#[test]
fn usage_through_the_agent_is_cached_and_equals_a_direct_scan() {
    let h = Home::new();
    let claude = h.dir.path().join("claude");
    std::fs::create_dir_all(&claude).unwrap();
    let run = |args: &[&str]| -> Value {
        let out = h
            .cmd(args)
            .env("HOME", h.dir.path())
            .env("TMUX_TMPDIR", h.dir.path())
            .stdin(Stdio::null())
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let dir = claude.to_str().unwrap();
    let direct = run(&["usage", dir]);
    let first = run(&["usage", "--agent", dir]);
    // The agent is up now; remember it so it is killed with the test.
    let mut a = h.rpc("60");
    let pid = a.call(1, "hello")["pid"].as_i64().unwrap() as i32;
    h.pids.borrow_mut().push(pid);
    let second = run(&["usage", "--agent", dir]);
    assert_eq!(first, direct);
    assert_eq!(second["sessions"], direct["sessions"]);
    // The cache answers on the wire too.
    let mut b = h.rpc("60");
    b.send(json!({"id": 9, "m": "usage", "dirs": [dir]}));
    let reply = loop {
        let v = b.line();
        if v["id"] == 9 {
            break v;
        }
    };
    assert_eq!(reply["cached"], true);
    // Bad input is refused, not scanned.
    b.send(json!({"id": 10, "m": "usage", "dirs": ["bad\u{7}dir"]}));
    loop {
        let v = b.line();
        if v["id"] == 10 {
            assert_eq!(v["ok"], false);
            break;
        }
    }
}

#[test]
fn state_includes_agents_found_in_tmux_without_hooks() {
    // A fake `claude` process in a private-socket tmux session (never the real server).
    let h = Home::new();
    let sock = h.dir.path().join("t.sock");
    let tmux = |args: &[&str]| {
        Command::new("tmux")
            .arg("-S")
            .arg(&sock)
            .args(args)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .status()
    };
    let Ok(st) = tmux(&[
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-s",
        "work",
        "exec -a claude sleep 60",
    ]) else {
        eprintln!("skipping: tmux missing");
        return;
    };
    if !st.success() {
        return;
    }
    let mut a = h
        .cmd(&["rpc", "--lease", "60"])
        .env("TMUX_TMPDIR", h.dir.path())
        .env("TMUX", format!("{},0,0", sock.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = a.stdin.take().unwrap();
    let mut out = BufReader::new(a.stdout.take().unwrap());
    writeln!(stdin, r#"{{"id":1,"m":"hello"}}"#).unwrap();
    writeln!(stdin, r#"{{"id":2,"m":"state"}}"#).unwrap();
    let mut state = Value::Null;
    for _ in 0..4 {
        let mut l = String::new();
        out.read_line(&mut l).unwrap();
        let v: Value = serde_json::from_str(l.trim()).unwrap();
        if v["id"] == 1 {
            h.pids.borrow_mut().push(v["pid"].as_i64().unwrap() as i32);
        }
        if v["id"] == 2 {
            state = v;
            break;
        }
    }
    let _ = tmux(&["kill-server"]);
    let _ = a.kill();
    let _ = a.wait();
    let found: Vec<&Value> = state["sessions"]
        .as_array()
        .map(|a| a.iter().filter(|s| s["session"] == "work").collect())
        .unwrap_or_default();
    assert_eq!(found.len(), 1, "{state}");
    assert_eq!(found[0]["state"], "running");
}

#[test]
fn an_agent_of_another_build_is_replaced_by_the_next_rpc() {
    let h = Home::new();
    // A second copy of the helper with one extra byte: another build with the same behaviour.
    let other = h.dir.path().join("ssh-app-host-old");
    let mut bytes = std::fs::read(BIN).unwrap();
    bytes.push(0);
    std::fs::write(&other, bytes).unwrap();
    std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut old = Command::new(&other)
        .args(["rpc", "--lease", "60"])
        .env("SSH_APP_HOME", h.dir.path())
        .env("TMUX_TMPDIR", h.dir.path())
        .env_remove("TMUX")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = old.stdin.take().unwrap();
    let mut out = BufReader::new(old.stdout.take().unwrap());
    writeln!(stdin, r#"{{"id":1,"m":"hello"}}"#).unwrap();
    let old_pid = loop {
        let mut l = String::new();
        out.read_line(&mut l).unwrap();
        let v: Value = serde_json::from_str(l.trim()).unwrap();
        if v["id"] == 1 {
            break v["pid"].as_i64().unwrap() as i32;
        }
    };
    h.pids.borrow_mut().push(old_pid);
    drop(stdin);
    let _ = old.wait();
    let mut a = h.rpc("60");
    let new_pid = a.call(1, "hello")["pid"].as_i64().unwrap() as i32;
    h.pids.borrow_mut().push(new_pid);
    assert_ne!(new_pid, old_pid, "the outdated agent was replaced");
    wait_for("old agent gone", || !alive(old_pid));
}

#[test]
fn the_agent_exits_when_its_state_folder_is_removed() {
    let h = Home::new();
    let mut a = h.rpc("600");
    let pid = a.call(1, "hello")["pid"].as_i64().unwrap() as i32;
    h.pids.borrow_mut().push(pid);
    std::fs::remove_dir_all(h.dir.path().join("run")).unwrap();
    std::fs::remove_dir_all(h.dir.path()).ok();
    wait_for("agent exit", || !alive(pid));
}

#[test]
fn a_subscriber_gets_events_for_agents_that_appear_in_tmux() {
    let h = Home::new();
    let sock = h.dir.path().join("t.sock");
    let tmux = |args: &[&str]| {
        Command::new("tmux")
            .arg("-S")
            .arg(&sock)
            .args(args)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .status()
    };
    let mut a = h
        .cmd(&["rpc", "--lease", "60"])
        .env("TMUX_TMPDIR", h.dir.path())
        .env("TMUX", format!("{},0,0", sock.display()))
        .env("SSH_APP_LIVE_RESCAN_MS", "300")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = a.stdin.take().unwrap();
    let mut out = BufReader::new(a.stdout.take().unwrap());
    writeln!(stdin, r#"{{"id":1,"m":"hello"}}"#).unwrap();
    writeln!(stdin, r#"{{"id":2,"m":"subscribe"}}"#).unwrap();
    let mut subscribed = false;
    while !subscribed {
        let mut l = String::new();
        out.read_line(&mut l).unwrap();
        let v: Value = serde_json::from_str(l.trim()).unwrap();
        if v["id"] == 1 {
            h.pids.borrow_mut().push(v["pid"].as_i64().unwrap() as i32);
        }
        subscribed = v["id"] == 2 && v["ok"] == true;
    }
    let started = tmux(&[
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-s",
        "live",
        "exec -a claude sleep 60",
    ]);
    let mut seen = None;
    if matches!(started, Ok(s) if s.success()) {
        for _ in 0..60 {
            let mut l = String::new();
            if out.read_line(&mut l).unwrap_or(0) == 0 {
                break;
            }
            let v: Value = serde_json::from_str(l.trim()).unwrap();
            if v["event"] == "state" && v["session"] == "live" {
                seen = Some(v);
                break;
            }
        }
        let _ = tmux(&["kill-server"]);
    } else {
        eprintln!("skipping: tmux missing");
        let _ = a.kill();
        let _ = a.wait();
        return;
    }
    let _ = a.kill();
    let _ = a.wait();
    let ev = seen.expect("an event for the tmux session");
    assert_eq!(
        (ev["from"].as_str(), ev["to"].as_str()),
        (Some("none"), Some("running"))
    );
}
