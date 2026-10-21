//! Auto-approval by category (feature 50), port of the `auto_*` half of `ssh-app-hook.py`.
//! The app compiles its policy into `auto_rules` in guard.json: `[{agents?, root?, allow, deny,
//! edits}]`. An entry with a root applies to a session whose cwd is inside that folder and to a
//! call that stays inside it. Nothing here reads files of the repository.

use crate::guard::{
    first_str, guard_allows, guard_match, guard_rules, realpath, shell_segments,
    strip_safe_redirects, tool_kind,
};
use crate::queue::{clean, now, private_dir, read_small, Paths, ANTIGRAVITY_CMD_KEYS};
use serde_json::{json, Value};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

const MAX_ENTRIES: usize = 100;
const MAX_WORDS: usize = 200;
const PROTECTED: [&str; 6] = [
    ".git",
    ".claude",
    ".gemini",
    ".agents",
    ".ssh-app",
    ".mcp.json",
];
const PATH_INPUT_KEYS: [&str; 9] = [
    "file_path",
    "path",
    "AbsolutePath",
    "TargetFile",
    "notebook_path",
    "filePath",
    "absolute_path",
    "pattern",
    "glob",
];
pub const LOG_MAX: u64 = 200_000;

pub struct Entry {
    root: Option<String>,
    allow: Vec<String>,
    deny: Vec<String>,
    edits: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AutoDecision {
    Deny(String),
    /// Rule (or reason) and the project root, if any.
    Allow(String, Option<String>),
}

pub fn entries(guard: &Value, agent: &str) -> Vec<Entry> {
    let Some(raw) = guard.get("auto_rules").and_then(Value::as_array) else {
        return vec![];
    };
    let mut out = vec![];
    for e in raw.iter().take(MAX_ENTRIES) {
        if !e.is_object() {
            continue;
        }
        match e.get("agents") {
            None | Some(Value::Null) => {}
            Some(Value::Array(a)) if a.iter().any(|x| x.as_str() == Some(agent)) => {}
            _ => continue,
        }
        let root = match e.get("root") {
            None | Some(Value::Null) => None,
            Some(Value::String(r)) if r.starts_with('/') && r.chars().count() <= 400 => {
                Some(r.clone())
            }
            _ => continue,
        };
        out.push(Entry {
            root,
            allow: guard_rules(e, "allow"),
            deny: guard_rules(e, "deny"),
            edits: e.get("edits") == Some(&Value::Bool(true)),
        });
    }
    out
}

fn inside(root: &str, path: &str) -> bool {
    path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

fn expand(p: &str) -> String {
    if p == "~" {
        home()
    } else if let Some(rest) = p.strip_prefix("~/") {
        format!("{}/{rest}", home())
    } else {
        p.to_string()
    }
}

fn abs(cwd: &str, p: &str) -> String {
    if p.starts_with('/') {
        p.to_string()
    } else {
        format!("{}/{p}", cwd.trim_end_matches('/'))
    }
}

fn project_root(entries: &[Entry], cwd: &str) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    let here = realpath(cwd);
    let mut best: Option<String> = None;
    for e in entries {
        let Some(root) = &e.root else { continue };
        let r = realpath(root);
        if inside(&r, &here) && best.as_ref().is_none_or(|b| r.len() > b.len()) {
            best = Some(r);
        }
    }
    best
}

fn word_stays_inside(root: &str, cwd: &str, word: &str) -> bool {
    if word.contains('$') || word.contains('`') {
        return false;
    }
    let mut parts = vec![word];
    if let Some((_, after)) = word.split_once('=') {
        parts.push(after);
    }
    for part in parts {
        if part.is_empty() || (part.starts_with('-') && !word.contains('=')) {
            continue;
        }
        let looks_like_path = part.starts_with(['/', '~', '.']) || part.contains('/');
        let target = expand(part);
        if !looks_like_path && std::fs::symlink_metadata(abs(cwd, &target)).is_err() {
            continue;
        }
        if !inside(root, &realpath(&abs(cwd, &target))) {
            return false;
        }
    }
    true
}

fn bash_stays_inside(root: &str, cwd: &str, command: &str) -> bool {
    let mut words: Vec<String> = vec![];
    for seg in shell_segments(&strip_safe_redirects(command)) {
        match shlex::split(&seg) {
            Some(w) => words.extend(w),
            None => return false,
        }
        if words.len() > MAX_WORDS {
            return false;
        }
    }
    words.iter().all(|w| word_stays_inside(root, cwd, w))
}

fn tool_paths(ti: &Value) -> Vec<String> {
    let mut out = vec![];
    for key in PATH_INPUT_KEYS {
        if let Some(s) = ti
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            out.push(s.to_string());
        }
    }
    for key in ["paths", "file_paths"] {
        if let Some(a) = ti.get(key).and_then(Value::as_array) {
            out.extend(
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .take(50)
                    .map(str::to_string),
            );
        }
    }
    out
}

