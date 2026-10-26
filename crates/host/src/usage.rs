//! Usage and limits of the agents running in tmux sessions (port of `usage_probe.py`, which stays
//! the reference and the fallback; `tests/parity.rs` compares the two on one fixture tree).
//!
//! Read-only: files are opened only when they are regular files, with size and time budgets.

use crate::timefmt::iso_epoch;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_DIRS: usize = 8;
const MAX_DIR_LEN: usize = 300;
const BUDGET_SECS: f64 = 15.0;
const TAIL_LIMIT: u64 = 1 << 20;
const MAX_STATE_FILE: u64 = 8192;
const MAX_LINE: usize = 8 << 20;

pub fn validate_dirs(args: &[String]) -> Result<Vec<String>, String> {
    if args.len() > MAX_DIRS {
        return Err("too many config directories".into());
    }
    for a in args {
        if a.trim().is_empty() || a.chars().count() > MAX_DIR_LEN || a.chars().any(char::is_control)
        {
            return Err("invalid config directory".into());
        }
    }
    Ok(args.to_vec())
}

pub(crate) fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

fn expand(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if p == "~" => home(),
        None => PathBuf::from(p),
    }
}

fn state_home() -> PathBuf {
    std::env::var_os("SSH_APP_HOME").map_or_else(|| home().join(".ssh-app"), PathBuf::from)
}

pub(crate) fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

// ---- running ps / tmux -----------------------------------------------------------------------

/// Runs a fixed argv (no shell) and returns stdout, or "" on any failure or after `secs`.
pub(crate) fn run(prog: &str, args: &[&str], secs: u64) -> String {
    let path = format!(
        "/opt/homebrew/bin:/usr/local/bin:{}",
        std::env::var("PATH").unwrap_or_default()
    );
    let Ok(mut child) = Command::new(prog)
        .args(args)
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return String::new();
    };
    let Some(mut out) = child.stdout.take() else {
        return String::new();
    };
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut out).take(16 << 20).read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned()
}

// ---- processes -----------------------------------------------------------------------------------

pub(crate) struct Procs {
    args: HashMap<u32, String>,
    children: HashMap<u32, Vec<u32>>,
}

/// Python's `line.strip().split(None, 2)`.
fn split3(line: &str) -> Option<(&str, &str, &str)> {
    let line = line.trim();
    let (a, rest) = line.split_once(char::is_whitespace)?;
    let rest = rest.trim_start();
    let (b, rest) = rest.split_once(char::is_whitespace)?;
    let rest = rest.trim_start();
    (!rest.is_empty()).then_some((a, b, rest))
}

pub(crate) fn read_procs() -> Procs {
    let mut procs = Procs {
        args: HashMap::new(),
        children: HashMap::new(),
    };
    for line in run("ps", &["-axo", "pid=,ppid=,args="], 8).lines() {
        let Some((pid, ppid, args)) = split3(line) else {
            continue;
        };
        let (Ok(pid), Ok(ppid)) = (pid.parse::<u32>(), ppid.parse::<u32>()) else {
            continue;
        };
        procs.args.insert(pid, args.to_string());
        procs.children.entry(ppid).or_default().push(pid);
    }
    procs
}

pub(crate) struct Detector {
    agents: Vec<(&'static str, Regex)>,
    ollama: Regex,
}

impl Detector {
    pub(crate) fn new() -> Self {
        let agent = |n: &'static str| (n, Regex::new(&format!(r"(^|/){n}(\s|$)")).expect("regex"));
        Self {
            agents: vec![agent("claude"), agent("codex"), agent("agy"), agent("grok")],
            ollama: Regex::new(r"(^|/)ollama\s+run\s+(\S+)").expect("regex"),
        }
    }

    /// Same walk as the Python: a stack, children pushed in order, first match wins.
    pub(crate) fn detect(
        &self,
        procs: &Procs,
        root: u32,
    ) -> Option<(&'static str, Option<String>)> {
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        while let Some(p) = stack.pop() {
            if !seen.insert(p) {
                continue;
            }
            let args = procs.args.get(&p).map_or("", String::as_str);
            if let Some(m) = self.ollama.captures(args) {
                return Some(("ollama", Some(m[2].to_string())));
            }
            for (name, rx) in &self.agents {
                if rx.is_match(args) {
                    return Some((name, None));
                }
            }
            if let Some(kids) = procs.children.get(&p) {
                stack.extend(kids.iter().copied());
            }
        }
        None
    }
}

// ---- file helpers -------------------------------------------------------------------------------

pub(crate) fn regular(path: &Path) -> Option<std::fs::Metadata> {
    std::fs::metadata(path).ok().filter(|m| m.is_file())
}

pub(crate) fn mtime(m: &std::fs::Metadata) -> f64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs_f64())
}

