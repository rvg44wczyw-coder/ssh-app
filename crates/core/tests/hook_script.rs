//! Runs the real `ssh-app-hook.py` against a fake approval broker on a Unix socket and checks the
//! exact JSON each agent receives. Skipped when `python3` is unavailable.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/agent-hooks/ssh-app-hook.py")
}

fn have_python() -> bool {
    Command::new("python3")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Runs the hook; returns (stdout, exit code).
fn run_hook(agent: &str, event: &str, stdin: &str, socket: &std::path::Path) -> (String, i32) {
    let mut child = Command::new("python3")
        .arg("-I")
        .arg(script())
        .args([agent, event])
        .env("SSH_APP_SOCKET", socket)
        .env("SSH_APP_HOOK_TIMEOUT", "2")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    (out, child.wait().unwrap().code().unwrap_or(-1))
}

/// Broker answering every line with `reply` (None = close without answering); returns the
/// request lines it saw via the join handle.
fn broker(
    dir: &std::path::Path,
    mode: u32,
    reply: Option<&'static str>,
    n: usize,
) -> thread::JoinHandle<Vec<String>> {
    let path = dir.join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    thread::spawn(move || {
        let mut seen = vec![];
        for _ in 0..n {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            seen.push(line);
            if let Some(r) = reply {
                let _ = stream.write_all(format!("{r}\n").as_bytes());
            }
        }
        seen
    })
}

fn tmp() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    d
}

const CLAUDE_IN: &str = r#"{"session_id":"s1","cwd":"/w","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"git push"}}"#;
const AGY_IN: &str = r#"{"conversationId":"c1","workspacePaths":["/w"],"toolCall":{"name":"run_command","args":{"CommandLine":"rm -rf build"}}}"#;

#[test]
fn claude_allow_and_deny() {
    if !have_python() {
        return;
    }
    for (reply, behavior) in [
        (r#"{"approved":true}"#, "allow"),
        (r#"{"approved":false}"#, "deny"),
    ] {
        let d = tmp();
        let b = broker(d.path(), 0o600, Some(reply), 1);
        let (out, code) = run_hook(
            "claude",
            "PermissionRequest",
            CLAUDE_IN,
            &d.path().join("control.sock"),
        );
        assert_eq!(code, 0);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], behavior);
        let req: serde_json::Value = serde_json::from_str(&b.join().unwrap()[0]).unwrap();
        assert_eq!(req["method"], "request_approval");
        assert_eq!(req["agent"], "claude");
        assert_eq!(req["command"], "git push");
        assert_eq!(req["cwd"], "/w");
    }
}

#[test]
fn antigravity_decisions() {
    if !have_python() {
        return;
    }
    for (reply, decision) in [
        (r#"{"approved":true}"#, "auto_approve"),
        (r#"{"approved":false}"#, "deny"),
    ] {
        let d = tmp();
        let b = broker(d.path(), 0o600, Some(reply), 1);
        let (out, code) = run_hook(
            "antigravity",
            "PreToolUse",
            AGY_IN,
            &d.path().join("control.sock"),
        );
        assert_eq!(code, 0);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&out).unwrap()["decision"],
            decision
        );
        let req: serde_json::Value = serde_json::from_str(&b.join().unwrap()[0]).unwrap();
        assert_eq!(req["command"], "rm -rf build");
        assert_eq!(req["tool"], "run_command");
    }
}

