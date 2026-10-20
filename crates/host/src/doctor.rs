//! `doctor`: what the host looks like, as facts only (feature 44).
//!
//! Read-only: no file is written, no file's contents are returned (only whether it exists), no
//! shell runs. Tools are found by walking `PATH` and a few usual install folders; their versions
//! come from a fixed argv with a short limit. A fact that cannot be found is left out. Judging
//! the facts (is a hook outdated, is a plan exceeded) is the app's job.

use crate::proc::run_with_input;
use crate::queue::{allowed_signers_trusted, private_dir, Paths};
use serde_json::{json, Map, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const TOOL_LIMIT: Duration = Duration::from_secs(2);
const EXTRA_DIRS: [&str; 6] = [
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
    "/Applications/Tailscale.app/Contents/MacOS",
    "/snap/bin",
];
const HOME_DIRS: [&str; 5] = [
    ".local/bin",
    ".cargo/bin",
    ".npm-global/bin",
    ".claude/local",
    "go/bin",
];

pub struct Env {
    pub home: PathBuf,
    pub path: Vec<PathBuf>,
}

impl Env {
    pub fn from_env() -> Env {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()));
        let path = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        Env { home, path }
    }

    fn dirs(&self) -> Vec<PathBuf> {
        let mut dirs = self.path.clone();
        dirs.extend(HOME_DIRS.iter().map(|d| self.home.join(d)));
        dirs.extend(EXTRA_DIRS.iter().map(PathBuf::from));
        dirs
    }

    /// The first executable regular file called `name` (links are followed by `metadata`).
    pub fn find(&self, name: &str) -> Option<PathBuf> {
        self.dirs().into_iter().map(|d| d.join(name)).find(|p| {
            std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    }
}

fn run(bin: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(bin);
    cmd.args(args).env_remove("TMUX").env_remove("TMUX_PANE");
    let (code, out) = run_with_input(cmd, b"", TOOL_LIMIT)?;
    (code == Some(0)).then(|| String::from_utf8_lossy(&out).into_owned())
}

fn exists_file(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file())
}

/// A version number such as `3.4` or `2.43.0` out of `tmux 3.4` / `git version 2.43.0`.
pub fn version_in(text: &str) -> Option<String> {
    let line = text.lines().next()?;
    line.split_whitespace()
        .find(|w| {
            w.chars().next().is_some_and(|c| c.is_ascii_digit())
                && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
        })
        .map(|w| w.chars().take(24).collect())
}

pub fn ipv4_in(text: &str) -> Option<String> {
    let line = text.lines().next()?.trim();
    let parts: Vec<&str> = line.split('.').collect();
    let ok = parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u16>().is_ok_and(|n| n <= 255)
        });
    ok.then(|| line.to_string())
}

fn tool(env: &Env, name: &str, version_args: Option<&[&str]>) -> Value {
    let Some(bin) = env.find(name) else {
        return json!({"installed": false});
    };
    let mut t = Map::new();
    t.insert("installed".into(), true.into());
    if let Some(v) = version_args
        .and_then(|a| run(&bin, a))
        .and_then(|o| version_in(&o))
    {
        t.insert("version".into(), v.into());
    }
    Value::Object(t)
}

