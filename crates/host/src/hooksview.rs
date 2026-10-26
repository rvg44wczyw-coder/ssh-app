//! What the Hooks screen needs from the host (spec 61): which agents exist here, which projects they
//! were used in, and whether our hooks are in each agent's user-level file and in each project's file.
//! Read-only: installing and removing stay in the core.

use crate::usage::{home, jsonl_files, mtime, read_procs, regular, run, Detector};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MARKER: &str = "ssh-app-hook";
const MAX_FOLDERS: usize = 64;
const MAX_FILE: u64 = 1 << 20;

struct Agent {
    id: &'static str,
    binary: &'static str,
    /// User-level hook file, relative to `$HOME`.
    user_file: &'static str,
    /// Project-level hook file, relative to the project (None = not available yet).
    project_file: Option<&'static str>,
}

const AGENTS: [Agent; 7] = [
    Agent {
        id: "claude",
        binary: "claude",
        user_file: ".claude/settings.json",
        project_file: Some(".claude/settings.local.json"),
    },
    Agent {
        id: "agy",
        binary: "agy",
        user_file: ".gemini/config/hooks.json",
        project_file: Some(".agents/hooks.json"),
    },
    Agent {
        id: "codex",
        binary: "codex",
        user_file: ".codex/hooks.json",
        project_file: Some(".codex/hooks.json"),
    },
    Agent {
        id: "grok",
        binary: "grok",
        user_file: ".grok/hooks/ssh-app.json",
        project_file: Some(".grok/hooks/ssh-app.json"),
    },
    Agent {
        id: "qwen",
        binary: "qwen",
        user_file: ".qwen/settings.json",
        project_file: Some(".qwen/settings.json"),
    },
    Agent {
        id: "crush",
        binary: "crush",
        user_file: ".config/crush/crush.json",
        project_file: None,
    },
    Agent {
        id: "opencode",
        binary: "opencode",
        user_file: ".config/opencode/plugins/ssh-app.js",
        project_file: Some(".opencode/plugins/ssh-app.js"),
    },
];

/// `none` (no file or no hooks), `ours` (our marker is in it) or `foreign` (other hooks only).
pub fn hook_state(path: &Path) -> &'static str {
    if regular(path).is_none_or(|m| m.len() > MAX_FILE) {
        return "none";
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return "none";
    };
    if text.contains(MARKER) {
        "ours"
    } else if serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("hooks").cloned())
        .is_some_and(|h| h.as_object().is_some_and(|o| !o.is_empty()) || h.is_array())
    {
        "foreign"
    } else {
        "none"
    }
}

fn on_path(binary: &str) -> bool {
    let h = home();
    let mut dirs: Vec<PathBuf> = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(PathBuf::from)
        .collect();
    for extra in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        ".local/bin",
        ".claude/local",
        ".npm-global/bin",
        ".cargo/bin",
        ".bun/bin",
        ".grok/bin",
        ".gemini/antigravity-cli/bin",
    ] {
        dirs.push(if extra.starts_with('/') {
            PathBuf::from(extra)
        } else {
            h.join(extra)
        });
    }
    dirs.iter().any(|d| {
        regular(&d.join(binary))
            .is_some_and(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0)
    })
}

fn expand_home(f: &str) -> PathBuf {
    match f.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if f == "~" => home(),
        None => PathBuf::from(f),
    }
}