/// The last `limit` bytes as lines; the first (possibly cut) line is dropped when the file is longer.
fn tail_lines(path: &Path, limit: u64) -> Vec<String> {
    let Some(meta) = regular(path) else {
        return vec![];
    };
    let Ok(mut f) = std::fs::File::open(path) else {
        return vec![];
    };
    let size = meta.len();
    let start = size.saturating_sub(limit);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return vec![];
    }
    let mut data = Vec::new();
    if f.take(limit).read_to_end(&mut data).is_err() {
        return vec![];
    }
    let text = String::from_utf8_lossy(&data);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if size > limit && !lines.is_empty() {
        lines.remove(0);
    }
    lines
}

/// `*.jsonl` files under `root`, at most `max_depth` levels down (symlinked folders are followed,
/// the depth bound stops loops).
pub(crate) fn jsonl_files(root: &Path, max_depth: usize, deadline: Instant) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if Instant::now() > deadline {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(meta) = std::fs::metadata(&p) else {
                continue;
            };
            if meta.is_dir() {
                if depth < max_depth {
                    stack.push((p, depth + 1));
                }
            } else if meta.is_file() && p.extension().is_some_and(|x| x == "jsonl") {
                out.push(p);
            }
        }
    }
    out
}

fn as_u64(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)),
        _ => None,
    }
    .unwrap_or(0)
}

fn percent(v: f64) -> i64 {
    // Python's round(): half to even.
    let c = v.clamp(0.0, 100.0);
    let r = c.round();
    if (c - c.trunc() - 0.5).abs() < f64::EPSILON && (r as i64) % 2 != 0 {
        (r - 1.0) as i64
    } else {
        r as i64
    }
}

fn limit_entry(label: &str, pct: f64, reset: Option<i64>) -> Value {
    json!({"label": label, "percent": percent(pct), "resets_epoch": reset})
}

// ---- Claude Code ---------------------------------------------------------------------------------

