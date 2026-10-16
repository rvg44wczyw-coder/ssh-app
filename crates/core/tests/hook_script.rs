//! Runs the real `ssh-app-hook.py` against a temporary `SSH_APP_HOME` and plays the phone with the
//! real signing code: queue protocol, SSHSIG verification by the host, and every way a forged,
//! stale or misdirected decision must be ignored. Skipped when `python3` or `ssh-keygen` is
//! unavailable.

use serde_json::{json, Value};
use ssh_core::{
    approval_request_digest, create_canonical_signing_bytes, generate_keypair,
    sign_approval_sshsig, KeypairResult,
};
use std::io::{Read, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/agent-hooks/ssh-app-hook.py")
}

fn available() -> bool {
    let ok = |c: &str, a: &str| {
        Command::new(c)
            .arg(a)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    };
    ok("python3", "--version") && ok("ssh-keygen", "-?")
}

fn chmod(p: &Path, mode: u32) {
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
}

struct Host {
    dir: tempfile::TempDir,
    key: KeypairResult,
}

impl Host {
    fn new() -> Host {
        let dir = tempfile::tempdir().unwrap();
        let key = generate_keypair().unwrap();
        let h = Host { dir, key };
        chmod(h.home(), 0o700);
        for d in [h.run(), h.pending(), h.decisions()] {
            std::fs::create_dir_all(&d).unwrap();
            chmod(&d, 0o700);
        }
        h.trust(&h.key.public_key_openssh.clone());
        h
    }
    fn home(&self) -> &Path {
        self.dir.path()
    }
    fn run(&self) -> PathBuf {
        self.home().join("run")
    }
    fn pending(&self) -> PathBuf {
        self.run().join("pending")
    }
    fn decisions(&self) -> PathBuf {
        self.run().join("decisions")
    }
    fn trust(&self, public: &str) {
        let mut it = public.split_whitespace();
        let line = format!(
            "ssh-app namespaces=\"ssh-app-approval\" {} {}\n",
            it.next().unwrap(),
            it.next().unwrap()
        );
        let p = self.home().join("allowed_signers");
        std::fs::write(&p, line).unwrap();
        chmod(&p, 0o600);
    }
    fn cmd(&self, args: &[&str], timeout: u32) -> Command {
        let mut c = Command::new("python3");
        c.arg("-I")
            .arg(script())
            .args(args)
            .env("SSH_APP_HOME", self.home())
            .env("SSH_APP_HOOK_TIMEOUT", timeout.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        c
    }
    fn spawn_hook(&self, agent: &str, event: &str, stdin: &str, timeout: u32) -> Hook {
        let mut child = self.cmd(&[agent, event], timeout).spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        Hook(child)
    }
    fn ctl(&self, sub: &str, stdin: &str) -> Value {
        let mut child = self.cmd(&["ctl", sub], 1).spawn().unwrap();
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
        child.wait().unwrap();
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("ctl {sub}: {e}: {out:?}"))
    }
    /// Waits until the hook has queued an item of `kind`; returns (item, host_time).
    fn wait_item(&self, kind: &str) -> (Value, u64) {
        let t = Instant::now();
        loop {
            let l = self.ctl("list", "");
            if let Some(i) = l["items"]
                .as_array()
                .and_then(|a| a.iter().find(|i| i["kind"] == kind))
            {
                return (i.clone(), l["host_time"].as_u64().unwrap());
            }
            assert!(
                t.elapsed() < Duration::from_secs(10),
                "no {kind} item appeared"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    /// A decision signed by `key` over the canonical message built from the given values; the
    /// `wire_*` values are what is sent, so tests can make the two differ.
    #[allow(clippy::too_many_arguments)]
    fn decision(
        &self,
        key: &KeypairResult,
        sign_id: &str,
        digest: &str,
        nonce: &str,
        sign_ts: u64,
        sign_approved: bool,
        wire_id: &str,
        wire_ts: u64,
        wire_approved: bool,
    ) -> String {
        let msg = create_canonical_signing_bytes(
            sign_id.into(),
            digest.into(),
            nonce.into(),
            sign_ts,
            sign_approved,
        );
        let sig = sign_approval_sshsig(key.private_key_openssh.clone(), msg).unwrap();
        json!({"v":1,"id":wire_id,"approved":wire_approved,"ts":wire_ts,"signature":sig})
            .to_string()
    }
    /// A correct decision from the phone's key.
    fn honest(&self, item: &Value, host_time: u64, approved: bool) -> String {
        let (id, d, n) = (
            item["id"].as_str().unwrap(),
            item["command_sha256"].as_str().unwrap(),
            item["nonce"].as_str().unwrap(),
        );
        self.decision(
            &self.key, id, d, n, host_time, approved, id, host_time, approved,
        )
    }
}

struct Hook(Child);

impl Hook {
    fn finish(mut self) -> (String, i32) {
        let mut out = String::new();
        self.0
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut out)
            .unwrap();
        (out, self.0.wait().unwrap().code().unwrap_or(-1))
    }
}

const CLAUDE_IN: &str = r#"{"tool_name":"Bash","tool_input":{"command":"git push origin main"},"cwd":"/work/proj","session_id":"s1"}"#;
const AGY_IN: &str = r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"rm -rf build"}},"workspacePaths":["/work/proj"],"conversationId":"c1"}"#;

