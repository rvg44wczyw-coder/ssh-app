//! Runs the real `ssh-app-hook-extra.py` wrapper against a stand-in hook in a temporary directory
//! (no agent, no tmux, no network): order, stop on failure, timeouts, stdin, output handling and
//! hostile arguments. Skipped when `python3` is unavailable.

use base64::Engine;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn wrapper() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/agent-hooks/ssh-app-hook-extra.py")
}

fn have_python() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        Env {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// A stand-in for the ssh-app hook: appends "hook" to the trace, prints what it got on stdin
    /// (so the test sees it was passed through), exits with `code`.
    fn hook(&self, code: i32) -> PathBuf {
        let p = self.path("hook.sh");
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\necho hook >> '{trace}'\ncat\nexit {code}\n",
                trace = self.path("trace").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
        p
    }

    fn trace(&self) -> String {
        std::fs::read_to_string(self.path("trace")).unwrap_or_default()
    }

    /// Runs the wrapper; returns (stdout, exit code, seconds).
    fn run(&self, args: &[&str], stdin: &str) -> (String, i32, f64) {
        let started = Instant::now();
        let mut child = Command::new("python3")
            .arg("-I")
            .arg(wrapper())
            .args(args)
            .env("SSH_APP_HOME", self.dir.path().join("home"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Written from a thread: the wrapper stops reading at its cap, so a big write must not
        // block the test (a closed pipe is fine).
        let mut pipe = child.stdin.take().unwrap();
        let input = stdin.to_owned();
        let writer = std::thread::spawn(move || {
            let _ = pipe.write_all(input.as_bytes());
        });
        let out = child.wait_with_output().unwrap();
        let _ = writer.join();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code().unwrap_or(-1),
            started.elapsed().as_secs_f64(),
        )
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.path("home/run/extra.log")).unwrap_or_default()
    }
}

fn b64(s: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s)
}

#[allow(clippy::too_many_arguments)]
fn args<'a>(
    cmd: &'a str,
    order: &'a str,
    stop: &'a str,
    timeout: &'a str,
    agent: &'a str,
    event: &'a str,
    hook: &'a Path,
) -> Vec<String> {
    let _ = (agent, event);
    vec![
        "--cmd".into(),
        b64(cmd),
        "--order".into(),
        order.into(),
        "--stop".into(),
        stop.into(),
        "--timeout".into(),
        timeout.into(),
        "--agent".into(),
        agent.into(),
        "--event".into(),
        event.into(),
        "--".into(),
        hook.to_string_lossy().into_owned(),
    ]
}

fn go(env: &Env, a: Vec<String>, stdin: &str) -> (String, i32, f64) {
    let refs: Vec<&str> = a.iter().map(String::as_str).collect();
    env.run(&refs, stdin)
}

#[test]
fn before_runs_the_extra_first_then_the_hook_with_the_same_input() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(0);
    let cmd = format!(
        "cat > '{}'; echo extra >> '{}'",
        e.path("got").display(),
        e.path("trace").display()
    );
    let (out, code, _) = go(
        &e,
        args(&cmd, "before", "0", "10", "claude", "Stop", &hook),
        "{\"a\":1}",
    );
    assert_eq!(code, 0);
    assert_eq!(
        out, "{\"a\":1}",
        "the hook's stdout is passed on, so it got the input too"
    );
    assert_eq!(e.trace(), "extra\nhook\n");
    assert_eq!(std::fs::read_to_string(e.path("got")).unwrap(), "{\"a\":1}");
}

#[test]
fn after_runs_the_hook_first() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(0);
    let cmd = format!("echo extra >> '{}'", e.path("trace").display());
    let (_, code, _) = go(
        &e,
        args(&cmd, "after", "0", "10", "claude", "Stop", &hook),
        "{}",
    );
    assert_eq!(code, 0);
    assert_eq!(e.trace(), "hook\nextra\n");
}

#[test]
fn the_extras_output_never_reaches_the_agent_and_the_hooks_exit_code_does() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(2);
    let (out, code, _) = go(
        &e,
        args(
            "echo '{\"decision\":\"allow\"}'; echo oops >&2",
            "before",
            "0",
            "10",
            "claude",
            "Stop",
            &hook,
        ),
        "in",
    );
    assert_eq!(out, "in", "only the hook's output is passed on");
    assert_eq!(code, 2, "and its exit code");
}

#[test]
fn a_failed_extra_with_stop_denies_and_skips_the_hook() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(0);
    for (agent, event, expect) in [
        ("claude", "PermissionRequest", "\"behavior\": \"deny\""),
        ("claude", "PreToolUse", "\"permissionDecision\": \"deny\""),
        ("antigravity", "PreToolUse", "\"decision\": \"deny\""),
        ("crush", "PreToolUse", "\"version\": 1"),
    ] {
        let (out, code, _) = go(
            &e,
            args("exit 1", "before", "1", "10", agent, event, &hook),
            "{}",
        );
        assert_eq!(code, 0, "{agent} {event}");
        assert!(out.contains(expect), "{agent} {event}: {out}");
        assert!(out.contains("Blocked by the extra hook command"));
    }
    assert_eq!(e.trace(), "", "the hook never ran");
    assert!(e.log().contains("exit 1"));
}

#[test]
fn a_failed_extra_without_stop_lets_the_hook_run() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(0);
    let (out, _, _) = go(
        &e,
        args(
            "exit 3",
            "before",
            "0",
            "10",
            "claude",
            "PermissionRequest",
            &hook,
        ),
        "x",
    );
    assert_eq!(out, "x");
    assert_eq!(e.trace(), "hook\n");
}