fn slug(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub struct Probe {
    now: f64,
    dirs: Vec<PathBuf>,
    limit_rx: Regex,
    reset_rx: Regex,
    agy_rx: Regex,
}

impl Probe {
    pub fn new(dirs: Vec<PathBuf>, now: f64) -> Self {
        Self {
            now,
            dirs,
            limit_rx: Regex::new(r"(?i)(usage|rate) limit|limit reached|limit will reset")
                .expect("regex"),
            reset_rx: Regex::new(r"\|(\d{10})\b").expect("regex"),
            agy_rx: Regex::new(r"(?i)quota|capacity|rate.?limit|429|exhausted|overloaded")
                .expect("regex"),
        }
    }

    /// The file of one known session (the id the hooks noted for the tmux session), in any project
    /// folder: another session of the same agent in the same folder cannot take its place.
    fn transcript_of(&self, sid: &str) -> Option<(PathBuf, usize)> {
        for (i, d) in self.dirs.iter().enumerate() {
            let Ok(rd) = std::fs::read_dir(d.join("projects")) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path().join(format!("{sid}.jsonl"));
                if regular(&p).is_some() {
                    return Some((p, i));
                }
            }
        }
        None
    }

    fn newest_transcript(&self, cwd: &str) -> Option<(PathBuf, usize)> {
        let mut best: Option<(PathBuf, usize)> = None;
        let mut best_m = 0.0;
        for (i, d) in self.dirs.iter().enumerate() {
            let folder = d.join("projects").join(slug(cwd));
            let Ok(rd) = std::fs::read_dir(&folder) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_none_or(|x| x != "jsonl") {
                    continue;
                }
                let Some(meta) = regular(&p) else { continue };
                let m = mtime(&meta);
                if m > best_m {
                    best = Some((p, i));
                    best_m = m;
                }
            }
        }
        best
    }

    /// Plan limits recorded by the status-line wrapper for this Claude config directory.
    pub fn recorded_limits(&self, config_dir: &Path) -> Vec<Value> {
        let want = std::fs::canonicalize(config_dir).unwrap_or_else(|_| config_dir.to_path_buf());
        let want = want.to_string_lossy().to_string();
        let folder = state_home().join("run").join("limits");
        let Ok(rd) = std::fs::read_dir(&folder) else {
            return vec![];
        };
        let mut best: Option<Value> = None;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let Some(meta) = regular(&p) else { continue };
            if meta.len() > MAX_STATE_FILE || !owned_by_me(&meta) {
                continue;
            }
            let Some(d) = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            else {
                continue;
            };
            if d.get("dir").and_then(Value::as_str) != Some(want.as_str()) {
                continue;
            }
            let ts = |v: &Value| v.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
            if best.as_ref().is_none_or(|b| ts(&d) > ts(b)) {
                best = Some(d);
            }
        }
        let mut out = vec![];
        for (key, label) in [("five_hour", "5h"), ("seven_day", "7d")] {
            let Some(w) = best.as_ref().and_then(|b| b.get(key)) else {
                continue;
            };
            let Some(pct) = w.get("used_percentage").and_then(Value::as_f64) else {
                continue;
            };
            let reset = w.get("resets_at").and_then(Value::as_f64).map(|r| r as i64);
            if reset.is_some_and(|r| (r as f64) < self.now) {
                continue; // the window rolled over since this was recorded
            }
            out.push(limit_entry(label, pct, reset));
        }
        out
    }

    pub fn claude_state(&self, cwd: &str, sid: Option<&str>) -> Map<String, Value> {
        let mut out = Map::new();
        let Some((file, dir_index)) = sid
            .and_then(|s| self.transcript_of(s))
            .or_else(|| self.newest_transcript(cwd))
        else {
            return out;
        };
        out.insert(
            "limits".into(),
            Value::Array(self.recorded_limits(&self.dirs[dir_index])),
        );
        let mut have_ctx = false;
        let mut have_note = false;
        for line in tail_lines(&file, TAIL_LIMIT).iter().rev() {
            let Ok(e) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(msg) = e.get("message").filter(|m| m.is_object()) else {
                continue;
            };
            let truthy = |k: &str| match e.get(k) {
                Some(Value::Bool(b)) => *b,
                Some(Value::Null) | None => false,
                Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
                Some(Value::String(s)) => !s.is_empty(),
                Some(_) => true,
            };
            let is_assistant = e.get("type").and_then(Value::as_str) == Some("assistant");
            if !have_ctx
                && is_assistant
                && msg.get("usage").is_some_and(Value::is_object)
                && !truthy("isSidechain")
                && !truthy("isApiErrorMessage")
            {
                let u = &msg["usage"];
                let used: u64 = [
                    "input_tokens",
                    "cache_read_input_tokens",
                    "cache_creation_input_tokens",
                    "output_tokens",
                ]
                .iter()
                .map(|k| as_u64(u.get(k)))
                .sum();
                out.insert("context_used".into(), json!(used));
                out.insert(
                    "context_limit".into(),
                    json!(if used > 200_000 { 1_000_000 } else { 200_000 }),
                );
                out.insert(
                    "model".into(),
                    msg.get("model").cloned().unwrap_or(Value::Null),
                );
                have_ctx = true;
            }
            if !have_note && (truthy("isApiErrorMessage") || is_assistant) {
                let ts = e
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(iso_epoch);
                if ts.is_some_and(|t| t > 0.0 && self.now - t < 6.0 * 3600.0) {
                    let text = match msg.get("content") {
                        Some(Value::Array(items)) => items
                            .iter()
                            .filter(|c| c.is_object())
                            .map(|c| c.get("text").and_then(Value::as_str).unwrap_or(""))
                            .collect::<Vec<_>>()
                            .join(" "),
                        Some(Value::String(s)) => s.clone(),
                        _ => String::new(),
                    };
                    if !text.is_empty()
                        && self.limit_rx.is_match(&text)
                        && (truthy("isApiErrorMessage") || text.chars().count() < 400)
                    {
                        out.insert("limit_note".into(), json!(text.trim()));
                        if let Some(m) = self.reset_rx.captures(&text) {
                            if let Ok(n) = m[1].parse::<u64>() {
                                out.insert("limit_reset_epoch".into(), json!(n));
                            }
                        }
                        have_note = true;
                    }
                }
            }
            if have_ctx && have_note {
                break;
            }
        }
        out
    }

    // ---- Codex ----------------------------------------------------------------------------------

    pub fn codex_limits(&self) -> Vec<Value> {
        let home_dir =
            std::env::var_os("CODEX_HOME").map_or_else(|| home().join(".codex"), PathBuf::from);
        let mut best: Option<PathBuf> = None;
        let mut best_m = 0.0;
        for p in jsonl_files(
            &home_dir.join("sessions"),
            4,
            Instant::now() + Duration::from_secs(3),
        ) {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("rollout-") {
                continue;
            }
            let Some(meta) = regular(&p) else { continue };
            if mtime(&meta) > best_m {
                best_m = mtime(&meta);
                best = Some(p);
            }
        }
        let Some(file) = best else { return vec![] };
        self.codex_limits_in(&file)
    }

    /// The plan limits the rollout `file` last recorded (`token_count` events carry `rate_limits`).
    fn codex_limits_in(&self, file: &Path) -> Vec<Value> {
        for line in tail_lines(file, TAIL_LIMIT).iter().rev() {
            if !line.contains("rate_limits") {
                continue;
            }
            let Ok(e) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let rl = e
                .get("payload")
                .and_then(|p| p.get("rate_limits"))
                .filter(|r| r.is_object())
                .or_else(|| e.get("rate_limits").filter(|r| r.is_object()));
            let Some(rl) = rl else { continue };
            let at = e
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(iso_epoch)
                .unwrap_or(self.now);
            let mut out = vec![];
            for key in ["primary", "secondary"] {
                let Some(w) = rl.get(key).filter(|w| w.is_object()) else {
                    continue;
                };
                let Some(pct) = w.get("used_percent").and_then(Value::as_f64) else {
                    continue;
                };
                let reset = w
                    .get("resets_at")
                    .and_then(Value::as_f64)
                    .or_else(|| {
                        w.get("resets_in_seconds")
                            .and_then(Value::as_f64)
                            .map(|s| at + s)
                    })
                    .map(|r| r as i64);
                if reset.is_some_and(|r| (r as f64) < self.now) {
                    continue;
                }
                let label = limit_label(w.get("window_minutes").and_then(Value::as_f64));
                out.push(limit_entry(&label, pct, reset));
            }
            return out;
        }
        vec![]
    }

    /// Codex's own state for the tab: the model, the context in use and the plan limits, read from the
    /// rollout of this session (the id the hooks noted, else the newest rollout of the folder). The
    /// limits fall back to the newest rollout anywhere: they belong to the account.
    pub fn codex_state(&self, cwd: &str, sid: Option<&str>) -> Map<String, Value> {
        let root =
            std::env::var_os("CODEX_HOME").map_or_else(|| home().join(".codex"), PathBuf::from);
        self.codex_state_in(&root, cwd, sid)
    }

    fn codex_state_in(&self, root: &Path, cwd: &str, sid: Option<&str>) -> Map<String, Value> {
        let mut out = Map::new();
        let mut files: Vec<(f64, PathBuf)> = jsonl_files(
            &root.join("sessions"),
            4,
            Instant::now() + Duration::from_secs(3),
        )
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("rollout-"))
        })
        .filter_map(|p| regular(&p).map(|m| (mtime(&m), p)))
        .collect();
        files.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mine = match sid {
            Some(id) => files.iter().find(|(_, p)| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(&format!("-{id}.jsonl")))
            }),
            None => files
                .iter()
                .take(20)
                .find(|(_, p)| first_line_names_cwd(p, cwd)),
        };
        let mut limits = vec![];
        if let Some((_, file)) = mine {
            let (mut model, mut ctx) = (None, None);
            for line in tail_lines(file, TAIL_LIMIT).iter().rev() {
                let Ok(e) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                let p = e.get("payload");
                if model.is_none() && e.get("type").and_then(Value::as_str) == Some("turn_context")
                {
                    model = p
                        .and_then(|p| p.get("model"))
                        .and_then(Value::as_str)
                        .map(|m| m.chars().take(80).collect::<String>());
                }
                if ctx.is_none() {
                    let info = p.and_then(|p| p.get("info")).filter(|i| i.is_object());
                    if let Some(info) = info {
                        let used = as_u64(
                            info.get("last_token_usage")
                                .and_then(|u| u.get("total_tokens")),
                        );
                        let limit = as_u64(info.get("model_context_window"));
                        if used > 0 && limit > 0 {
                            ctx = Some((used, limit));
                        }
                    }
                }
                if model.is_some() && ctx.is_some() {
                    break;
                }
            }
            if let Some(m) = model {
                out.insert("model".into(), json!(m));
            }
            if let Some((used, limit)) = ctx {
                out.insert("context_used".into(), json!(used));
                out.insert("context_limit".into(), json!(limit));
            }
            limits = self.codex_limits_in(file);
        }
        if limits.is_empty() {
            limits = self.codex_limits();
        }
        out.insert("limits".into(), Value::Array(limits));
        out
    }

    // ---- Grok -----------------------------------------------------------------------------------

    /// Grok keeps `summary.json` (the model) and `signals.json` (context tokens used and the window) in
    /// `sessions/<folder with / as %2F>/<session id>/`.
    pub fn grok_state(&self, cwd: &str, sid: Option<&str>) -> Map<String, Value> {
        let root =
            std::env::var_os("GROK_HOME").map_or_else(|| home().join(".grok"), PathBuf::from);
        let mut out = Self::grok_state_in(&root.join("sessions"), cwd, sid);
        // The plan limit exists only in grok's `/usage` dialog: a cached reading, refreshed in the background.
        crate::grokquota::refresh_if_stale(cwd);
        let limits: Vec<Value> = crate::grokquota::cached(self.now)
            .into_iter()
            .map(|(minutes, used, reset)| {
                limit_entry(&limit_label(Some(f64::from(minutes))), used, reset)
            })
            .collect();
        if !limits.is_empty() {
            out.insert("limits".into(), Value::Array(limits));
        }
        out
    }

    fn grok_state_in(root: &Path, cwd: &str, sid: Option<&str>) -> Map<String, Value> {
        let mut out = Map::new();
        let dir = match sid {
            Some(id) => std::fs::read_dir(root).ok().and_then(|rd| {
                rd.flatten()
                    .map(|e| e.path().join(id))
                    .find(|p| p.join("summary.json").is_file())
            }),
            None => {
                let enc = cwd
                    .replace('%', "%25")
                    .replace('/', "%2F")
                    .replace(' ', "%20");
                std::fs::read_dir(root.join(enc)).ok().and_then(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter_map(|p| regular(&p.join("summary.json")).map(|m| (mtime(&m), p)))
                        .max_by(|a, b| a.0.total_cmp(&b.0))
                        .map(|(_, p)| p)
                })
            }
        };
        let Some(dir) = dir else { return out };
        let read = |name: &str| -> Option<Value> {
            let meta = regular(&dir.join(name))?;
            if meta.len() > 1 << 20 {
                return None;
            }
            serde_json::from_str(&std::fs::read_to_string(dir.join(name)).ok()?).ok()
        };
        if let Some(m) = read("summary.json")
            .and_then(|v| {
                v.get("current_model_id")
                    .and_then(Value::as_str)
                    .map(String::from)
            })
            .filter(|m| !m.is_empty())
        {
            out.insert(
                "model".into(),
                json!(m.chars().take(80).collect::<String>()),
            );
        }
        if let Some(sig) = read("signals.json") {
            let used = as_u64(sig.get("contextTokensUsed"));
            let limit = as_u64(sig.get("contextWindowTokens"));
            if used > 0 && limit > 0 {
                out.insert("context_used".into(), json!(used));
                out.insert("context_limit".into(), json!(limit));
            }
        }
        out
    }

    // ---- Antigravity ----------------------------------------------------------------------------

    pub fn agy_state(&self, cwd: &str, sid: Option<&str>) -> Map<String, Value> {
        let mut out = Map::new();
        let agy_home = home().join(".gemini").join("antigravity-cli");
        let Some(mapping) =
            std::fs::read_to_string(agy_home.join("cache").join("last_conversations.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            return out;
        };
        let real = std::fs::canonicalize(cwd)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| cwd.to_string());
        let conv = sid.or_else(|| {
            mapping
                .get(&real)
                .or_else(|| mapping.get(cwd))
                .and_then(Value::as_str)
        });
        let Some(conv) = conv.filter(|c| {
            (8..=64).contains(&c.len()) && c.chars().all(|x| x.is_ascii_hexdigit() || x == '-')
        }) else {
            return out;
        };
        let transcript = agy_home
            .join("brain")
            .join(conv)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");
        if let Some(model) = std::fs::read_to_string(agy_home.join("settings.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string))
            .filter(|m| !m.is_empty())
        {
            let model: String = model.chars().take(80).collect();
            // Window sizes as documented for agy (Flash 1M, Pro 2M); not read from the tool.
            let limit = if model.to_lowercase().contains("pro") {
                2_000_000
            } else {
                1_000_000
            };
            out.insert("model".into(), json!(model));
            out.insert("context_limit".into(), json!(limit));
        }
        // The plan limits exist only on agy's `/usage` screen: a cached reading, refreshed in the background.
        crate::agyquota::refresh_if_stale(cwd);
        let limits: Vec<Value> = crate::agyquota::limits_for(
            &crate::agyquota::cached(),
            out.get("model").and_then(Value::as_str),
            self.now,
        )
        .into_iter()
        .map(|(minutes, used, reset)| {
            limit_entry(&limit_label(Some(f64::from(minutes))), used, reset)
        })
        .collect();
        if !limits.is_empty() {
            out.insert("limits".into(), Value::Array(limits));
        }
        let (mut have_ctx, mut have_note) = (false, false);
        for line in tail_lines(&transcript, TAIL_LIMIT).iter().rev() {
            let Ok(e) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if !e.is_object() {
                continue;
            }
            if !have_ctx && e.get("type").and_then(Value::as_str) == Some("PLANNER_RESPONSE") {
                let used: u64 = ["input_tokens", "cache_read_tokens", "output_tokens"]
                    .iter()
                    .map(|k| as_u64(e.get(k)))
                    .sum();
                if used > 0 {
                    out.insert("context_used".into(), json!(used));
                    have_ctx = true;
                }
            }
            if !have_note && e.get("status").and_then(Value::as_str) == Some("ERROR") {
                let ts = e
                    .get("created_at")
                    .and_then(Value::as_str)
                    .and_then(iso_epoch);
                if let (Some(t), Some(text)) = (ts, e.get("content").and_then(Value::as_str)) {
                    let head: String = text.chars().take(2000).collect();
                    if t > 0.0 && self.now - t < 6.0 * 3600.0 && self.agy_rx.is_match(&head) {
                        let note: String = text.trim().chars().take(200).collect();
                        out.insert("limit_note".into(), json!(note));
                        have_note = true;
                    }
                }
            }
            if have_ctx && have_note {
                break;
            }
        }
        if !have_ctx {
            out.remove("context_limit");
        }
        out
    }

    // ---- rolling windows ------------------------------------------------------------------------

    pub fn window_totals(&self) -> ([u64; 5], [u64; 5], bool) {
        let (mut five, mut week) = ([0u64; 5], [0u64; 5]);
        let mut seen: HashSet<(Option<String>, Option<String>)> = HashSet::new();
        let cutoff = self.now - 7.0 * 86_400.0;
        let deadline = Instant::now() + Duration::from_secs_f64(BUDGET_SECS);
        for d in &self.dirs {
            for f in jsonl_files(&d.join("projects"), 8, deadline) {
                if Instant::now() > deadline {
                    return (five, week, true);
                }
                let Some(meta) = regular(&f) else { continue };
                if mtime(&meta) < cutoff {
                    continue;
                }
                let Ok(fh) = std::fs::File::open(&f) else {
                    continue;
                };
                let mut reader = std::io::BufReader::new(fh);
                let mut raw = Vec::new();
                loop {
                    raw.clear();
                    match read_line_capped(&mut reader, &mut raw, MAX_LINE) {
                        None => break,
                        Some(false) => continue, // longer than the cap: skipped
                        Some(true) => {}
                    }
                    let line = String::from_utf8_lossy(&raw);
                    if !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
                        continue;
                    }
                    let Ok(e) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    let Some(msg) = e.get("message").filter(|m| m.is_object()) else {
                        continue;
                    };
                    let Some(u) = msg.get("usage").filter(|u| u.is_object()) else {
                        continue;
                    };
                    let Some(ts) = e
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .and_then(iso_epoch)
                        .filter(|t| *t > 0.0 && *t >= cutoff)
                    else {
                        continue;
                    };
                    let key = (
                        msg.get("id").and_then(Value::as_str).map(str::to_string),
                        e.get("requestId")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    );
                    if key != (None, None) && !seen.insert(key) {
                        continue;
                    }
                    let vals = [
                        as_u64(u.get("input_tokens")),
                        as_u64(u.get("output_tokens")),
                        as_u64(u.get("cache_creation_input_tokens")),
                        as_u64(u.get("cache_read_input_tokens")),
                        1,
                    ];
                    for (i, v) in vals.iter().enumerate() {
                        week[i] += v;
                        if self.now - ts <= 5.0 * 3600.0 {
                            five[i] += v;
                        }
                    }
                }
            }
        }
        (five, week, false)
    }
}