/// Whether something listens on the local Ollama port (a connect, nothing is sent).
fn ollama_running() -> bool {
    use std::net::{SocketAddr, TcpStream};
    let addr = SocketAddr::from(([127, 0, 0, 1], 11434));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

pub fn report(env: &Env, paths: &Paths, version: &str, ollama_up: bool) -> Value {
    let home = &env.home;
    let mut tailscale = tool(env, "tailscale", None);
    if let Some(ip) = env
        .find("tailscale")
        .and_then(|b| run(&b, &["ip", "-4"]))
        .and_then(|o| ipv4_in(&o))
    {
        tailscale["ip"] = ip.into();
    }
    let claude_dir = home.join(".claude");
    let hooks_file = home.join(".gemini/config/hooks.json");
    let mut ollama = tool(env, "ollama", None);
    ollama["running"] = (ollama["installed"] == true && ollama_up).into();
    json!({
        "ok": true,
        "version": version,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "tmux": tool(env, "tmux", Some(&["-V"])),
        "git": tool(env, "git", Some(&["--version"])),
        "python3": tool(env, "python3", None),
        "tailscale": tailscale,
        "agents": {
            "claude": {
                "found": env.find("claude").is_some() || claude_dir.is_dir(),
                "config_dir": "~/.claude",
                "settings_exists": exists_file(&claude_dir.join("settings.json")),
            },
            "antigravity": {
                "found": env.find("agy").is_some() || home.join(".gemini").is_dir(),
                "hooks_file": "~/.gemini/config/hooks.json",
                "hooks_file_exists": exists_file(&hooks_file),
            },
            "ollama": ollama,
        },
        "ssh_app": {
            "allowed_signers": allowed_signers_trusted(paths),
            "hook_script": exists_file(&paths.home.join("bin/ssh-app-hook")),
            "helper_dir_private": private_dir(&paths.home),
        },
    })
}

/// The `doctor` subcommand.
pub fn doctor(version: &str) -> Value {
    report(
        &Env::from_env(),
        &Paths::from_env(),
        version,
        ollama_running(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &Path, name: &str, body: &str) {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn fixture() -> (tempfile::TempDir, Env, Paths) {
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let env = Env {
            home: t.path().to_path_buf(),
            path: vec![bin],
        };
        let paths = Paths::from_env();
        let paths = Paths {
            home: t.path().join(".ssh-app"),
            ..paths
        };
        (t, env, paths)
    }

    #[test]
    fn versions_and_addresses_are_read_from_the_first_line() {
        assert_eq!(version_in("tmux 3.4\n").as_deref(), Some("3.4"));
        assert_eq!(
            version_in("git version 2.43.0\nmore").as_deref(),
            Some("2.43.0")
        );
        assert_eq!(version_in("tmux next-3.5"), None);
        assert_eq!(version_in(""), None);
        assert_eq!(
            ipv4_in("100.92.14.88\nfd7a::1").as_deref(),
            Some("100.92.14.88")
        );
        for bad in [
            "",
            "1.2.3",
            "1.2.3.999",
            "a.b.c.d",
            "1.2.3.4.5",
            "100.1.1.1; rm",
        ] {
            assert_eq!(ipv4_in(bad), None, "{bad}");
        }
    }

    #[test]
    fn tools_are_found_on_the_given_path_without_a_shell() {
        let (t, env, paths) = fixture();
        script(&t.path().join("bin"), "tmux", "echo tmux 3.4");
        script(&t.path().join("bin"), "git", "echo git version 2.43.0");
        let r = report(&env, &paths, "9.9.9", false);
        assert_eq!(r["ok"], true);
        assert_eq!(r["version"], "9.9.9");
        assert_eq!(r["tmux"], json!({"installed": true, "version": "3.4"}));
        assert_eq!(r["git"]["version"], "2.43.0");
        assert!(r["python3"]["installed"].is_boolean());
    }

    #[test]
    fn a_missing_tool_is_not_installed_and_has_no_version() {
        let (_t, env, paths) = fixture();
        let r = report(&env, &paths, "1", false);
        assert_eq!(r["tmux"]["installed"], env.find("tmux").is_some());
        let mut env2 = env;
        env2.home = PathBuf::from("/nonexistent-home");
        env2.path = vec![];
        assert_eq!(env2.find("surely-not-a-tool-name"), None);
    }

    #[test]
    fn a_failing_or_garbage_version_command_leaves_the_version_out() {
        let (t, env, paths) = fixture();
        script(&t.path().join("bin"), "tmux", "exit 3");
        script(&t.path().join("bin"), "git", "echo ';;; hostile output'");
        let r = report(&env, &paths, "1", false);
        assert_eq!(r["tmux"]["installed"], true);
        assert!(r["tmux"].get("version").is_none());
        assert!(r["git"].get("version").is_none());
    }

    #[test]
    fn a_slow_tool_is_cut_off() {
        let (t, env, paths) = fixture();
        script(&t.path().join("bin"), "tmux", "sleep 30");
        let started = std::time::Instant::now();
        let r = report(&env, &paths, "1", false);
        assert!(started.elapsed() < Duration::from_secs(8));
        assert!(r["tmux"].get("version").is_none());
    }

    #[test]
    fn tailscale_reports_an_address_only_when_it_looks_like_one() {
        let (t, env, paths) = fixture();
        script(&t.path().join("bin"), "tailscale", "echo 100.64.0.7");
        assert_eq!(
            report(&env, &paths, "1", false)["tailscale"]["ip"],
            "100.64.0.7"
        );
        script(&t.path().join("bin"), "tailscale", "echo 'not an ip'");
        assert!(report(&env, &paths, "1", false)["tailscale"]
            .get("ip")
            .is_none());
    }

    #[test]
    fn agents_are_facts_about_files_and_never_their_contents() {
        let (t, env, paths) = fixture();
        std::fs::create_dir_all(t.path().join(".claude")).unwrap();
        std::fs::write(t.path().join(".claude/settings.json"), "SECRET-TOKEN").unwrap();
        std::fs::create_dir_all(t.path().join(".gemini/config")).unwrap();
        script(&t.path().join("bin"), "ollama", "true");
        let r = report(&env, &paths, "1", true);
        assert_eq!(r["agents"]["claude"]["found"], true);
        assert_eq!(r["agents"]["claude"]["settings_exists"], true);
        assert_eq!(r["agents"]["antigravity"]["found"], true);
        assert_eq!(r["agents"]["antigravity"]["hooks_file_exists"], false);
        assert_eq!(
            r["agents"]["ollama"],
            json!({"installed": true, "running": true})
        );
        assert!(!r.to_string().contains("SECRET-TOKEN"));
        assert!(!r.to_string().contains(t.path().to_str().unwrap()));
    }

    #[test]
    fn ollama_running_needs_the_binary() {
        let (_t, env, paths) = fixture();
        let r = report(&env, &paths, "1", true);
        if env.find("ollama").is_none() {
            assert_eq!(r["agents"]["ollama"]["running"], false);
        }
    }

    #[test]
    fn doctor_writes_nothing() {
        let (t, env, paths) = fixture();
        let before = std::fs::read_dir(t.path()).unwrap().count();
        let _ = report(&env, &paths, "1", false);
        let after = std::fs::read_dir(t.path()).unwrap().count();
        assert_eq!(before, after);
        assert!(!paths.home.exists());
    }
}