#[test]
fn no_decision_defers_to_native_policy() {
    if !have_python() {
        return;
    }
    let d = tmp();
    let missing = d.path().join("control.sock");
    // Broker absent.
    assert_eq!(
        run_hook("claude", "PermissionRequest", CLAUDE_IN, &missing),
        (String::new(), 0)
    );
    let (out, _) = run_hook("antigravity", "PreToolUse", AGY_IN, &missing);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&out).unwrap()["decision"],
        "allow"
    );

    // Garbage / non-boolean / closed-without-reply all mean "no opinion", never approval.
    for reply in [
        Some("not json"),
        Some(r#"{"approved":"yes"}"#),
        Some(r#"{}"#),
        None,
    ] {
        let d = tmp();
        let b = broker(d.path(), 0o600, reply, 1);
        let (out, code) = run_hook(
            "claude",
            "PermissionRequest",
            CLAUDE_IN,
            &d.path().join("control.sock"),
        );
        assert_eq!((out.as_str(), code), ("", 0), "{reply:?}");
        b.join().unwrap();
    }
}

#[test]
fn untrusted_socket_is_ignored() {
    if !have_python() {
        return;
    }
    // World-accessible socket: the hook must not even connect (broker would hang on accept).
    let d = tmp();
    let _b = broker(d.path(), 0o666, Some(r#"{"approved":true}"#), 1);
    let (out, code) = run_hook(
        "claude",
        "PermissionRequest",
        CLAUDE_IN,
        &d.path().join("control.sock"),
    );
    assert_eq!((out.as_str(), code), ("", 0));

    // Group/other-accessible directory.
    let d = tmp();
    let _b = broker(d.path(), 0o600, Some(r#"{"approved":true}"#), 1);
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let (out, _) = run_hook(
        "claude",
        "PermissionRequest",
        CLAUDE_IN,
        &d.path().join("control.sock"),
    );
    assert_eq!(out, "");
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();

    // A symlink pointing at a good socket is rejected (lstat).
    let d = tmp();
    let _b = broker(d.path(), 0o600, Some(r#"{"approved":true}"#), 1);
    let link = d.path().join("link.sock");
    std::os::unix::fs::symlink(d.path().join("control.sock"), &link).unwrap();
    let (out, _) = run_hook("claude", "PermissionRequest", CLAUDE_IN, &link);
    assert_eq!(out, "");
}

#[test]
fn passive_events_notify_without_waiting() {
    if !have_python() {
        return;
    }
    let d = tmp();
    let b = broker(d.path(), 0o600, None, 1);
    let (out, code) = run_hook("claude", "Stop", CLAUDE_IN, &d.path().join("control.sock"));
    assert_eq!((out.as_str(), code), ("", 0));
    let req: serde_json::Value = serde_json::from_str(&b.join().unwrap()[0]).unwrap();
    assert_eq!(req["method"], "notify");
    assert_eq!(req["event"], "Stop");

    let d = tmp();
    let _b = broker(d.path(), 0o600, None, 1);
    let (out, _) = run_hook(
        "antigravity",
        "Stop",
        AGY_IN,
        &d.path().join("control.sock"),
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&out).unwrap()["decision"],
        "stop"
    );
}

#[test]
fn malformed_input_and_unknown_agent_are_harmless() {
    if !have_python() {
        return;
    }
    let d = tmp();
    let missing = d.path().join("control.sock");
    assert_eq!(
        run_hook("claude", "PermissionRequest", "not json", &missing),
        (String::new(), 0)
    );
    assert_eq!(
        run_hook("unknown", "PreToolUse", "{}", &missing),
        (String::new(), 0)
    );
    assert_eq!(run_hook("", "", "", &missing), (String::new(), 0));
}

#[test]
fn wrong_typed_json_never_crashes_the_hook() {
    if !have_python() {
        return;
    }
    let d = tmp();
    let missing = d.path().join("control.sock");
    for input in [
        "[1,2,3]",
        "\"str\"",
        "null",
        r#"{"tool_input":"x","cwd":5}"#,
        r#"{"toolCall":"x"}"#,
        r#"{"toolCall":{"name":1,"args":"s"},"workspacePaths":"abc"}"#,
        r#"{"toolCall":null,"workspacePaths":[]}"#,
    ] {
        for (agent, event) in [
            ("claude", "PermissionRequest"),
            ("antigravity", "PreToolUse"),
        ] {
            let (out, code) = run_hook(agent, event, input, &missing);
            assert_eq!(code, 0, "{agent} {input}");
            // Never an approval without a broker.
            assert!(
                !out.contains("auto_approve") && !out.contains("\"allow\"")
                    || agent == "antigravity",
                "{out}"
            );
        }
    }
    let deep = "[".repeat(200_000);
    assert_eq!(
        run_hook("claude", "PermissionRequest", &deep, &missing).1,
        0
    );
}

#[test]
fn slow_drip_broker_cannot_exceed_the_deadline() {
    if !have_python() {
        return;
    }
    let d = tmp();
    let path = d.path().join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let h = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(6) {
            if s.write_all(b" ").is_err() {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(300));
        }
    });
    let t = std::time::Instant::now();
    let (out, code) = run_hook("claude", "PermissionRequest", CLAUDE_IN, &path);
    assert_eq!((out.as_str(), code), ("", 0));
    assert!(t.elapsed().as_secs_f32() < 4.0, "took {:?}", t.elapsed());
    let _ = h.join();
}