/// Reads one line into `buf`. `None` at end of input, `Some(false)` when the line was longer than
/// `cap` bytes (it is consumed but not kept), `Some(true)` for a line that fits.
fn read_line_capped<R: std::io::BufRead>(r: &mut R, buf: &mut Vec<u8>, cap: usize) -> Option<bool> {
    let mut fits = true;
    let mut any = false;
    loop {
        let chunk = match r.fill_buf() {
            Ok(c) if !c.is_empty() => c,
            _ => return any.then_some(fits),
        };
        any = true;
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        if fits {
            if buf.len() + take > cap + 1 {
                fits = false;
                buf.clear();
            } else {
                buf.extend_from_slice(&chunk[..take]);
            }
        }
        r.consume(take);
        if done {
            return Some(fits);
        }
    }
}

fn limit_label(minutes: Option<f64>) -> String {
    match minutes {
        Some(m) if m > 0.0 && m.fract() == 0.0 => {
            let m = m as u64;
            if m.is_multiple_of(1440) {
                format!("{}d", m / 1440)
            } else if m.is_multiple_of(60) {
                format!("{}h", m / 60)
            } else {
                format!("{m}m")
            }
        }
        _ => "limit".into(),
    }
}

#[cfg(unix)]
fn owned_by_me(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    meta.uid() == unsafe { geteuid() }
}