#[test]
fn stop_is_ignored_where_nothing_can_be_blocked_or_when_running_after() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(0);
    let (_, _, _) = go(
        &e,
        args("exit 1", "before", "1", "10", "claude", "Stop", &hook),
        "{}",
    );
    let (_, _, _) = go(
        &e,
        args(
            "exit 1",
            "after",
            "1",
            "10",
            "claude",
            "PermissionRequest",
            &hook,
        ),
        "{}",
    );
    assert_eq!(e.trace(), "hook\nhook\n", "the hook ran both times");
}

#[test]
fn a_command_that_runs_too_long_is_killed_with_its_children() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(0);
    let mark = e.path("child-alive");
    let cmd = format!("(sleep 3; touch '{}') & sleep 30", mark.display());
    let (out, code, secs) = go(
        &e,
        args(
            &cmd,
            "before",
            "1",
            "1",
            "claude",
            "PermissionRequest",
            &hook,
        ),
        "{}",
    );
    assert!(secs < 6.0, "returned after the timeout, took {secs}s");
    assert_eq!(code, 0);
    assert!(out.contains("deny"), "a timeout counts as a failure: {out}");
    std::thread::sleep(Duration::from_millis(3500));
    assert!(
        !mark.exists(),
        "the background child was killed with the group"
    );
    assert!(e.log().contains("timed out"));
}

#[test]
fn hostile_text_in_the_command_stays_data() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(0);
    let canary = e.path("canary");
    // The command is run by sh on purpose; what must not happen is the *wrapper* or the argument
    // passing interpreting it: quotes, newlines-in-base64 and options inside the text stay inside.
    let cmd = format!(
        "printf %s '--stop 1 -- evil; touch {}' > /dev/null",
        canary.display()
    );
    let (_, code, _) = go(
        &e,
        args(&cmd, "before", "0", "10", "claude", "Stop", &hook),
        "{}",
    );
    assert_eq!(code, 0);
    assert!(!canary.exists());
    assert_eq!(e.trace(), "hook\n");
}

#[test]
fn bad_arguments_mean_no_opinion() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(0);
    let good = args("true", "before", "0", "10", "claude", "Stop", &hook);
    let mutate = |f: &dyn Fn(&mut Vec<String>)| {
        let mut a = good.clone();
        f(&mut a);
        go(&e, a, "{}")
    };
    // no separator, odd option count, bad order, bad timeouts, bad base64, empty/multiline command
    for (out, code, _) in [
        e.run(&[], ""),
        e.run(&["--cmd"], ""),
        mutate(&|a| a.retain(|x| x != "--")),
        mutate(&|a| a[3] = "sideways".into()),
        mutate(&|a| a[7] = "0".into()),
        mutate(&|a| a[7] = "61".into()),
        mutate(&|a| a[7] = "nan".into()),
        mutate(&|a| a[1] = "!!!".into()),
        mutate(&|a| a[1] = b64("")),
        mutate(&|a| a[1] = b64("a\nb")),
        mutate(&|a| a.truncate(13)),
    ] {
        assert_eq!((out.as_str(), code), ("", 0));
    }
    assert_eq!(e.trace(), "", "the hook is not run on bad input");
}

#[test]
fn a_missing_hook_and_a_huge_input_do_not_crash_the_wrapper() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let missing = e.path("nothing-here");
    let (out, code, _) = go(
        &e,
        args("true", "before", "0", "10", "claude", "Stop", &missing),
        "{}",
    );
    assert_eq!((out.as_str(), code), ("", 0));
    assert!(e.log().contains("cannot start"));
    let hook = e.hook(0);
    let big = "x".repeat(3 << 20);
    let (out, code, _) = go(
        &e,
        args("true", "before", "0", "10", "claude", "Stop", &hook),
        &big,
    );
    assert_eq!(code, 0);
    assert_eq!(
        out.len(),
        3 << 20,
        "the hook gets the whole event: a cut-off one would make it fail to parse and let a guarded call through"
    );
}

#[test]
fn the_extra_command_sees_at_most_one_megabyte() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(0);
    let seen = e.path("seen");
    let cmd = format!("wc -c > '{}'", seen.display());
    let big = "x".repeat(3 << 20);
    go(
        &e,
        args(&cmd, "before", "0", "10", "claude", "Stop", &hook),
        &big,
    );
    let n: usize = std::fs::read_to_string(&seen)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(n, 1 << 20);
}

#[test]
fn a_child_left_in_the_background_does_not_hold_the_wrapper() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(0);
    let (out, code, secs) = go(
        &e,
        args(
            "sleep 5 & true",
            "before",
            "1",
            "20",
            "claude",
            "PreToolUse",
            &hook,
        ),
        "{}",
    );
    assert!(secs < 2.0, "returned at once, took {secs}s");
    assert_eq!(code, 0);
    assert!(!out.contains("deny"), "the command itself succeeded: {out}");
}

#[test]
fn a_flood_on_stderr_is_not_kept_and_escapes_do_not_reach_the_log() {
    if !have_python() {
        return;
    }
    let e = Env::new();
    let hook = e.hook(0);
    go(
        &e,
        args(
            "head -c 50000000 /dev/zero >&2; printf '\\033]0;x\\007' >&2; exit 1",
            "before",
            "0",
            "20",
            "claude",
            "Stop",
            &hook,
        ),
        "{}",
    );
    let log = e.log();
    assert!(log.contains("extra: exit 1"), "{log}");
    assert!(log.len() < 1000, "the log line is short: {}", log.len());
    assert!(!log.contains('\u{1b}') && !log.contains('\u{7}'), "{log:?}");
}