fn edit_target_ok(root: &str, cwd: &str, paths: &[String]) -> bool {
    if paths.is_empty() {
        return false;
    }
    for p in paths {
        if p.contains('$') {
            return false;
        }
        let r = realpath(&abs(cwd, &expand(p)));
        if !inside(root, &r) {
            return false;
        }
        let rel = r.strip_prefix(root.trim_end_matches('/')).unwrap_or("");
        if rel.split('/').any(|c| PROTECTED.contains(&c)) {
            return false;
        }
    }
    true
}

/// What the compiled auto rules say about a tool call: refuse, allow, or nothing.
pub fn auto_decision(
    guard: &Value,
    agent: &str,
    tool: &str,
    input: &Value,
    cwd: &str,
) -> Option<AutoDecision> {
    let entries = entries(guard, agent);
    if entries.is_empty() {
        return None;
    }
    for e in &entries {
        if let Some(hit) = guard_match(&e.deny, tool, input, cwd) {
            return Some(AutoDecision::Deny(hit));
        }
    }
    let kind = tool_kind(tool);
    let root = project_root(&entries, cwd);
    let (mut project_rules, mut free_rules, mut edits) = (vec![], vec![], false);
    for e in &entries {
        match &e.root {
            None => free_rules.extend(e.allow.iter().cloned()),
            Some(r) if root.as_deref() == Some(realpath(r).as_str()) => {
                project_rules.extend(e.allow.iter().cloned());
                edits |= e.edits;
            }
            _ => {}
        }
    }
    if kind == "Edit" {
        return match &root {
            Some(r) if edits && edit_target_ok(r, cwd, &tool_paths(input)) => Some(
                AutoDecision::Allow("project file edits".into(), Some(r.clone())),
            ),
            _ => None,
        };
    }
    if let (Some(r), false) = (&root, project_rules.is_empty()) {
        let mut all = project_rules;
        all.extend(free_rules.iter().cloned());
        let keys: Vec<&str> = std::iter::once("command")
            .chain(ANTIGRAVITY_CMD_KEYS[..4].iter().copied())
            .collect();
        let stays = if kind == "Bash" {
            let empty = serde_json::Map::new();
            bash_stays_inside(
                r,
                cwd,
                first_str(input.as_object().unwrap_or(&empty), &keys),
            )
        } else {
            tool_paths(input)
                .iter()
                .all(|p| word_stays_inside(r, cwd, p))
        };
        if let (Some(hit), true) = (guard_allows(&all, tool, input, cwd), stays) {
            return Some(AutoDecision::Allow(hit, Some(r.clone())));
        }
    }
    guard_allows(&free_rules, tool, input, cwd).map(|hit| AutoDecision::Allow(hit, root))
}

pub fn log_path(p: &Paths) -> std::path::PathBuf {
    p.run.join("auto.log")
}

/// One line per automatic decision, newest last, trimmed by size. Best effort.
pub fn log(p: &Paths, base: &crate::queue::Base, decision: &str, rule: &str, root: Option<&str>) {
    if !private_dir(&p.home) {
        return;
    }
    let _ = std::fs::DirBuilder::new().mode(0o700).create(&p.run);
    let path = log_path(p);
    let line = json!({
        "ts": now(), "agent": clean(base.agent, 40), "session": clean(base.session, 80),
        "tool": clean(base.tool, 60), "summary": clean(base.command, 200),
        "decision": decision, "rule": clean(rule, 120), "root": clean(root.unwrap_or(""), 300),
    })
    .to_string()
        + "\n";
    trim_log(&path);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

fn trim_log(path: &Path) {
    let Ok(m) = std::fs::symlink_metadata(path) else {
        return;
    };
    if m.len() <= LOG_MAX {
        return;
    }
    let Some(raw) = read_small(path, (LOG_MAX * 2) as usize) else {
        return;
    };
    let keep = &raw[raw.len().saturating_sub((LOG_MAX / 2) as usize)..];
    let start = keep.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        let _ = f.write_all(&keep[start..]);
    }
}

/// `ctl auto-log`: the last 50 decisions.
pub fn log_items(p: &Paths) -> Vec<Value> {
    let raw = read_small(&log_path(p), (LOG_MAX * 2) as usize).unwrap_or_default();
    let text = String::from_utf8_lossy(&raw);
    let lines: Vec<&str> = text.lines().collect();
    let items: Vec<Value> = lines[lines.len().saturating_sub(200)..]
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(Value::is_object)
        .collect();
    items[items.len().saturating_sub(50)..].to_vec()
}