#[cfg(not(unix))]
fn owned_by_me(_: &std::fs::Metadata) -> bool {
    true
}

// ---- Ollama -----------------------------------------------------------------------------------------

fn ollama_loaded() -> Vec<Value> {
    let Ok(addr) = "127.0.0.1:11434".parse() else {
        return vec![];
    };
    let Ok(mut s) = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)) else {
        return vec![];
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    if s.write_all(b"GET /api/ps HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return vec![];
    }
    let mut body = Vec::new();
    let _ = (&s).take(1 << 20).read_to_end(&mut body);
    let text = String::from_utf8_lossy(&body);
    let Some((_, json_part)) = text.split_once("\r\n\r\n") else {
        return vec![];
    };
    let Ok(data) = serde_json::from_str::<Value>(json_part) else {
        return vec![];
    };
    data.get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .map(|m| {
                    let name = m
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|n| !n.is_empty())
                        .or_else(|| m.get("model").and_then(Value::as_str))
                        .unwrap_or("");
                    let exp = m
                        .get("expires_at")
                        .and_then(Value::as_str)
                        .and_then(iso_epoch)
                        .map(|e| e as i64);
                    json!({
                        "name": name,
                        "size": as_u64(m.get("size")),
                        "size_vram": as_u64(m.get("size_vram")),
                        "context_length": m.get("context_length").cloned().unwrap_or(Value::Null),
                        "expires_epoch": exp,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---- the report -------------------------------------------------------------------------------------

/// The agent's own session id for a tmux session, as the hooks noted it (the newest record wins).
/// The first line of a rollout (`session_meta`) names the folder the session was started in.
fn first_line_names_cwd(path: &Path, cwd: &str) -> bool {
    let Ok(f) = std::fs::File::open(path) else {
        return false;
    };
    let mut line = String::new();
    if std::io::BufRead::read_line(&mut std::io::BufReader::new(f.take(65_536)), &mut line).is_err()
    {
        return false;
    }
    serde_json::from_str::<Value>(&line)
        .ok()
        .and_then(|v| {
            v.get("payload")
                .and_then(|p| p.get("cwd"))
                .and_then(Value::as_str)
                .map(|c| c == cwd)
        })
        .unwrap_or(false)
}

fn registered_id(registry: &[Value], tmux: &str, agent: &str) -> Option<String> {
    registry
        .iter()
        .filter(|r| r["tmux"] == tmux && r["agent"] == agent)
        .max_by_key(|r| r["ts"].as_i64().unwrap_or(0))
        .and_then(|r| r["session"].as_str())
        .filter(|id| {
            (1..=80).contains(&id.len())
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .map(String::from)
}

pub fn report(config_dirs: &[String]) -> Value {
    let dirs: Vec<PathBuf> = if config_dirs.is_empty() {
        vec![home().join(".claude")]
    } else {
        config_dirs.iter().map(|d| expand(d)).collect()
    };
    let probe = Probe::new(dirs, now());
    let registry = crate::sessions::list_sessions(&crate::queue::Paths::from_env());
    let procs = read_procs();
    let detector = Detector::new();
    let panes = run(
        "tmux",
        &[
            "list-panes",
            "-a",
            "-F",
            "#{session_name}\t#{pane_pid}\t#{pane_current_path}",
        ],
        8,
    );
    let mut sessions = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    for line in panes.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3
            || seen.contains(parts[0])
            || parts[1].is_empty()
            || !parts[1].bytes().all(|b| b.is_ascii_digit())
        {
            continue;
        }
        let Ok(pid) = parts[1].parse::<u32>() else {
            continue;
        };
        let Some((agent, arg)) = detector.detect(&procs, pid) else {
            continue;
        };
        seen.insert(parts[0].to_string());
        let mut s = Map::new();
        s.insert("session".into(), json!(parts[0]));
        s.insert("agent".into(), json!(agent));
        let cwd = parts[2];
        match agent {
            "claude" => s.extend(
                probe.claude_state(cwd, registered_id(&registry, parts[0], "claude").as_deref()),
            ),
            "codex" => s.extend(
                probe.codex_state(cwd, registered_id(&registry, parts[0], "codex").as_deref()),
            ),
            "grok" => s.extend(
                probe.grok_state(cwd, registered_id(&registry, parts[0], "grok").as_deref()),
            ),
            "agy" => s.extend(probe.agy_state(
                cwd,
                registered_id(&registry, parts[0], "antigravity").as_deref(),
            )),
            "ollama" => {
                if let Some(a) = arg.filter(|a| !a.is_empty()) {
                    s.insert("ollama_model".into(), json!(a));
                }
            }
            _ => {}
        }
        sessions.push(Value::Object(s));
    }
    let (five, week, partial) = probe.window_totals();
    let keys = ["input", "output", "cache_write", "cache_read", "messages"];
    let window = |w: [u64; 5]| -> Value {
        Value::Object(
            keys.iter()
                .zip(w)
                .map(|(k, v)| ((*k).to_string(), json!(v)))
                .collect(),
        )
    };
    json!({
        "sessions": sessions,
        "ollama": ollama_loaded(),
        "five_hour": window(five),
        "week": window(week),
        "partial": partial,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_are_validated() {
        assert!(validate_dirs(&["~/.claude".into()]).is_ok());
        assert!(validate_dirs(&["a\nb".into()]).is_err());
        assert!(validate_dirs(&["".into()]).is_err());
        assert!(validate_dirs(&vec!["x".to_string(); 9]).is_err());
        assert!(validate_dirs(&["y".repeat(301)]).is_err());
    }

    #[test]
    fn split3_matches_python_split_none_2() {
        assert_eq!(
            split3("  12  1   /bin/zsh -l  "),
            Some(("12", "1", "/bin/zsh -l"))
        );
        assert_eq!(split3("12 1"), None);
        assert_eq!(split3("12 1   "), None);
    }

    #[test]
    fn detection_walks_the_tree_like_the_python() {
        let mut procs = Procs {
            args: HashMap::new(),
            children: HashMap::new(),
        };
        procs.args.insert(1, "-zsh".into());
        procs.args.insert(2, "node /x/bin/claude --resume".into());
        procs.args.insert(3, "ollama run qwen2.5-coder:7b".into());
        procs.args.insert(4, "vim claudette.txt".into());
        procs.children.insert(1, vec![2]);
        procs.children.insert(2, vec![3]);
        let d = Detector::new();
        assert_eq!(d.detect(&procs, 1), Some(("claude", None)));
        assert_eq!(
            d.detect(&procs, 3),
            Some(("ollama", Some("qwen2.5-coder:7b".into())))
        );
        assert_eq!(d.detect(&procs, 4), None);
    }

    #[test]
    fn long_lines_are_skipped_not_loaded() {
        let mut data = b"short\n".to_vec();
        data.extend(std::iter::repeat_n(b'x', 5000));
        data.extend_from_slice(b"\nlast");
        let mut r = std::io::BufReader::with_capacity(16, &data[..]);
        let mut buf = Vec::new();
        assert_eq!(read_line_capped(&mut r, &mut buf, 100), Some(true));
        assert_eq!(buf, b"short\n");
        buf.clear();
        assert_eq!(read_line_capped(&mut r, &mut buf, 100), Some(false));
        assert!(buf.is_empty());
        assert_eq!(read_line_capped(&mut r, &mut buf, 100), Some(true));
        assert_eq!(buf, b"last");
        buf.clear();
        assert_eq!(read_line_capped(&mut r, &mut buf, 100), None);
    }

    #[test]
    fn labels_and_rounding() {
        assert_eq!(limit_label(Some(300.0)), "5h");
        assert_eq!(limit_label(Some(10080.0)), "7d");
        assert_eq!(limit_label(Some(45.0)), "45m");
        assert_eq!(limit_label(None), "limit");
        assert_eq!(percent(12.5), 12);
        assert_eq!(percent(13.5), 14);
        assert_eq!(percent(250.0), 100);
        assert_eq!(percent(-3.0), 0);
    }

    #[test]
    fn registered_id_takes_the_newest_valid_record_of_that_tmux_and_agent() {
        let reg = vec![
            json!({"agent":"claude","session":"old","tmux":"t","ts":1}),
            json!({"agent":"claude","session":"new","tmux":"t","ts":9}),
            json!({"agent":"claude","session":"other","tmux":"u","ts":99}),
            json!({"agent":"antigravity","session":"agy","tmux":"t","ts":99}),
        ];
        assert_eq!(registered_id(&reg, "t", "claude").as_deref(), Some("new"));
        assert_eq!(
            registered_id(&reg, "t", "antigravity").as_deref(),
            Some("agy")
        );
        assert_eq!(registered_id(&reg, "none", "claude"), None);
        let bad = vec![json!({"agent":"claude","session":"../x","tmux":"t","ts":1})];
        assert_eq!(registered_id(&bad, "t", "claude"), None);
    }

    #[test]
    fn a_known_session_id_beats_the_newest_file_of_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects/-w");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("mine.jsonl"), "{}\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(proj.join("laptop.jsonl"), "{}\n").unwrap();
        let probe = Probe::new(vec![dir.path().to_path_buf()], 0.0);
        assert_eq!(
            probe.newest_transcript("/w").unwrap().0,
            proj.join("laptop.jsonl")
        );
        assert_eq!(
            probe.transcript_of("mine").unwrap().0,
            proj.join("mine.jsonl")
        );
        assert!(probe.transcript_of("absent").is_none());
    }

    #[test]
    fn slug_matches_claude_folder_names() {
        assert_eq!(slug("/Users/ivs/rust/ssh-app"), "-Users-ivs-rust-ssh-app");
    }

    #[test]
    fn codex_state_reads_the_model_context_and_limits_of_its_own_rollout() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/10/26");
        std::fs::create_dir_all(&day).unwrap();
        let mine = day.join("rollout-2026-10-26T10-00-00-aaa111.jsonl");
        let other = day.join("rollout-2026-10-26T11-00-00-bbb222.jsonl");
        let body = |cwd: &str, model: &str, used: u64| {
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{cwd}\"}}}}\n\
                 {{\"type\":\"turn_context\",\"payload\":{{\"model\":\"{model}\"}}}}\n\
                 {{\"type\":\"event_msg\",\"timestamp\":\"2026-10-26T10:00:00Z\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"model_context_window\":258400,\"last_token_usage\":{{\"total_tokens\":{used}}}}},\"rate_limits\":{{\"primary\":{{\"used_percent\":12.0,\"window_minutes\":300,\"resets_at\":4102444800}}}}}}}}\n"
            )
        };
        std::fs::write(&mine, body("/w/mine", "gpt-6-luna", 17770)).unwrap();
        std::fs::write(&other, body("/w/other", "other-model", 5)).unwrap();
        let probe = Probe::new(vec![], 1_800_000_000.0);
        let st = probe.codex_state_in(dir.path(), "/w/mine", None);
        assert_eq!(st["model"], "gpt-6-luna");
        assert_eq!(st["context_used"], 17770);
        assert_eq!(st["context_limit"], 258400);
        assert_eq!(st["limits"][0]["label"], "5h");
        // a noted id beats the folder
        let st = probe.codex_state_in(dir.path(), "/w/mine", Some("bbb222"));
        assert_eq!(st["model"], "other-model");
        // a folder with no rollout: nothing of its own
        let st = probe.codex_state_in(dir.path(), "/w/none", None);
        assert!(st.get("model").is_none() && st.get("context_used").is_none());
    }

    #[test]
    fn grok_state_reads_the_model_and_context_of_its_session() {
        let dir = tempfile::tempdir().unwrap();
        let s = dir.path().join("%2Fw%2Fmy%20dir/sess-1");
        std::fs::create_dir_all(&s).unwrap();
        std::fs::write(s.join("summary.json"), r#"{"current_model_id":"grok-4.7"}"#).unwrap();
        std::fs::write(
            s.join("signals.json"),
            r#"{"contextTokensUsed":36366,"contextWindowTokens":256000}"#,
        )
        .unwrap();
        let st = Probe::grok_state_in(dir.path(), "/w/my dir", None);
        assert_eq!(st["model"], "grok-4.7");
        assert_eq!(st["context_used"], 36366);
        assert_eq!(st["context_limit"], 256000);
        assert_eq!(
            Probe::grok_state_in(dir.path(), "/x", Some("sess-1"))["model"],
            "grok-4.7"
        );
        assert!(Probe::grok_state_in(dir.path(), "/w/none", None).is_empty());
        // a path-like id finds nothing
        assert!(Probe::grok_state_in(dir.path(), "/x", Some("../x")).is_empty());
    }
}