fn decide_ok(h: &Host, body: &str) -> Value {
    h.ctl("decide", body)
}

macro_rules! need_tools {
    () => {
        if !available() {
            eprintln!("skipping: python3/ssh-keygen missing");
            return;
        }
    };
}

#[test]
fn claude_allow_and_deny_after_a_verified_signature() {
    need_tools!();
    for (approved, expect) in [
        (true, "\"behavior\": \"allow\""),
        (false, "\"behavior\": \"deny\""),
    ] {
        let h = Host::new();
        let hook = h.spawn_hook("claude", "PermissionRequest", CLAUDE_IN, 20);
        let (item, now) = h.wait_item("approval");
        assert_eq!(item["agent"], "claude");
        assert_eq!(item["tool"], "Bash");
        assert_eq!(item["command"], "git push origin main");
        assert_eq!(item["cwd"], "/work/proj");
        assert_eq!(
            item["command_sha256"].as_str().unwrap(),
            approval_request_digest("claude", "Bash", "/work/proj", "git push origin main")
        );
        let r = decide_ok(&h, &h.honest(&item, now, approved));
        assert_eq!(r["ok"], true, "{r}");
        let (out, code) = hook.finish();
        assert_eq!(code, 0);
        assert!(
            out.contains(&expect.replace(": ", ":")) || out.contains(expect),
            "{out}"
        );
        assert!(out.contains("PermissionRequest"), "{out}");
        // The hook cleans up after itself.
        assert!(std::fs::read_dir(h.pending()).unwrap().next().is_none());
        assert!(std::fs::read_dir(h.decisions()).unwrap().next().is_none());
    }
}

#[test]
fn antigravity_approve_and_deny() {
    need_tools!();
    for (approved, expect) in [(true, "allow"), (false, "deny")] {
        let h = Host::new();
        let hook = h.spawn_hook("antigravity", "PreToolUse", AGY_IN, 20);
        let (item, now) = h.wait_item("approval");
        assert_eq!(item["command"], "rm -rf build");
        assert_eq!(item["cwd"], "/work/proj");
        decide_ok(&h, &h.honest(&item, now, approved));
        let (out, _) = hook.finish();
        assert!(out.contains(expect), "{out}");
    }
}

#[test]
fn no_decision_defers_to_the_native_policy_and_cleans_up() {
    need_tools!();
    let h = Host::new();
    let (out, code) = h
        .spawn_hook("claude", "PermissionRequest", CLAUDE_IN, 1)
        .finish();
    assert_eq!((out.as_str(), code), ("", 0));
    let (out, _) = h
        .spawn_hook("antigravity", "PreToolUse", AGY_IN, 1)
        .finish();
    assert!(
        out.contains("\"ask\"") && !out.contains("allow") && !out.contains("auto_approve"),
        "{out}"
    );
    assert!(std::fs::read_dir(h.pending()).unwrap().next().is_none());
}

fn assert_ignored(h: &Host, mutate: impl Fn(&Host, &Value, u64) -> String) {
    let hook = h.spawn_hook("claude", "PermissionRequest", CLAUDE_IN, 3);
    let (item, now) = h.wait_item("approval");
    let body = mutate(h, &item, now);
    let r = h.ctl("decide", &body);
    // `ctl decide` only checks the shape; the hook is the verifier.
    let (out, _) = hook.finish();
    assert_eq!(out, "", "a bad decision was honoured ({r}): {out}");
}

