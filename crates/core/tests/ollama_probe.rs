//! Runs the real Ollama probe command through `sh` against fake `ollama`/`curl` binaries, so the
//! quoting, the login-shell hop, the output markers and the host watchdog are exercised for real.
//! (A fake binary cannot be injected through sshd: the login shell there resets `PATH`.)

use ssh_core::{build_ollama_probe_command, parse_ollama_probe};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn script(dir: &Path, name: &str, body: &str) {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A `$SHELL` that puts the fake bin dir first on PATH and drops `-l` (a real login shell would
/// run /etc/profile, whose path_helper puts system dirs, and a real Homebrew `ollama`, first).
fn run_probe(dir: &Path) -> (String, Duration) {
    run_probe_with(dir, "sh")
}

fn run_probe_with(dir: &Path, outer_sh: &str) -> (String, Duration) {
    script(
        dir,
        "fakeshell",
        &format!(
            "PATH=\"{}:$PATH\"; export PATH\nif [ \"$1\" = \"-l\" ]; then shift; fi\nexec /bin/sh \"$@\"",
            dir.display()
        ),
    );
    let t = Instant::now();
    let out = Command::new(outer_sh)
        .args(["-c", &build_ollama_probe_command()])
        .env("SHELL", dir.join("fakeshell"))
        .stderr(Stdio::null())
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        t.elapsed(),
    )
}

#[test]
fn probe_reports_version_and_models_from_a_stopped_cli_and_running_server() {
    let dir = tempfile::tempdir().unwrap();
    // The real CLI prints these warnings (on stderr) before the version when the server is down.
    script(
        dir.path(),
        "ollama",
        "echo 'Warning: could not connect to a running Ollama instance' >&2\n\
         echo 'Warning: client version is 0.12.3' >&2",
    );
    script(
        dir.path(),
        "curl",
        "printf '%s\\n' '{\"models\":[{\"name\":\"qwen2.5-coder:7b\"},{\"name\":\"bad name; rm -rf ~\"}]}'",
    );
    let (out, _) = run_probe(dir.path());
    let s = parse_ollama_probe(out.clone());
    assert!(s.installed && s.running, "{out}");
    assert_eq!(s.version.as_deref(), Some("0.12.3"), "{out}");
    assert_eq!(s.models, ["qwen2.5-coder:7b"]);
    assert!(s.binary_path.unwrap().ends_with("/ollama"));
}

#[test]
fn probe_without_ollama_or_server_reports_nothing_installed_here() {
    let dir = tempfile::tempdir().unwrap();
    // `curl` that finds nothing; no `ollama` in the fake dir. Only meaningful on a machine that
    // has no real Ollama at the well-known paths.
    script(dir.path(), "curl", "exit 7");
    let (out, _) = run_probe(dir.path());
    let s = parse_ollama_probe(out);
    let real = [
        "/opt/homebrew/bin/ollama",
        "/usr/local/bin/ollama",
        "/Applications/Ollama.app/Contents/Resources/ollama",
    ]
    .iter()
    .any(|p| Path::new(p).exists());
    if !real {
        assert!(!s.installed && !s.running);
    }
    assert!(!s.running);
}

#[test]
fn hung_ollama_is_killed_by_the_host_watchdog() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    script(
        dir.path(),
        "ollama",
        &format!("echo $$ > {}\nexec sleep 120", pidfile.display()),
    );
    script(dir.path(), "curl", "exit 7");
    let (_, took) = run_probe(dir.path());
    assert!(took < Duration::from_secs(15), "{took:?}");
    std::thread::sleep(Duration::from_millis(300));
    let pid = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .to_string();
    let alive = Command::new("kill")
        .args(["-0", &pid])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!alive, "hung ollama {pid} was left running");
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[test]
fn watchdog_kills_the_whole_tree_under_dash() {
    // Linux hosts (Debian/Ubuntu) run exec commands through dash: no job control without a tty.
    if !Path::new("/bin/dash").exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    // A grandchild (sleep) that would keep the output pipe open.
    script(
        dir.path(),
        "ollama",
        &format!("sleep 120 &\necho $! > {}\nwait", pidfile.display()),
    );
    script(dir.path(), "curl", "exit 7");
    let (_, took) = run_probe_with(dir.path(), "/bin/dash");
    assert!(took < Duration::from_secs(15), "{took:?}");
    std::thread::sleep(Duration::from_millis(300));
    let pid = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .to_string();
    assert!(!alive(&pid), "grandchild {pid} survived");
}

#[test]
fn escape_sequences_in_host_output_cannot_forge_markers() {
    let dir = tempfile::tempdir().unwrap();
    script(dir.path(), "ollama", "echo 'client version is 1.2.3' >&2");
    // A listener answering with the two characters backslash-n inside the JSON body.
    script(
        dir.path(),
        "curl",
        "printf '%s\\n' '{\"models\":[{\"name\":\"a\\nOLLAMA_PATH=/evil\"}]}'",
    );
    let (out, _) = run_probe(dir.path());
    assert_eq!(
        out.lines()
            .filter(|l| l.starts_with("OLLAMA_PATH="))
            .count(),
        1,
        "{out}"
    );
    let s = parse_ollama_probe(out);
    assert!(s.running);
    assert_ne!(s.binary_path.as_deref(), Some("/evil"));
}

#[test]
fn oversized_version_output_is_bounded_on_the_host() {
    let dir = tempfile::tempdir().unwrap();
    // 50 MB without a newline, then the real line would be after it: head -c cuts it off.
    script(
        dir.path(),
        "ollama",
        "head -c 52428800 /dev/zero | tr '\\0' 'x'; echo; echo 'client version is 9.9.9'",
    );
    script(dir.path(), "curl", "exit 7");
    let (out, took) = run_probe(dir.path());
    assert!(took < Duration::from_secs(15), "{took:?}");
    assert_eq!(parse_ollama_probe(out).version, None);
}
