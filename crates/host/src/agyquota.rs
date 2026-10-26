//! Antigravity plan limits. agy shows them only on its `/usage` screen ("Models & Quota"), they are not
//! on disk, so the host reads that screen: `parse` turns the text into groups, `refresh` drives a
//! private agy in its own tmux server (never the user's) and caches the result for the usage report.

use regex::Regex;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A cache younger than this is used as is.
const FRESH_SECS: f64 = 900.0;
/// A cache older than this is not shown (the numbers would mislead).
const STALE_SECS: f64 = 86_400.0;
/// A refresh claimed less than this ago is not started again.
const LOCK_SECS: f64 = 120.0;

pub(crate) fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

pub(crate) fn run_dir() -> PathBuf {
    std::env::var_os("SSH_APP_HOME")
        .map_or_else(
            || {
                std::env::var_os("HOME")
                    .map_or_else(|| PathBuf::from("/"), PathBuf::from)
                    .join(".ssh-app")
            },
            PathBuf::from,
        )
        .join("run")
}

/// One window of one model group: what is left (percent) and when it refills.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub minutes: u32,
    pub remaining: f64,
    pub resets_epoch: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// Lower-cased model family names of the group ("gemini flash", "claude opus", …).
    pub models: Vec<String>,
    pub windows: Vec<Window>,
}

fn window_minutes(title: &str) -> Option<u32> {
    let t = title.to_lowercase();
    if !t.contains("limit remaining") {
        return None;
    }
    if t.contains("five hour") || t.contains("5 hour") {
        Some(300)
    } else if t.contains("week") {
        Some(10_080)
    } else if t.contains("day") || t.contains("daily") {
        Some(1_440)
    } else {
        None
    }
}

/// The groups of the `/usage` screen `text`; `at` is the capture time (reset durations are relative).
pub fn parse(text: &str, at: f64) -> Vec<Group> {
    let (Ok(pct_rx), Ok(reset_rx)) = (
        Regex::new(r"(\d{1,3}(?:\.\d+)?)\s*%"),
        Regex::new(r"Refreshes in\s+(?:(\d+)h)?\s*(?:(\d+)m)?"),
    ) else {
        return vec![];
    };
    let mut groups: Vec<Group> = vec![];
    let mut pending: Option<u32> = None;
    let mut open: Option<usize> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let heading = !raw.starts_with(' ')
            && line.chars().any(|c| c.is_ascii_uppercase())
            && line
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ');
        if heading {
            groups.push(Group {
                models: vec![],
                windows: vec![],
            });
            open = Some(groups.len() - 1);
            pending = None;
            continue;
        }
        let Some(g) = open else { continue };
        if let Some(rest) = line.strip_prefix("Models within this group:") {
            groups[g].models = rest
                .split(',')
                .map(|m| m.trim().to_lowercase())
                .filter(|m| !m.is_empty())
                .collect();
        } else if let Some(minutes) = window_minutes(line) {
            pending = Some(minutes);
        } else if let Some(minutes) = pending {
            if let Some(p) = pct_rx.captures(line).and_then(|c| c[1].parse::<f64>().ok()) {
                if (0.0..=100.0).contains(&p) {
                    groups[g].windows.push(Window {
                        minutes,
                        remaining: p,
                        resets_epoch: None,
                    });
                }
                // Wait for the line under the bar; a window without one is already complete.
            } else if let Some(c) = reset_rx.captures(line) {
                let num = |i| c.get(i).and_then(|m| m.as_str().parse::<u64>().ok());
                let secs = num(1).unwrap_or(0) * 3600 + num(2).unwrap_or(0) * 60;
                if let Some(w) = groups[g].windows.last_mut() {
                    if w.minutes == minutes && secs > 0 {
                        w.resets_epoch = Some((at + secs as f64) as i64);
                    }
                }
                pending = None;
            } else if line.contains("Quota available") {
                pending = None;
            }
        }
    }
    groups.retain(|g| !g.windows.is_empty());
    groups
}

/// Picks the group of `model` ("gemini-3.8-flash-high", "Claude Opus 5.5 (High)"); when none matches the
/// worst window of each size across all groups stands in.
pub fn limits_for(groups: &[Group], model: Option<&str>, at: f64) -> Vec<(u32, f64, Option<i64>)> {
    let m = model
        .unwrap_or("")
        .to_lowercase()
        .replace(['-', '.', '_'], " ");
    let hit = groups.iter().find(|g| {
        g.models.iter().any(|name| {
            name.split_whitespace()
                .last()
                .is_some_and(|family| m.split_whitespace().any(|w| w == family))
        })
    });
    let live = |w: &Window| w.resets_epoch.is_none_or(|r| (r as f64) > at);
    let mut out: Vec<(u32, f64, Option<i64>)> = vec![];
    let pool: Vec<&Group> = hit.map_or_else(|| groups.iter().collect(), |g| vec![g]);
    for g in pool {
        for w in g.windows.iter().filter(|w| live(w)) {
            let used = 100.0 - w.remaining;
            match out.iter_mut().find(|o| o.0 == w.minutes) {
                Some(o) if o.1 >= used => {}
                Some(o) => *o = (w.minutes, used, w.resets_epoch),
                None => out.push((w.minutes, used, w.resets_epoch)),
            }
        }
    }
    out.sort_by_key(|o| o.0);
    out
}

