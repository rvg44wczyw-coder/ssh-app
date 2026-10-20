//! Command-line behaviour of `ssh-app-host hook` that the Python reference did not have.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

fn home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for d in ["", "run", "run/pending", "run/decisions"] {
        let p = dir.path().join(d);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

fn hook(home: &std::path::Path, agent: &str, event: &str, stdin: &[u8]) -> (String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ssh-app-host"))
        .args(["hook", agent, event])
        .env("SSH_APP_HOME", home)
        .env("SSH_APP_HOOK_TIMEOUT", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    let data = stdin.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = pipe.write_all(&data);
    });
    let out = child.wait_with_output().unwrap();
    let _ = writer.join();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code(),
    )
}

#[test]
fn a_payload_beyond_the_read_cap_is_refused_not_sent_to_the_phone_blank() {
    let h = home();
    let big = vec![b'x'; (33 << 20) + 1];
    for (agent, event) in [("claude", "PreToolUse"), ("opencode", "PermissionRequest")] {
        let (out, code) = hook(h.path(), agent, event, &big);
        assert_eq!(code, Some(0));
        assert!(out.contains("deny"), "{agent}: {out}");
    }
    assert_eq!(
        std::fs::read_dir(h.path().join("run/pending"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn unknown_agents_and_garbage_are_harmless() {
    let h = home();
    assert_eq!(
        hook(h.path(), "nobody", "Stop", b"{}"),
        (String::new(), Some(0))
    );
    let (out, code) = hook(h.path(), "claude", "Stop", b"\xff\xfe not json");
    assert_eq!((out.as_str(), code), ("", Some(0)));
}

#[test]
fn any_first_event_registers_the_session_for_every_agent() {
    // Antigravity has no SessionStart hook: its tmux name must be noted by whatever event comes first.
    let cases = [
        ("claude", r#"{"session_id":"s-claude","cwd":"/w"}"#),
        ("codex", r#"{"session_id":"s-codex","cwd":"/w"}"#),
        ("qwen", r#"{"session_id":"s-qwen","cwd":"/w"}"#),
        ("crush", r#"{"session_id":"s-crush","cwd":"/w"}"#),
        (
            "antigravity",
            r#"{"conversationId":"s-agy","workspacePaths":["/w"]}"#,
        ),
        ("opencode", r#"{"session":"s-oc","cwd":"/w"}"#),
        ("grok", r#"{"sessionId":"s-grok","cwd":"/w"}"#),
    ];
    for (agent, payload) in cases {
        let h = home();
        hook(h.path(), agent, "Stop", payload.as_bytes());
        let n = std::fs::read_dir(h.path().join("run/sessions")).map_or(0, |d| d.flatten().count());
        assert_eq!(n, 1, "{agent} was not registered by its first event");
    }
}