/// A folder worth listing: it exists, is a directory and belongs to the current user.
fn usable(p: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(p).ok()?;
    let m = std::fs::metadata(&real).ok()?;
    // SAFETY: geteuid has no preconditions.
    (m.is_dir() && m.uid() == unsafe { libc::geteuid() }).then_some(real)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

type Found = HashMap<(&'static str, PathBuf), (f64, bool)>;

fn note(found: &mut Found, agent: &'static str, folder: &Path, at: f64, running: bool) {
    let Some(real) = usable(folder) else { return };
    let e = found.entry((agent, real)).or_insert((0.0, false));
    e.0 = e.0.max(at);
    e.1 |= running;
}

fn claude_projects(found: &mut Found, deadline: Instant) {
    let root = home().join(".claude").join("projects");
    let Ok(rd) = std::fs::read_dir(&root) else {
        return;
    };
    for e in rd.flatten().take(256) {
        if Instant::now() > deadline {
            return;
        }
        let files = jsonl_files(&e.path(), 0, deadline);
        let Some((file, at)) = files
            .iter()
            .filter_map(|f| Some((f, mtime(&regular(f)?))))
            .max_by(|a, b| a.1.total_cmp(&b.1))
        else {
            continue;
        };
        let Ok(f) = std::fs::File::open(file) else {
            continue;
        };
        for line in
            std::io::BufRead::lines(std::io::BufReader::new(std::io::Read::take(f, 262_144)))
                .take(40)
                .map_while(Result::ok)
        {
            if let Some(cwd) = serde_json::from_str::<Value>(&line)
                .ok()
                .and_then(|v| v.get("cwd").and_then(Value::as_str).map(str::to_string))
            {
                note(found, "claude", Path::new(&cwd), at, false);
                break;
            }
        }
    }
}

fn codex_projects(found: &mut Found, deadline: Instant) {
    let root = std::env::var_os("CODEX_HOME")
        .map_or_else(|| home().join(".codex"), PathBuf::from)
        .join("sessions");
    let mut files: Vec<(PathBuf, f64)> = jsonl_files(&root, 4, deadline)
        .into_iter()
        .filter_map(|f| {
            let at = mtime(&regular(&f)?);
            Some((f, at))
        })
        .collect();
    files.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (file, at) in files.into_iter().take(64) {
        let Ok(f) = std::fs::File::open(&file) else {
            continue;
        };
        let mut line = String::new();
        if std::io::BufRead::read_line(
            &mut std::io::BufReader::new(std::io::Read::take(f, 65_536)),
            &mut line,
        )
        .is_err()
        {
            continue;
        }
        // Only the folder: the account fields of session_meta are never read.
        if let Some(cwd) = serde_json::from_str::<Value>(&line)
            .ok()
            .and_then(|v| v["payload"]["cwd"].as_str().map(str::to_string))
        {
            note(found, "codex", Path::new(&cwd), at, false);
        }
    }
}

fn grok_projects(found: &mut Found) {
    let root = std::env::var_os("GROK_HOME")
        .map_or_else(|| home().join(".grok"), PathBuf::from)
        .join("sessions");
    let Ok(rd) = std::fs::read_dir(&root) else {
        return;
    };
    for e in rd.flatten().take(256) {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("%2F") {
            continue;
        }
        let at = std::fs::metadata(e.path()).map_or(0.0, |m| mtime(&m));
        note(found, "grok", Path::new(&percent_decode(&name)), at, false);
    }
}

fn agy_projects(found: &mut Found) {
    let file = home()
        .join(".gemini")
        .join("antigravity-cli")
        .join("cache")
        .join("last_conversations.json");
    let at = regular(&file).map_or(0.0, |m| mtime(&m));
    let Some(map) = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        return;
    };
    for key in map.as_object().into_iter().flat_map(|o| o.keys()).take(256) {
        note(found, "agy", Path::new(key), at, false);
    }
}

fn running_panes(found: &mut Found) {
    let panes = run(
        "tmux",
        &[
            "list-panes",
            "-a",
            "-F",
            "#{pane_pid}\t#{pane_current_path}",
        ],
        8,
    );
    let procs = read_procs();
    let detector = Detector::new();
    for line in panes.lines() {
        let Some((pid, path)) = line.split_once('\t') else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if let Some((agent, _)) = detector.detect(&procs, pid) {
            if let Some(id) = AGENTS.iter().find(|a| a.id == agent).map(|a| a.id) {
                note(found, id, Path::new(path), crate::usage::now(), true);
            }
        }
    }
}

/// The reply of `hooks-overview`: every agent, whether it is installed here, its user-level hook state
/// and the projects (running panes, agent histories and the `folders` the phone asked about).
pub fn overview(folders: &[String], claude_dir: Option<&str>) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut found: Found = HashMap::new();
    running_panes(&mut found);
    claude_projects(&mut found, deadline);
    codex_projects(&mut found, deadline);
    grok_projects(&mut found);
    agy_projects(&mut found);
    for f in folders.iter().take(MAX_FOLDERS) {
        let path = expand_home(f);
        if path.is_absolute() {
            for a in &AGENTS {
                note(&mut found, a.id, &path, 0.0, false);
            }
        }
    }
    let h = home();
    let agents: Vec<Value> = AGENTS
        .iter()
        .map(|a| {
            let mut projects: Vec<(&PathBuf, &(f64, bool))> = found
                .iter()
                .filter(|((id, _), _)| *id == a.id)
                .map(|((_, p), v)| (p, v))
                .collect();
            projects.sort_by(|x, y| y.1 .1.cmp(&x.1 .1).then(y.1 .0.total_cmp(&x.1 .0)));
            let projects: Vec<Value> = projects
                .into_iter()
                .take(MAX_FOLDERS)
                .map(|(p, (at, running))| {
                    let file = a.project_file.map(|f| p.join(f));
                    json!({
                        "folder": p.to_string_lossy(),
                        "file": file.as_ref().map(|f| f.to_string_lossy().into_owned()),
                        "state": file.as_deref().map_or("unavailable", hook_state),
                        "running": running,
                        "last_used": *at as i64,
                    })
                })
                .collect();
            let user = match (a.id, claude_dir) {
                ("claude", Some(d)) => expand_home(d).join("settings.json"),
                _ => h.join(a.user_file),
            };
            json!({
                "agent": a.id,
                "found": on_path(a.binary) || found.keys().any(|(id, _)| *id == a.id),
                "user": {"file": user.to_string_lossy(), "state": hook_state(&user)},
                "projects": projects,
            })
        })
        .collect();
    json!({"ok": true, "agents": agents})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_state_tells_ours_from_foreign_from_nothing() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("s.json");
        assert_eq!(hook_state(&f), "none");
        std::fs::write(&f, r#"{"model":"x"}"#).unwrap();
        assert_eq!(hook_state(&f), "none");
        std::fs::write(&f, r#"{"hooks":{"Stop":[{"hooks":[{"command":"mine"}]}]}}"#).unwrap();
        assert_eq!(hook_state(&f), "foreign");
        std::fs::write(
            &f,
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"python3 -I \"$HOME/.ssh-app/bin/ssh-app-hook\" claude Stop"}]}]}}"#,
        )
        .unwrap();
        assert_eq!(hook_state(&f), "ours");
    }

    #[test]
    fn grok_folder_names_decode() {
        assert_eq!(
            percent_decode("%2FUsers%2Fme%2Fmy%20dir"),
            "/Users/me/my dir"
        );
        assert_eq!(percent_decode("%2"), "%2");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn a_folder_the_phone_names_is_listed_for_every_agent_with_its_state() {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("proj");
        std::fs::create_dir_all(proj.join(".claude")).unwrap();
        std::fs::write(
            proj.join(".claude/settings.local.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"ssh-app-hook x"}]}]}}"#,
        )
        .unwrap();
        let v = overview(&[proj.to_string_lossy().into_owned()], None);
        let real = std::fs::canonicalize(&proj).unwrap();
        let claude = &v["agents"][0];
        assert_eq!(claude["agent"], "claude");
        let row = claude["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["folder"] == real.to_string_lossy().as_ref())
            .expect("listed");
        assert_eq!(row["state"], "ours");
        let codex = v["agents"][2]["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["folder"] == real.to_string_lossy().as_ref())
            .expect("listed");
        assert_eq!(codex["state"], "none");
        let crush = v["agents"][5]["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["folder"] == real.to_string_lossy().as_ref())
            .expect("listed");
        assert_eq!(crush["state"], "unavailable");
        assert!(v["agents"][1]["projects"].as_array().unwrap().len() <= MAX_FOLDERS);
    }

    #[test]
    fn hostile_folders_are_dropped() {
        let v = overview(
            &[
                "relative".into(),
                "/nonexistent/x".into(),
                "/etc/passwd".into(),
                "~/../../etc".into(),
            ],
            None,
        );
        for a in v["agents"].as_array().unwrap() {
            for p in a["projects"].as_array().unwrap() {
                assert_ne!(p["folder"], "/etc/passwd");
            }
        }
    }
}