/// A folder whose own agent config would run code when a fresh agent starts there (MCP servers, hooks
/// that are not ours): the quota probes never start an agent in it on their own.
pub(crate) fn folder_runs_project_code(cwd: &str) -> bool {
    let root = Path::new(cwd);
    [
        ".mcp.json",
        ".agents/mcp.json",
        ".agents/mcp_config.json",
        ".grok/mcp.json",
        ".cursor/mcp.json",
    ]
    .iter()
    .any(|f| root.join(f).exists())
        || [
            ".agents/hooks.json",
            ".grok/hooks/ssh-app.json",
            ".claude/settings.json",
            ".claude/settings.local.json",
        ]
        .iter()
        .any(|f| crate::hooksview::hook_state(&root.join(f)) == "foreign")
}

/// A relocated state folder (`SSH_APP_HOME`) means a test or a sandbox: agy is never launched there.
pub(crate) fn probe_disabled() -> bool {
    std::env::var_os("SSH_APP_HOME").is_some()
}

fn cache_file() -> PathBuf {
    run_dir().join("agy-quota.json")
}

fn lock_file() -> PathBuf {
    run_dir().join("agy-quota.lock")
}

pub(crate) fn age(path: &Path) -> Option<f64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    SystemTime::now()
        .duration_since(m)
        .ok()
        .map(|d| d.as_secs_f64())
}