#[test]
fn forged_stale_and_misdirected_decisions_are_ignored() {
    need_tools!();
    let intruder = generate_keypair().unwrap();
    let get = |v: &Value, k: &str| v[k].as_str().unwrap().to_string();

    // Signed by a key the host does not trust.
    assert_ignored(&Host::new(), |h, i, now| {
        h.decision(
            &intruder,
            &get(i, "id"),
            &get(i, "command_sha256"),
            &get(i, "nonce"),
            now,
            true,
            &get(i, "id"),
            now,
            true,
        )
    });
    // Signed `deny`, flipped to `approve` on the wire.
    assert_ignored(&Host::new(), |h, i, now| {
        h.decision(
            &h.key,
            &get(i, "id"),
            &get(i, "command_sha256"),
            &get(i, "nonce"),
            now,
            false,
            &get(i, "id"),
            now,
            true,
        )
    });
    // Signature over another request's hash (e.g. replayed from a different command).
    assert_ignored(&Host::new(), |h, i, now| {
        h.decision(
            &h.key,
            &get(i, "id"),
            &"0".repeat(64),
            &get(i, "nonce"),
            now,
            true,
            &get(i, "id"),
            now,
            true,
        )
    });
    // Wrong nonce.
    assert_ignored(&Host::new(), |h, i, now| {
        h.decision(
            &h.key,
            &get(i, "id"),
            &get(i, "command_sha256"),
            &"1".repeat(32),
            now,
            true,
            &get(i, "id"),
            now,
            true,
        )
    });
    // Timestamp before the request, and in the host's future (which also covers > 120 s after it).
    for before in [true, false] {
        assert_ignored(&Host::new(), move |h, i, now| {
            let t = if before {
                i["ts"].as_u64().unwrap() - 1
            } else {
                now + 1000
            };
            h.decision(
                &h.key,
                &get(i, "id"),
                &get(i, "command_sha256"),
                &get(i, "nonce"),
                t,
                true,
                &get(i, "id"),
                t,
                true,
            )
        });
    }
    // The signed timestamp differs from the one sent.
    assert_ignored(&Host::new(), |h, i, now| {
        h.decision(
            &h.key,
            &get(i, "id"),
            &get(i, "command_sha256"),
            &get(i, "nonce"),
            now,
            true,
            &get(i, "id"),
            now + 1,
            true,
        )
    });
    // Garbage signature.
    assert_ignored(&Host::new(), |_, i, now| {
        json!({"v":1,"id":i["id"],"approved":true,"ts":now,"signature":"-----BEGIN SSH SIGNATURE-----\nAAAA\n-----END SSH SIGNATURE-----\n"}).to_string()
    });
}

#[test]
fn junk_decisions_cannot_block_the_real_one() {
    need_tools!();
    let h = Host::new();
    let intruder = generate_keypair().unwrap();
    let hook = h.spawn_hook("claude", "PermissionRequest", CLAUDE_IN, 20);
    let (item, now) = h.wait_item("approval");
    let id = item["id"].as_str().unwrap();
    let junk = h.decision(
        &intruder,
        id,
        item["command_sha256"].as_str().unwrap(),
        item["nonce"].as_str().unwrap(),
        now,
        false,
        id,
        now,
        false,
    );
    assert_eq!(decide_ok(&h, &junk)["ok"], true);
    assert_eq!(decide_ok(&h, &h.honest(&item, now, true))["ok"], true);
    let (out, _) = hook.finish();
    assert!(out.contains("allow"), "{out}");
}

#[test]
fn untrusted_queue_or_key_file_means_no_opinion() {
    need_tools!();
    // Group-readable run directory.
    let h = Host::new();
    chmod(&h.run(), 0o750);
    let (out, _) = h
        .spawn_hook("claude", "PermissionRequest", CLAUDE_IN, 2)
        .finish();
    assert_eq!(out, "");
    assert!(std::fs::read_dir(h.pending()).unwrap().next().is_none());
    // Group-writable allowed_signers: a valid-looking decision is not honoured.
    let h = Host::new();
    chmod(&h.home().join("allowed_signers"), 0o664);
    assert_ignored(&h, |h, i, now| h.honest(i, now, true));
    // Missing queue entirely.
    let empty = tempfile::tempdir().unwrap();
    chmod(empty.path(), 0o700);
    let mut c = Command::new("python3");
    c.arg("-I")
        .arg(script())
        .args(["claude", "PermissionRequest"])
        .env("SSH_APP_HOME", empty.path())
        .env("SSH_APP_HOOK_TIMEOUT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(CLAUDE_IN.as_bytes())
        .unwrap();
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    child.wait().unwrap();
    assert_eq!(out, "");
}

#[test]
fn flood_guard_refuses_to_queue_more() {
    need_tools!();
    let h = Host::new();
    for n in 0..100 {
        std::fs::write(h.pending().join(format!("{n:032x}.json")), "{}").unwrap();
    }
    let t = Instant::now();
    let (out, _) = h
        .spawn_hook("claude", "PermissionRequest", CLAUDE_IN, 10)
        .finish();
    assert_eq!(out, "");
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn notifications_are_listed_and_acknowledged() {
    need_tools!();
    let h = Host::new();
    let (out, _) = h.spawn_hook("claude", "Stop", CLAUDE_IN, 5).finish();
    assert_eq!(out, "");
    let (item, _) = h.wait_item("notify");
    let id = item["id"].as_str().unwrap().to_string();
    assert_eq!(h.ctl("ack", &json!({"id": id}).to_string())["ok"], true);
    assert!(h.ctl("list", "")["items"].as_array().unwrap().is_empty());
    // Antigravity Stop keeps its protocol answer.
    let (out, _) = h.spawn_hook("antigravity", "Stop", AGY_IN, 5).finish();
    assert!(out.contains("stop"), "{out}");
    // An approval request cannot be dismissed through `ack`.
    let hook = h.spawn_hook("claude", "PermissionRequest", CLAUDE_IN, 3);
    let (item, _) = h.wait_item("approval");
    h.ctl("ack", &json!({"id": item["id"]}).to_string());
    assert!(h.ctl("list", "")["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["kind"] == "approval"));
    hook.finish();
}

#[test]
fn ctl_decide_validates_its_input() {
    need_tools!();
    let h = Host::new();
    let none = "0".repeat(32);
    for body in [
        "".to_string(),
        "not json".to_string(),
        "[]".to_string(),
        json!({"id":"../../etc/passwd","approved":true,"ts":1,"signature":"x"}).to_string(),
        json!({"id":none,"approved":true,"ts":1,"signature":"x"}).to_string(), // unknown id
        "x".repeat(20_000),
    ] {
        assert_eq!(
            h.ctl("decide", &body)["ok"],
            false,
            "{}",
            &body[..body.len().min(40)]
        );
    }
    assert_eq!(h.ctl("nope", "")["ok"], false);
    let hook = h.spawn_hook("claude", "PermissionRequest", CLAUDE_IN, 3);
    let (item, now) = h.wait_item("approval");
    let id = item["id"].as_str().unwrap();
    for bad in [
        json!({"id":id,"approved":"yes","ts":now,"signature":"x"}),
        json!({"id":id,"approved":true,"ts":true,"signature":"x"}),
        json!({"id":id,"approved":true,"ts":now,"signature":""}),
        json!({"id":id,"approved":true,"ts":now,"signature":"x".repeat(9000)}),
    ] {
        assert_eq!(h.ctl("decide", &bad.to_string())["ok"], false, "{bad}");
    }
    hook.finish();
    // At most five decision files per request (no hook running, so nothing consumes them).
    let h = Host::new();
    let id = "4".repeat(32);
    let req = json!({"v":1,"id":id,"kind":"approval","agent":"claude","event":"e","tool":"t","command":"c","cwd":"/","session":"s","ts":1,"nonce":"n","command_sha256":"h","expires":99_999_999_999u64});
    std::fs::write(h.pending().join(format!("{id}.json")), req.to_string()).unwrap();
    let accepted = (0..8)
        .filter(|_| {
            h.ctl(
                "decide",
                &json!({"id":id,"approved":true,"ts":5,"signature":"x"}).to_string(),
            )["ok"]
                == true
        })
        .count();
    assert_eq!(accepted, 5);
}

#[test]
fn list_skips_symlinks_expired_and_oversized_entries() {
    need_tools!();
    let h = Host::new();
    let real = h.run().join("secret.json");
    std::fs::write(
        &real,
        json!({"id":"1".repeat(32),"kind":"approval"}).to_string(),
    )
    .unwrap();
    symlink(&real, h.pending().join(format!("{}.json", "1".repeat(32)))).unwrap();
    let expired = json!({"v":1,"id":"2".repeat(32),"kind":"approval","agent":"claude","event":"e","tool":"t","command":"c","cwd":"/","session":"s","ts":1,"nonce":"n","command_sha256":"h","expires":2});
    std::fs::write(
        h.pending().join(format!("{}.json", "2".repeat(32))),
        expired.to_string(),
    )
    .unwrap();
    std::fs::write(
        h.pending().join(format!("{}.json", "3".repeat(32))),
        "x".repeat(40_000),
    )
    .unwrap();
    std::fs::write(h.pending().join("not-an-id.json"), "{}").unwrap();
    let l = h.ctl("list", "");
    assert!(l["items"].as_array().unwrap().is_empty(), "{l}");
    assert!(l["host_time"].as_u64().unwrap() > 1_700_000_000);
}

#[test]
fn status_reports_capabilities() {
    need_tools!();
    let h = Host::new();
    let s = h.ctl("status", "");
    assert_eq!(
        (
            s["ssh_keygen"].as_bool(),
            s["allowed_signers"].as_bool(),
            s["pending"].as_u64()
        ),
        (Some(true), Some(true), Some(0))
    );
    std::fs::remove_file(h.home().join("allowed_signers")).unwrap();
    assert_eq!(h.ctl("status", "")["allowed_signers"], false);
}

#[test]
fn sigterm_removes_the_pending_request() {
    need_tools!();
    let h = Host::new();
    let mut hook = h.spawn_hook("claude", "PermissionRequest", CLAUDE_IN, 30);
    let _ = h.wait_item("approval");
    Command::new("kill")
        .args(["-TERM", &hook.0.id().to_string()])
        .status()
        .unwrap();
    hook.0.wait().unwrap();
    assert!(std::fs::read_dir(h.pending()).unwrap().next().is_none());
}

#[test]
fn long_and_hostile_text_is_truncated_and_valid_utf8() {
    need_tools!();
    let h = Host::new();
    let input = json!({"tool_name":"Bash","tool_input":{"command":"a".repeat(5000)},"cwd":"/p\u{202e}x","session_id":"s"}).to_string();
    let hook = h.spawn_hook("claude", "PermissionRequest", &input, 5);
    let (item, now) = h.wait_item("approval");
    assert_eq!(item["command"].as_str().unwrap().chars().count(), 2000);
    assert_eq!(
        item["command_sha256"].as_str().unwrap(),
        approval_request_digest(
            "claude",
            "Bash",
            item["cwd"].as_str().unwrap(),
            item["command"].as_str().unwrap()
        )
    );
    decide_ok(&h, &h.honest(&item, now, false));
    hook.finish();
}

#[test]
fn malformed_input_and_unknown_agent_are_harmless() {
    need_tools!();
    let h = Host::new();
    for (agent, event, input) in [
        ("claude", "PermissionRequest", "not json"),
        ("claude", "PermissionRequest", "[1,2]"),
        ("nobody", "PermissionRequest", "{}"),
        ("", "", ""),
        ("claude", "Stop", r#"{"tool_input":"x","cwd":5}"#),
        (
            "antigravity",
            "PreToolUse",
            r#"{"toolCall":"x","workspacePaths":7}"#,
        ),
    ] {
        let (_, code) = h.spawn_hook(agent, event, input, 1).finish();
        assert_eq!(code, 0, "{agent} {event} {input}");
    }
}

#[test]
fn fifos_in_the_queue_cannot_hang_the_hook_or_ctl() {
    need_tools!();
    let h = Host::new();
    let mkfifo = |p: &Path| assert!(Command::new("mkfifo").arg(p).status().unwrap().success());
    // A FIFO posing as a pending request: `ctl list` must not block opening it.
    mkfifo(&h.pending().join(format!("{}.json", "5".repeat(32))));
    let t = Instant::now();
    let l = h.ctl("list", "");
    assert!(l["items"].as_array().unwrap().is_empty());
    assert!(t.elapsed() < Duration::from_secs(5));
    // A FIFO posing as a decision: the hook skips it and still ends at its own deadline.
    let hook = h.spawn_hook("claude", "PermissionRequest", CLAUDE_IN, 2);
    let (item, _) = h.wait_item("approval");
    let id = item["id"].as_str().unwrap();
    mkfifo(&h.decisions().join(format!("{id}.{}.json", "0".repeat(16))));
    let t = Instant::now();
    let (out, _) = hook.finish();
    assert_eq!(out, "");
    assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
}