fn to_json(groups: &[Group], at: f64) -> Value {
    json!({
        "at": at,
        "groups": groups.iter().map(|g| json!({
            "models": g.models,
            "windows": g.windows.iter().map(|w| json!({
                "minutes": w.minutes, "remaining": w.remaining, "resets_epoch": w.resets_epoch,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn from_json(v: &Value) -> Vec<Group> {
    v.get("groups")
        .and_then(Value::as_array)
        .map(|gs| {
            gs.iter()
                .map(|g| Group {
                    models: g["models"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|m| m.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    windows: g["windows"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|w| {
                                    Some(Window {
                                        minutes: u32::try_from(w["minutes"].as_u64()?).ok()?,
                                        remaining: w["remaining"].as_f64()?,
                                        resets_epoch: w["resets_epoch"].as_i64(),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The cached groups (empty when there is no cache or it is a day old).
pub fn cached() -> Vec<Group> {
    let file = cache_file();
    if age(&file).is_none_or(|a| a > STALE_SECS) {
        return vec![];
    }
    std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|v| from_json(&v))
        .unwrap_or_default()
}

/// Starts a detached refresh when the cache is older than 15 minutes and nobody has just started one.
/// Called only while an agy tab exists, so agy is never launched for nothing.
pub fn refresh_if_stale(cwd: &str) {
    if probe_disabled()
        || age(&cache_file()).is_some_and(|a| a < FRESH_SECS)
        || age(&lock_file()).is_some_and(|a| a < LOCK_SECS)
        || !Path::new(cwd).is_absolute()
        || !Path::new(cwd).is_dir()
    {
        return;
    }
    let dir = run_dir();
    if std::fs::create_dir_all(&dir).is_err() || std::fs::write(lock_file(), b"").is_err() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = Command::new(exe)
        .args(["agy-quota-refresh", cwd])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub(crate) fn tmux(sock: &Path, args: &[&str]) -> String {
    Command::new("tmux")
        .arg("-S")
        .arg(sock)
        .args(args)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env(
            "PATH",
            format!(
                "/opt/homebrew/bin:/usr/local/bin:{}",
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

pub(crate) fn wait_for(sock: &Path, needle: &str, secs: u64) -> Option<String> {
    for _ in 0..secs * 2 {
        let screen = tmux(sock, &["capture-pane", "-p", "-t", "q"]);
        if screen.contains(needle) {
            return Some(screen);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    None
}

/// Runs agy on its own tmux server, opens `/usage`, reads the screen and writes the cache. The user's
/// tmux is never addressed: a private socket in a 0700 folder, `TMUX` removed from the environment.
pub fn refresh(cwd: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if probe_disabled() || !Path::new(cwd).is_absolute() || !Path::new(cwd).is_dir() {
        return false;
    }
    let dir = run_dir().join("agyq");
    if std::fs::create_dir_all(&dir).is_err()
        || std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).is_err()
    {
        return false;
    }
    let sock = dir.join("sock");
    let ok = (|| {
        tmux(
            &sock,
            &[
                "new-session",
                "-d",
                "-s",
                "q",
                "-x",
                "100",
                "-y",
                "60",
                "-c",
                cwd,
                "agy",
            ],
        );
        wait_for(&sock, "Gemini", 25)?;
        tmux(&sock, &["send-keys", "-t", "q", "/usage"]);
        std::thread::sleep(Duration::from_secs(2));
        tmux(&sock, &["send-keys", "-t", "q", "Enter"]);
        let screen = wait_for(&sock, "Models & Quota", 12)?;
        let at = now();
        let groups = parse(&screen, at);
        if groups.is_empty() {
            return None;
        }
        let file = cache_file();
        let tmp = file.with_extension("tmp");
        std::fs::write(&tmp, to_json(&groups, at).to_string()).ok()?;
        std::fs::rename(&tmp, &file).ok()
    })()
    .is_some();
    tmux(&sock, &["kill-server"]);
    let _ = std::fs::remove_file(&sock);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: &str = "└ Models & Quota\nGEMINI MODELS\n  Models within this group: Gemini Flash, Gemini Pro\n  Weekly Limit Remaining\n    [██████████] 99.68%\n    Refreshes in 120h 10m\n  Five Hour Limit Remaining\n    [██████████] 100.00%\n    Quota available\nCLAUDE AND GPT MODELS\n  Models within this group: Claude Opus, Claude Sonnet, GPT-OSS\n  Weekly Limit Remaining\n    [████░░░░░░] 40.00%\n    Refreshes in 10h 5m\n  Five Hour Limit Remaining\n    [██████████] 100.00%\n    Quota available\nGEMINI 4 ARGON\n  For paid Pro plans, Argon quota is separate.\n  (1–27 of 38 lines)\n";

    #[test]
    fn parses_groups_windows_and_resets() {
        let g = parse(SCREEN, 1000.0);
        assert_eq!(g.len(), 2, "a group with no windows is dropped");
        assert_eq!(g[0].models, ["gemini flash", "gemini pro"]);
        assert_eq!(g[0].windows[0].minutes, 10_080);
        assert_eq!(g[0].windows[0].remaining, 99.68);
        assert_eq!(g[0].windows[0].resets_epoch, Some(1000 + 120 * 3600 + 600));
        assert_eq!(g[0].windows[1].minutes, 300);
        assert_eq!(g[0].windows[1].resets_epoch, None);
    }

    #[test]
    fn picks_the_group_of_the_model_else_the_worst_window() {
        let g = parse(SCREEN, 1000.0);
        let flash = limits_for(&g, Some("gemini-3.8-flash-high"), 1000.0);
        assert_eq!(flash.iter().map(|l| l.0).collect::<Vec<_>>(), [300, 10_080]);
        assert!((flash[1].1 - 0.32).abs() < 1e-9);
        let claude = limits_for(&g, Some("Claude Opus 5.5 (High)"), 1000.0);
        assert!((claude[1].1 - 60.0).abs() < 1e-9);
        let any = limits_for(&g, None, 1000.0);
        assert!(
            (any[1].1 - 60.0).abs() < 1e-9,
            "no model: the tightest group"
        );
    }

    #[test]
    fn an_elapsed_reset_drops_the_window() {
        let g = parse(SCREEN, 1000.0);
        let later = 1000.0 + 200.0 * 3600.0;
        assert_eq!(limits_for(&g, Some("flash"), later).len(), 1);
    }

    #[test]
    fn hostile_text_yields_nothing() {
        assert!(parse("", 0.0).is_empty());
        assert!(parse("WEEKLY\nWeekly Limit Remaining\n999%\n", 0.0).is_empty());
        assert!(parse(&"A".repeat(1 << 20), 0.0).is_empty());
    }

    #[test]
    fn a_folder_with_mcp_or_foreign_hooks_is_not_probed() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().to_str().unwrap();
        assert!(!folder_runs_project_code(root));
        std::fs::write(d.path().join(".mcp.json"), "{}").unwrap();
        assert!(folder_runs_project_code(root));
        std::fs::remove_file(d.path().join(".mcp.json")).unwrap();
        std::fs::create_dir_all(d.path().join(".agents")).unwrap();
        std::fs::write(
            d.path().join(".agents/hooks.json"),
            r#"{"hooks":{"Stop":[{"command":"theirs"}]}}"#,
        )
        .unwrap();
        assert!(folder_runs_project_code(root));
        std::fs::write(
            d.path().join(".agents/hooks.json"),
            r#"{"hooks":{"Stop":[{"command":"ssh-app-hook agy Stop"}]}}"#,
        )
        .unwrap();
        assert!(!folder_runs_project_code(root), "our own hooks are fine");
    }

    #[test]
    fn json_round_trip() {
        let g = parse(SCREEN, 1000.0);
        assert_eq!(from_json(&to_json(&g, 1.0)), g);
    }
}
