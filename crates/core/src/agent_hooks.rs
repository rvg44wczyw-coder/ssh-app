//! Installation of the ssh-app approval hooks into the agents' own config files (Claude Code,
//! Antigravity, Codex, Grok): presets, the planned entries, idempotent merge and removal.
//! Launch profiles live in `agents`.

use crate::agents::{has_control, invalid, remote_path_expr, AgentKind};
use crate::error::SshCoreError;
use crate::tmux::shell_quote;
use serde_json::{json, Map, Value};

mod opencode;
mod statusline;

use opencode::*;
pub use statusline::*;

pub const HOOK_MARKER: &str = "ssh-app-hook";
pub const DEFAULT_HOOK_DIR: &str = "~/.ssh-app/bin";
const HOOK_TIMEOUT_SECS: u64 = 130;
const ANTIGRAVITY_HOOK_KEY: &str = "ssh-app";
/// Only shell commands go to the phone by default: `PreToolUse` fires for every tool call (unlike
/// Claude's `PermissionRequest`), so file tools would cause approval fatigue.
const ANTIGRAVITY_TOOL_MATCHER: &str = "run_command";
const GROK_TOOL_MATCHER: &str = "Bash";
const CRUSH_TOOL_MATCHER: &str = "^bash$";
const OPENCODE_TOOL_MATCHER: &str = "bash";

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentHooksStatus {
    pub config_path: String,
    pub hooks_installed: bool,
    pub script_installed: bool,
    /// The ssh-app entries found in the config, to compare with the plan (`hooks_drift`).
    pub installed: Vec<PlannedHook>,
}

// ---------------------------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------------------------

#[uniffi::export]
pub fn agent_hook_script() -> String {
    let script = include_str!("../../../scripts/agent-hooks/ssh-app-hook.py");
    // The pin of the embedded typed helper (feature 25): the script hands over to it only when
    // the helper next to it has exactly this digest.
    match crate::usage::bundled_host_helper_sha256() {
        Some(pin) => script.replacen(
            "HELPER_SHA256 = \"\"",
            &format!("HELPER_SHA256 = \"{pin}\""),
            1,
        ),
        None => script.to_string(),
    }
}

/// What to do with the hook script already on a host after the helper changed (its pin is inside
/// the script, so a stale pin silently sends every hook to the Python code).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HookScriptRefresh {
    /// No script on the host: nothing to keep in step.
    NotInstalled,
    /// Already the script this build ships, pin included.
    UpToDate,
    /// It was ssh-app's own script and was replaced.
    Refreshed,
    /// A file is there that is not ssh-app's script: only replaced when the user says so.
    NeedsConfirmation,
}

/// Whether `existing` is a script of ours (any version): the docstring and the pin slot are the
/// signature. Anything else at that path is somebody's own file.
pub(crate) fn is_our_hook_script(existing: &str) -> bool {
    let head: String = existing.chars().take(600).collect();
    head.contains("ssh-app agent hook") && existing.contains("HELPER_SHA256 = ")
}

/// The decision, without touching the host: `Refreshed` here means "write the new script".
pub(crate) fn plan_hook_script_refresh(
    existing: &str,
    new: &str,
    overwrite_foreign: bool,
) -> HookScriptRefresh {
    if existing.trim().is_empty() {
        HookScriptRefresh::NotInstalled
    } else if existing == new {
        HookScriptRefresh::UpToDate
    } else if is_our_hook_script(existing) || overwrite_foreign {
        HookScriptRefresh::Refreshed
    } else {
        HookScriptRefresh::NeedsConfirmation
    }
}

/// The hook dir is embedded in a command the *agent's shell* executes (inside double quotes), so
/// it must not contain anything the shell would interpret (`$`, backticks, quotes, `\`, ...).
fn validate_hook_dir(dir: &str) -> Result<(), SshCoreError> {
    validate_dir(dir)?;
    if !dir
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '~' | ' '))
    {
        return Err(invalid(
            "Hook directory may only contain letters, digits, space and . _ - / ~",
        ));
    }
    Ok(())
}

fn validate_dir(dir: &str) -> Result<(), SshCoreError> {
    if dir.trim().is_empty() || has_control(dir) {
        return Err(invalid("Invalid directory"));
    }
    Ok(())
}

/// Remote path (with a leading `~/`) of the config file the hooks live in.
#[uniffi::export]
pub fn agent_hooks_config_path(
    agent: AgentKind,
    config_dir: Option<String>,
) -> Result<String, SshCoreError> {
    let (default_dir, file) = match agent {
        AgentKind::ClaudeCode => ("~/.claude", "settings.json"),
        AgentKind::Antigravity => ("~/.gemini/config", "hooks.json"),
        // Codex trusts hooks by hash (the user reviews them with /hooks); Grok reads every
        // `*.json` in its hooks directory, so the installer owns one file there.
        AgentKind::Codex => ("~/.codex", "hooks.json"),
        AgentKind::Grok => ("~/.grok/hooks", "ssh-app.json"),
        // Qwen Code reads Claude-style hooks from `settings.json`; Crush from `crush.json`;
        // OpenCode loads every file in its plugins directory (the installer owns one). The Qwen
        // and Crush user paths are not stated in the docs consulted: the preview shows the target.
        AgentKind::QwenCode => ("~/.qwen", "settings.json"),
        AgentKind::Crush => ("~/.config/crush", "crush.json"),
        AgentKind::OpenCode => ("~/.config/opencode/plugins", "ssh-app.js"),
        other => return Err(no_hooks(other)),
    };
    let dir = match config_dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        Some(d) => {
            validate_dir(d)?;
            d.trim_end_matches('/').to_string()
        }
        None => default_dir.to_string(),
    };
    Ok(format!("{dir}/{file}"))
}

fn no_hooks(agent: AgentKind) -> SshCoreError {
    invalid(format!(
        "{agent:?} has no hook mechanism ssh-app can use; it asks in its own terminal"
    ))
}

fn hook_command(hook_dir: &str, agent: &str, event: &str) -> String {
    let dir = hook_dir.trim_end_matches('/');
    let path = match dir.strip_prefix("~/") {
        Some(rest) => format!("$HOME/{rest}/{HOOK_MARKER}"),
        None => format!("{dir}/{HOOK_MARKER}"),
    };
    format!("python3 -I \"{path}\" {agent} {event}")
}

/// Exactly the commands we write: the script invocation (`python3 -I ".../ssh-app-hook" <agent>
/// <Event>`) or a custom command ending in our marker comment. A substring match would also
/// delete a user's unrelated hook that merely mentions the marker in a path.
fn is_our_command(command: &str) -> bool {
    command.ends_with(&format!(" # {HOOK_MARKER}-custom"))
        || (command.starts_with("python3 -I \"") && command.contains(&format!("/{HOOK_MARKER}\" ")))
}

fn is_our_handler(h: &Value) -> bool {
    h.get("command")
        .and_then(Value::as_str)
        .map(is_our_command)
        .unwrap_or(false)
}

/// Key that holds `{event: [entries]}` in the agent's config.
fn container_key(agent: AgentKind) -> Result<&'static str, SshCoreError> {
    match agent {
        AgentKind::ClaudeCode
        | AgentKind::Codex
        | AgentKind::Grok
        | AgentKind::QwenCode
        | AgentKind::Crush => Ok("hooks"),
        AgentKind::Antigravity => Ok(ANTIGRAVITY_HOOK_KEY),
        // OpenCode's hooks are a JS plugin, not a JSON container (see `opencode_*`).
        other => Err(no_hooks(other)),
    }
}

/// Best-effort scan for an object that repeats a key (a rewrite would silently keep only the
/// last one). Runs on text; escapes inside keys are compared in their raw form.
fn find_duplicate_key(text: &str) -> Option<String> {
    use std::collections::HashSet;
    // One frame per open container: Some(keys seen) for objects, None for arrays.
    let mut stack: Vec<Option<HashSet<String>>> = vec![];
    let mut expect_key: Vec<bool> = vec![];
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                stack.push(Some(HashSet::new()));
                expect_key.push(true);
            }
            '[' => {
                stack.push(None);
                expect_key.push(false);
            }
            '}' | ']' => {
                stack.pop();
                expect_key.pop();
            }
            ',' => {
                if let (Some(Some(_)), Some(e)) = (stack.last(), expect_key.last_mut()) {
                    *e = true;
                }
            }
            ':' => {
                if let Some(e) = expect_key.last_mut() {
                    *e = false;
                }
            }
            '"' => {
                let mut raw = String::new();
                while let Some(ch) = chars.next() {
                    match ch {
                        '\\' => {
                            raw.push(ch);
                            if let Some(n) = chars.next() {
                                raw.push(n);
                            }
                        }
                        '"' => break,
                        _ => raw.push(ch),
                    }
                }
                if let (Some(Some(keys)), Some(true)) = (stack.last_mut(), expect_key.last()) {
                    if !keys.insert(raw.clone()) {
                        return Some(raw);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn parse_object(existing: &str) -> Result<Map<String, Value>, SshCoreError> {
    if existing.trim().is_empty() {
        return Ok(Map::new());
    }
    if let Some(key) = find_duplicate_key(existing) {
        return Err(invalid(format!(
            "Config file has the duplicate key \"{key}\"; refusing to modify it"
        )));
    }
    match serde_json::from_str::<Value>(existing) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(invalid(
            "Config file is not a JSON object; refusing to modify it",
        )),
        Err(e) => Err(invalid(format!(
            "Config file is not valid JSON ({e}); refusing to modify it"
        ))),
    }
}

pub(crate) fn render(map: Map<String, Value>) -> Result<String, SshCoreError> {
    serde_json::to_string_pretty(&Value::Object(map))
        .map(|s| s + "\n")
        .map_err(|e| invalid(format!("Failed to serialize config: {e}")))
}

fn handler(command: String) -> Value {
    json!({ "type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECS })
}

/// Removes only our handlers from `<key>.<event>` (keeping everyone else's, including siblings in
/// the same entry) and drops entries/events/containers that become empty.
fn strip_container(map: &mut Map<String, Value>, key: &str) {
    if let Some(Value::Object(events)) = map.get_mut(key) {
        for (_, list) in events.iter_mut() {
            if let Value::Array(items) = list {
                // Antigravity's flat events (Stop, ...) hold handler objects directly.
                items.retain(|item| !(item.get("hooks").is_none() && is_our_handler(item)));
                // Only entries that *became* empty because we took our handler out are dropped.
                let emptied: Vec<bool> = items
                    .iter_mut()
                    .map(|item| match item.get_mut("hooks") {
                        Some(Value::Array(hs)) => {
                            let before = hs.len();
                            hs.retain(|h| !is_our_handler(h));
                            before > 0 && hs.is_empty()
                        }
                        _ => false,
                    })
                    .collect();
                let mut it = emptied.into_iter();
                items.retain(|_| !it.next().unwrap_or(false));
            }
        }
        events.retain(|_, v| !matches!(v, Value::Array(a) if a.is_empty()));
    }
    if matches!(map.get(key), Some(Value::Object(h)) if h.is_empty()) {
        map.shift_remove(key);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HookPresetKind {
    /// Ask the phone to approve tool use (falls back to the agent's own prompt), plus notifications.
    PhoneApproval,
    /// Only Notification/Stop events are forwarded; nothing can be approved or blocked.
    NotifyOnly,
    /// The user's own command for the chosen events; the ssh-app hook script is not used.
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HookPreset {
    pub kind: HookPresetKind,
    /// `Custom` only: shell command the agent runs for each event (it receives the event JSON on stdin).
    pub custom_command: String,
    /// `Custom` only: agent events to attach it to (see `hook_events_for`).
    pub custom_events: Vec<String>,
    /// `Custom` only: tool matcher for tool events; empty = the agent's default.
    pub custom_matcher: String,
    /// `PhoneApproval` / `NotifyOnly`: tools whose requests reach the phone; empty = the agent's default.
    pub approval_matcher: String,
    /// `PhoneApproval` / `NotifyOnly`: events to install, a subset of the preset's own; empty = all of them.
    pub approval_events: Vec<String>,
}

/// One hook entry that will be written into the agent's config; shown to the user before install.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlannedHook {
    pub event: String,
    pub matcher: Option<String>,
    pub command: String,
}

#[uniffi::export]
pub fn default_hook_preset() -> HookPreset {
    HookPreset {
        kind: HookPresetKind::PhoneApproval,
        custom_command: String::new(),
        custom_events: vec![],
        custom_matcher: String::new(),
        approval_matcher: String::new(),
        approval_events: vec![],
    }
}

/// Events a `Custom` preset may attach to, per agent.
#[uniffi::export]
pub fn hook_events_for(agent: AgentKind) -> Vec<String> {
    let events: &[&str] = match agent {
        AgentKind::ClaudeCode => &[
            "PermissionRequest",
            "PreToolUse",
            "PostToolUse",
            "UserPromptSubmit",
            "Notification",
            "SessionStart",
            "SessionEnd",
            "SubagentStop",
            "PreCompact",
            "Stop",
        ],
        AgentKind::Antigravity => &["PreToolUse", "Stop"],
        AgentKind::Codex => &["PermissionRequest", "PreToolUse", "Stop"],
        AgentKind::Grok => &["PreToolUse", "Notification", "Stop"],
        AgentKind::QwenCode => &["PermissionRequest", "PreToolUse", "Notification", "Stop"],
        AgentKind::Crush => &["PreToolUse"],
        AgentKind::OpenCode => &["PreToolUse", "Stop"],
        AgentKind::Cline | AgentKind::Aider | AgentKind::Goose | AgentKind::OllamaChat => &[],
    };
    events.iter().map(|e| (*e).to_string()).collect()
}

/// Whether the preset runs the ssh-app hook script (so it must be installed on the host).
#[uniffi::export]
pub fn hook_preset_uses_script(preset: HookPreset) -> bool {
    preset.kind != HookPresetKind::Custom
}

fn event_takes_matcher(event: &str) -> bool {
    matches!(event, "PermissionRequest" | "PreToolUse" | "PostToolUse")
}

/// Best-effort check that appending ` # marker` really appends a comment: no trailing backslash,
/// no dangling operator, balanced quotes and parentheses.
fn shell_syntax_problem(cmd: &str) -> Option<&'static str> {
    let (mut single, mut double, mut depth) = (false, false, 0i32);
    let mut chars = cmd.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' if !single => {
                if chars.next().is_none() {
                    return Some("trailing backslash");
                }
            }
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double => depth += 1,
            ')' if !single && !double => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return Some("unbalanced parenthesis");
        }
    }
    if single || double {
        return Some("unterminated quote");
    }
    if depth != 0 {
        return Some("unbalanced parenthesis");
    }
    let t = cmd.trim_end();
    if t.ends_with('|') || t.ends_with("&&") {
        return Some("ends with a pipe or &&");
    }
    None
}

fn validate_custom_hook(agent: AgentKind, preset: &HookPreset) -> Result<(), SshCoreError> {
    let cmd = preset.custom_command.trim();
    if cmd.is_empty() || cmd.len() > 1000 || has_control(cmd) {
        return Err(invalid(
            "Custom hook command must be 1-1000 characters on a single line",
        ));
    }
    if let Some(problem) = shell_syntax_problem(cmd) {
        return Err(invalid(format!(
            "Custom hook command looks incomplete ({problem}); the marker comment appended to it would not be a comment"
        )));
    }
    if preset.custom_events.is_empty() {
        return Err(invalid("Choose at least one event for the custom hook"));
    }
    let allowed = hook_events_for(agent);
    if let Some(bad) = preset.custom_events.iter().find(|e| !allowed.contains(e)) {
        return Err(invalid(format!("{agent:?} has no hook event '{bad}'")));
    }
    let m = preset.custom_matcher.trim();
    if !m
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '*' | '|' | '.' | ':' | '-'))
    {
        return Err(invalid(
            "Matcher may only contain letters, digits and _ * | . : -",
        ));
    }
    Ok(())
}

/// The user's matcher in the form the agent reads it. Crush matches tool names with a regex, so a
/// plain name list `a|b` becomes `^(a|b)$` (and `*` is "every tool"); OpenCode's plugin does the
/// same with the unanchored text. Other agents take the text as is.
fn agent_matcher(agent: AgentKind, m: &str) -> Result<String, SshCoreError> {
    if !matches!(agent, AgentKind::Crush | AgentKind::OpenCode) {
        return Ok(m.to_string());
    }
    if m == "*" {
        return Ok(if agent == AgentKind::Crush { ".*" } else { "*" }.to_string());
    }
    if !m
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '|' | '-'))
    {
        return Err(invalid(
            "For this agent the matcher is a list of tool names separated by | (or *)",
        ));
    }
    Ok(if agent == AgentKind::Crush {
        format!("^({m})$")
    } else {
        m.to_string()
    })
}

/// `None` = the agent's default matcher.
fn validate_approval_matcher(raw: &str) -> Result<Option<String>, SshCoreError> {
    let m = raw.trim();
    if m.is_empty() {
        return Ok(None);
    }
    if m.len() > 200
        || !m
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '*' | '|' | '.' | ':' | '-'))
    {
        return Err(invalid(
            "Approval matcher must be up to 200 characters of letters, digits and _ * | . : -",
        ));
    }
    Ok(Some(m.to_string()))
}

/// Chosen events in the preset's own order, deduplicated; empty input = all defaults.
fn validate_approval_events(
    chosen: &[String],
    defaults: &[&str],
    optional: &[&str],
) -> Result<Vec<String>, SshCoreError> {
    if chosen.is_empty() {
        return Ok(defaults.iter().map(|e| (*e).to_string()).collect());
    }
    if let Some(bad) = chosen
        .iter()
        .find(|e| !defaults.contains(&e.as_str()) && !optional.contains(&e.as_str()))
    {
        return Err(invalid(format!(
            "Event '{}' is not part of this preset (choose from {})",
            crate::approval_queue::display_safe(&bad.chars().take(40).collect::<String>()),
            defaults.join(", ")
        )));
    }
    Ok(defaults
        .iter()
        .chain(optional.iter())
        .filter(|d| chosen.iter().any(|c| c == *d))
        .map(|e| (*e).to_string())
        .collect())
}

/// Resolves a preset to the exact entries that will be written. Pure; used for the preview too.
#[uniffi::export]
pub fn plan_agent_hooks(
    agent: AgentKind,
    preset: HookPreset,
    hook_dir: Option<String>,
) -> Result<Vec<PlannedHook>, SshCoreError> {
    let dir = hook_dir.unwrap_or_else(|| DEFAULT_HOOK_DIR.to_string());
    validate_hook_dir(&dir)?;
    let (name, default_matcher) = match agent {
        AgentKind::ClaudeCode => ("claude", "*"),
        AgentKind::Antigravity => ("antigravity", ANTIGRAVITY_TOOL_MATCHER),
        AgentKind::Codex => ("codex", "*"),
        // Grok has no approval prompt a hook can answer, only PreToolUse deny; shell commands
        // are the sensible default so file reads and edits do not flood the phone.
        AgentKind::Grok => ("grok", GROK_TOOL_MATCHER),
        AgentKind::QwenCode => ("qwen", "*"),
        // Crush matches the tool name as a regex; OpenCode's plugin builds one from this too.
        AgentKind::Crush => ("crush", CRUSH_TOOL_MATCHER),
        AgentKind::OpenCode => ("opencode", OPENCODE_TOOL_MATCHER),
        other => return Err(no_hooks(other)),
    };
    let planned = |event: &str, matcher: Option<&str>, command: String| PlannedHook {
        event: event.to_string(),
        matcher: matcher.map(str::to_string),
        command,
    };
    Ok(match preset.kind {
        HookPresetKind::Custom if matches!(agent, AgentKind::Crush | AgentKind::OpenCode) => {
            return Err(invalid(
                "Custom hook commands are not supported for this agent; use the phone approval preset",
            ))
        }
        HookPresetKind::Custom => {
            validate_custom_hook(agent, &preset)?;
            // The trailing comment is how uninstall recognises (only) our entries.
            let command = format!("{} # {HOOK_MARKER}-custom", preset.custom_command.trim());
            let m = preset.custom_matcher.trim();
            let mut seen: Vec<&String> = vec![];
            preset
                .custom_events
                .iter()
                .filter(|e| {
                    let new = !seen.contains(e);
                    seen.push(e);
                    new
                })
                .map(|e| {
                    let matcher = event_takes_matcher(e).then_some(if m.is_empty() {
                        default_matcher
                    } else {
                        m
                    });
                    planned(e, matcher, command.clone())
                })
                .collect()
        }
        kind => {
            let notify_only = kind == HookPresetKind::NotifyOnly;
            let defaults: &[&str] = match (agent, notify_only) {
                (AgentKind::ClaudeCode, false) => &["PermissionRequest", "Notification", "Stop"],
                (AgentKind::ClaudeCode, true) => &["Notification", "Stop"],
                (AgentKind::Codex, false) => &["PermissionRequest", "Stop"],
                (AgentKind::Codex, true) => &["Stop"],
                (AgentKind::Grok, false) => &["PreToolUse", "Notification", "Stop"],
                (AgentKind::Grok, true) => &["Notification", "Stop"],
                (AgentKind::QwenCode, false) => &["PermissionRequest", "Notification", "Stop"],
                (AgentKind::QwenCode, true) => &["Notification", "Stop"],
                (AgentKind::Crush, false) => &["PreToolUse"],
                (AgentKind::Crush, true) => {
                    return Err(invalid(
                        "Crush only has a PreToolUse hook, so there is nothing to notify about",
                    ))
                }
                (AgentKind::OpenCode, false) => &["PreToolUse", "Stop"],
                (AgentKind::OpenCode, true) => &["Stop"],
                (_, false) => &["PreToolUse", "Stop"],
                (_, true) => &["Stop"],
            };
            let matcher = validate_approval_matcher(&preset.approval_matcher)?
                .map(|m| agent_matcher(agent, &m))
                .transpose()?
                .unwrap_or_else(|| default_matcher.to_string());
            // Recording events the script handles itself; never part of the default set.
            let optional: &[&str] = match agent {
                AgentKind::ClaudeCode => &[
                    "SessionStart",
                    "SessionEnd",
                    "UserPromptSubmit",
                    "PreToolUse",
                ],
                _ => &[],
            };
            let chosen = validate_approval_events(&preset.approval_events, defaults, optional)?;
            chosen
                .iter()
                .map(|event| {
                    planned(
                        event,
                        event_takes_matcher(event).then_some(matcher.as_str()),
                        hook_command(&dir, name, event),
                    )
                })
                .collect()
        }
    })
}

/// Idempotently adds the default (phone approval) ssh-app hooks to an agent's config JSON.
#[uniffi::export]
pub fn merge_agent_hooks(
    agent: AgentKind,
    existing: String,
    hook_dir: Option<String>,
) -> Result<String, SshCoreError> {
    let plan = plan_agent_hooks(agent, default_hook_preset(), hook_dir)?;
    merge_planned_hooks(agent, existing, plan)
}

/// Idempotently writes `planned` into an agent's existing config JSON (replacing any previous
/// ssh-app entries, preserving everything else). Used for install and for the dry-run preview.
#[uniffi::export]
pub fn merge_planned_hooks(
    agent: AgentKind,
    existing: String,
    planned: Vec<PlannedHook>,
) -> Result<String, SshCoreError> {
    if agent == AgentKind::OpenCode {
        return opencode_plugin(&existing, &planned);
    }
    let key = container_key(agent)?;
    let mut map = parse_object(&existing)?;
    strip_container(&mut map, key);
    let events = map
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| invalid(format!("`{key}` is not an object; refusing to modify it")))?;
    for h in &planned {
        let list = events
            .entry(h.event.as_str())
            .or_insert_with(|| Value::Array(vec![]))
            .as_array_mut()
            .ok_or_else(|| invalid(format!("`{key}.{}` is not an array", h.event)))?;
        if agent == AgentKind::Antigravity && h.matcher.is_none() {
            // Events without a matcher are flat in hooks.json: the handler goes in directly.
            list.push(handler(h.command.clone()));
            continue;
        }
        if agent == AgentKind::Crush {
            // Crush entries are flat: matcher (regex on the tool name), command, timeout (s).
            let mut entry = Map::new();
            if let Some(m) = &h.matcher {
                entry.insert("matcher".into(), json!(m));
            }
            entry.insert("command".into(), json!(h.command));
            entry.insert("timeout".into(), json!(HOOK_TIMEOUT_SECS));
            list.push(Value::Object(entry));
            continue;
        }
        let mut entry = Map::new();
        if let Some(m) = &h.matcher {
            entry.insert("matcher".into(), json!(m));
        }
        entry.insert("hooks".into(), json!([handler(h.command.clone())]));
        list.push(Value::Object(entry));
    }
    render(map)
}

fn entries_from(event: &str, list: &Value, out: &mut Vec<PlannedHook>) {
    let Some(items) = list.as_array() else { return };
    for item in items {
        if item.get("hooks").is_none() && is_our_handler(item) {
            if let Some(command) = item.get("command").and_then(Value::as_str) {
                out.push(PlannedHook {
                    event: event.to_string(),
                    // Crush's flat entries carry a matcher; Antigravity's do not.
                    matcher: item
                        .get("matcher")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    command: command.to_string(),
                });
            }
            continue;
        }
        let matcher = item
            .get("matcher")
            .and_then(Value::as_str)
            .map(str::to_string);
        for h in item
            .get("hooks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|h| is_our_handler(h))
        {
            if let Some(command) = h.get("command").and_then(Value::as_str) {
                out.push(PlannedHook {
                    event: event.to_string(),
                    matcher: matcher.clone(),
                    command: command.to_string(),
                });
            }
        }
    }
}

/// Differences between what the profile plans and what the host config holds: entries that are
/// missing, changed or not part of the plan any more. Empty when they match, and also when
/// nothing is installed at all (that is "not installed", not a change).
#[uniffi::export]
pub fn hooks_drift(planned: Vec<PlannedHook>, installed: Vec<PlannedHook>) -> Vec<String> {
    if installed.is_empty() {
        return vec![];
    }
    let show = |h: &PlannedHook| match &h.matcher {
        Some(m) => format!("{} ({m})", h.event),
        None => h.event.clone(),
    };
    let mut out = vec![];
    for p in &planned {
        if installed.contains(p) {
            continue;
        }
        match installed.iter().find(|i| i.event == p.event) {
            Some(_) => out.push(format!("{}: differs from the plan", show(p))),
            None => out.push(format!("{}: missing on the host", show(p))),
        }
    }
    for i in &installed {
        if !planned.iter().any(|p| p.event == i.event) {
            out.push(format!("{}: on the host but not in the plan", show(i)));
        }
    }
    out
}

/// The ssh-app entries currently present in an agent's config (unparsable/foreign content: none).
#[uniffi::export]
pub fn installed_hook_entries(agent: AgentKind, existing: String) -> Vec<PlannedHook> {
    if agent == AgentKind::OpenCode {
        return opencode_entries(&existing);
    }
    let (Ok(map), Ok(key)) = (parse_object(&existing), container_key(agent)) else {
        return vec![];
    };
    let mut out = vec![];
    if let Some(Value::Object(events)) = map.get(key) {
        for (event, list) in events {
            entries_from(event, list, &mut out);
        }
    }
    out
}

/// Only the hooks part of a config (Claude: `hooks`; Antigravity: `ssh-app`), pretty-printed.
/// Other settings (e.g. an `env` block with API keys) are never returned, so the app can show it.
#[uniffi::export]
pub fn hooks_section_of(agent: AgentKind, config_json: String) -> Result<String, SshCoreError> {
    if agent == AgentKind::OpenCode {
        // The plugin file holds no secrets (only the paths and matcher it was generated from).
        return Ok(config_json);
    }
    let key = container_key(agent)?;
    let map = parse_object(&config_json)?;
    let section = map.get(key).cloned().unwrap_or(Value::Object(Map::new()));
    serde_json::to_string_pretty(&section)
        .map_err(|e| invalid(format!("Failed to serialize hooks: {e}")))
}

/// Removes the ssh-app hooks, leaving every other setting and hook untouched.
#[uniffi::export]
pub fn remove_agent_hooks(agent: AgentKind, existing: String) -> Result<String, SshCoreError> {
    if agent == AgentKind::OpenCode {
        // The config writer cannot delete a file, so removal leaves an inert plugin of ours.
        return opencode_plugin(&existing, &[]);
    }
    let key = container_key(agent)?;
    let mut map = parse_object(&existing)?;
    strip_container(&mut map, key);
    render(map)
}

#[uniffi::export]
pub fn agent_hooks_installed(agent: AgentKind, existing: String) -> bool {
    !installed_hook_entries(agent, existing).is_empty()
}

/// Shell command that atomically writes `content` to `path_expr` (already shell-quoted) with `mode`.
pub(crate) fn build_write_file_command(path_expr: &str, content: &str, mode: &str) -> String {
    format!(
        "set -e; p={path_expr}; mkdir -p \"$(dirname \"$p\")\"; t=\"$p.ssh-app.tmp\"; \
         rm -f \"$t\"; umask 077; set -C; printf %s {} > \"$t\"; set +C; chmod {mode} \"$t\"; mv \"$t\" \"$p\"",
        shell_quote(content)
    )
}

pub(crate) fn remote_script_path(hook_dir: &str) -> String {
    remote_path_expr(&format!("{}/{HOOK_MARKER}", hook_dir.trim_end_matches('/')))
}

#[cfg(test)]
pub(crate) fn validate_custom_hook_for_tests(preset: &HookPreset) {
    for agent in [
        AgentKind::ClaudeCode,
        AgentKind::Antigravity,
        AgentKind::Codex,
        AgentKind::Grok,
    ] {
        if preset
            .custom_events
            .iter()
            .all(|e| hook_events_for(agent).contains(e))
        {
            validate_custom_hook(agent, preset).unwrap();
        }
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;

    #[test]
    fn only_our_own_script_is_refreshed_without_asking() {
        let new = agent_hook_script();
        assert_eq!(
            plan_hook_script_refresh("", &new, false),
            HookScriptRefresh::NotInstalled
        );
        assert_eq!(
            plan_hook_script_refresh(&new, &new, false),
            HookScriptRefresh::UpToDate
        );
        // an older copy of ours with another pin
        let old = new.replacen("HELPER_SHA256 = \"", "HELPER_SHA256 = \"00", 1);
        assert!(is_our_hook_script(&old) || !new.contains("HELPER_SHA256 = \""));
        if is_our_hook_script(&old) {
            assert_eq!(
                plan_hook_script_refresh(&old, &new, false),
                HookScriptRefresh::Refreshed
            );
        }
        // somebody else's file at that path
        let foreign = "#!/bin/sh\necho my own hook\n";
        assert_eq!(
            plan_hook_script_refresh(foreign, &new, false),
            HookScriptRefresh::NeedsConfirmation
        );
        assert_eq!(
            plan_hook_script_refresh(foreign, &new, true),
            HookScriptRefresh::Refreshed
        );
        // a file that only mentions the name is not ours
        let mention = "#!/bin/sh\n# calls ssh-app agent hook\nexec other\n";
        assert_eq!(
            plan_hook_script_refresh(mention, &new, false),
            HookScriptRefresh::NeedsConfirmation
        );
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn statusline_wraps_keeps_and_restores_the_users_command() {
        let none = merge_claude_statusline("{}".into(), None).unwrap();
        assert!(claude_statusline_installed(none.clone()));
        assert!(none.contains("claude statusline"));
        assert_eq!(merge_claude_statusline(none.clone(), None).unwrap(), none);
        assert_eq!(remove_claude_statusline(none).unwrap().trim(), "{}");

        let mine = r#"{"model":"x","statusLine":{"type":"command","command":"~/bin/sl.sh --a 'b'","padding":2}}"#;
        let wrapped = merge_claude_statusline(mine.into(), None).unwrap();
        let v: Value = serde_json::from_str(&wrapped).unwrap();
        assert_eq!(v["statusLine"]["padding"], 2);
        assert_eq!(v["model"], "x");
        // Installing twice must not wrap the wrapper.
        assert_eq!(
            merge_claude_statusline(wrapped.clone(), None).unwrap(),
            wrapped
        );
        let back: Value =
            serde_json::from_str(&remove_claude_statusline(wrapped).unwrap()).unwrap();
        assert_eq!(back["statusLine"]["command"], "~/bin/sl.sh --a 'b'");
        assert!(!claude_statusline_installed(back.to_string()));
    }

    #[test]
    fn statusline_refuses_what_it_cannot_wrap() {
        for bad in [
            r#"{"statusLine":{"type":"static","text":"x"}}"#,
            r#"{"statusLine":"x"}"#,
            "[1]",
        ] {
            assert!(merge_claude_statusline(bad.into(), None).is_err(), "{bad}");
        }
        let long = format!(
            r#"{{"statusLine":{{"type":"command","command":"{}"}}}}"#,
            "a".repeat(2001)
        );
        assert!(merge_claude_statusline(long, None).is_err());
    }
    use super::*;
    #[test]
    fn claude_merge_is_idempotent_and_preserves_user_settings() {
        let existing = r#"{"model":"opus","hooks":{"PermissionRequest":[{"matcher":"Bash","hooks":[{"type":"command","command":"/usr/local/bin/mine"}]}],"PreToolUse":[{"hooks":[{"type":"command","command":"x"}]}]}}"#;
        let once = merge_agent_hooks(AgentKind::ClaudeCode, existing.into(), None).unwrap();
        let twice = merge_agent_hooks(AgentKind::ClaudeCode, once.clone(), None).unwrap();
        assert_eq!(once, twice);
        let v: Value = serde_json::from_str(&once).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "x");
        let pr = v["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(pr.len(), 2);
        assert_eq!(pr[0]["hooks"][0]["command"], "/usr/local/bin/mine");
        assert_eq!(
            pr[1]["hooks"][0]["command"],
            "python3 -I \"$HOME/.ssh-app/bin/ssh-app-hook\" claude PermissionRequest"
        );
        assert_eq!(pr[1]["hooks"][0]["timeout"], 130);
        assert!(agent_hooks_installed(AgentKind::ClaudeCode, once));
    }

    #[test]
    fn claude_remove_restores_original_hooks() {
        let existing = r#"{"model":"opus","hooks":{"PermissionRequest":[{"matcher":"Bash","hooks":[{"type":"command","command":"mine"}]}]}}"#;
        let merged = merge_agent_hooks(AgentKind::ClaudeCode, existing.into(), None).unwrap();
        let removed = remove_agent_hooks(AgentKind::ClaudeCode, merged).unwrap();
        let a: Value = serde_json::from_str(existing).unwrap();
        let b: Value = serde_json::from_str(&removed).unwrap();
        assert_eq!(a, b);
        // From an empty config the `hooks` key disappears entirely.
        let m = merge_agent_hooks(AgentKind::ClaudeCode, String::new(), None).unwrap();
        let r = remove_agent_hooks(AgentKind::ClaudeCode, m).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&r).unwrap(), json!({}));
        assert!(!agent_hooks_installed(AgentKind::ClaudeCode, r));
    }

    #[test]
    fn antigravity_merge_and_remove() {
        let existing = r#"{"linter":{"PostToolUse":[{"matcher":"run_command","hooks":[{"type":"command","command":"./lint.sh"}]}]}}"#;
        let merged = merge_agent_hooks(
            AgentKind::Antigravity,
            existing.into(),
            Some("/opt/h".into()),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&merged).unwrap();
        assert!(v.get("linter").is_some());
        assert_eq!(
            v["ssh-app"]["PreToolUse"][0]["hooks"][0]["command"],
            "python3 -I \"/opt/h/ssh-app-hook\" antigravity PreToolUse"
        );
        assert_eq!(v["ssh-app"]["PreToolUse"][0]["matcher"], "run_command");
        assert_eq!(
            merge_agent_hooks(
                AgentKind::Antigravity,
                merged.clone(),
                Some("/opt/h".into())
            )
            .unwrap(),
            merged
        );
        assert!(agent_hooks_installed(
            AgentKind::Antigravity,
            merged.clone()
        ));
        let removed = remove_agent_hooks(AgentKind::Antigravity, merged).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&removed).unwrap(),
            serde_json::from_str::<Value>(existing).unwrap()
        );
    }

    #[test]
    fn invalid_or_non_object_config_is_never_overwritten() {
        for bad in ["{not json", "[1,2]", "\"str\""] {
            assert!(merge_agent_hooks(AgentKind::ClaudeCode, bad.into(), None).is_err());
            assert!(remove_agent_hooks(AgentKind::Antigravity, bad.into()).is_err());
            assert!(!agent_hooks_installed(AgentKind::ClaudeCode, bad.into()));
        }
        assert!(merge_agent_hooks(AgentKind::ClaudeCode, r#"{"hooks":[]}"#.into(), None).is_err());
        assert!(merge_agent_hooks(
            AgentKind::ClaudeCode,
            r#"{"hooks":{"Stop":{}}}"#.into(),
            None
        )
        .is_err());
    }

    #[test]
    fn merge_preserves_number_fidelity() {
        let existing = r#"{"big":18446744073709551616,"n":12345678901234567890123,"p":0.1000000000000000055511151231257827,"f":1.0}"#;
        let out = merge_agent_hooks(AgentKind::ClaudeCode, existing.into(), None).unwrap();
        for lit in [
            "18446744073709551616",
            "12345678901234567890123",
            "0.1000000000000000055511151231257827",
            "1.0",
        ] {
            assert!(out.contains(lit), "{lit} altered: {out}");
        }
    }

    #[test]
    fn codex_and_grok_hooks_follow_their_documented_formats() {
        let (c, g) = (AgentKind::Codex, AgentKind::Grok);
        assert_eq!(
            agent_hooks_config_path(c, None).unwrap(),
            "~/.codex/hooks.json"
        );
        assert_eq!(
            agent_hooks_config_path(c, Some("~/.codex-work".into())).unwrap(),
            "~/.codex-work/hooks.json"
        );
        assert_eq!(
            agent_hooks_config_path(g, None).unwrap(),
            "~/.grok/hooks/ssh-app.json"
        );
        // Codex: PermissionRequest (all tools) + Stop, grouped like Claude; user hooks survive.
        let existing =
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]}}"#;
        let plan = plan_agent_hooks(c, default_hook_preset(), None).unwrap();
        let out = merge_planned_hooks(c, existing.into(), plan).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hooks"]["PermissionRequest"][0]["matcher"], "*");
        assert!(v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with("codex PermissionRequest"));
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(v["hooks"].get("Notification").is_none());
        let removed: Value = serde_json::from_str(&remove_agent_hooks(c, out).unwrap()).unwrap();
        assert_eq!(removed["hooks"]["Stop"].as_array().unwrap().len(), 1);
        // Grok: PreToolUse limited to Bash by default, plus Notification and Stop.
        let plan = plan_agent_hooks(g, default_hook_preset(), None).unwrap();
        let out = merge_planned_hooks(g, String::new(), plan).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert!(v["hooks"]["Notification"].is_array() && v["hooks"]["Stop"].is_array());
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["timeout"], 130);
        let events: Vec<_> = installed_hook_entries(g, out)
            .into_iter()
            .map(|h| h.event)
            .collect();
        assert_eq!(events, ["PreToolUse", "Notification", "Stop"]);
        // Presets: notify-only and custom events are validated per agent.
        let mut notify = default_hook_preset();
        notify.kind = HookPresetKind::NotifyOnly;
        assert_eq!(plan_agent_hooks(c, notify.clone(), None).unwrap().len(), 1);
        assert_eq!(plan_agent_hooks(g, notify, None).unwrap().len(), 2);
        assert!(plan_agent_hooks(
            c,
            approval(HookPresetKind::PhoneApproval, "", &["Notification"]),
            None
        )
        .is_err());
    }

    #[test]
    fn config_paths_follow_config_dir() {
        assert_eq!(
            agent_hooks_config_path(AgentKind::ClaudeCode, None).unwrap(),
            "~/.claude/settings.json"
        );
        assert_eq!(
            agent_hooks_config_path(AgentKind::ClaudeCode, Some("~/.claude-personal/".into()))
                .unwrap(),
            "~/.claude-personal/settings.json"
        );
        assert_eq!(
            agent_hooks_config_path(AgentKind::Antigravity, None).unwrap(),
            "~/.gemini/config/hooks.json"
        );
        assert!(agent_hooks_config_path(AgentKind::ClaudeCode, Some("a\nb".into())).is_err());
        assert!(
            merge_agent_hooks(AgentKind::ClaudeCode, String::new(), Some("a\nb".into())).is_err()
        );
    }

    #[test]
    fn hook_dir_cannot_inject_shell_into_the_agent_command() {
        for bad in [
            "/x\" ; id #",
            "/x$(id)",
            "/x`id`",
            "/x\\y",
            "/x'y",
            "/x;y",
            "/x|y",
            "/x&y",
            "/x\ty",
        ] {
            assert!(
                merge_agent_hooks(AgentKind::ClaudeCode, String::new(), Some(bad.into())).is_err(),
                "{bad}"
            );
            assert!(
                merge_agent_hooks(AgentKind::Antigravity, String::new(), Some(bad.into())).is_err(),
                "{bad}"
            );
        }
        assert!(merge_agent_hooks(
            AgentKind::ClaudeCode,
            String::new(),
            Some("/opt/my hooks/bin".into())
        )
        .is_ok());
    }

    #[test]
    fn write_command_and_script_path_are_quoted() {
        let c = build_write_file_command("\"$HOME\"'/x y'", "it's {}", "600");
        assert!(c.contains("printf %s 'it'\\''s {}'"));
        assert!(c.contains("chmod 600"));
        assert!(c.contains("rm -f \"$t\"") && c.contains("set -C"));
        assert_eq!(
            remote_script_path("~/.ssh-app/bin/"),
            "\"$HOME\"'/.ssh-app/bin/ssh-app-hook'"
        );
        assert!(agent_hook_script().contains("def main"));
        let pinned = agent_hook_script();
        match crate::usage::bundled_host_helper_sha256() {
            Some(pin) => assert!(pinned.contains(&format!("HELPER_SHA256 = \"{pin}\""))),
            None => assert!(pinned.contains("HELPER_SHA256 = \"\"")),
        }
    }

    #[test]
    fn antigravity_flat_events_are_written_read_and_removed_as_flat_handlers() {
        let a = AgentKind::Antigravity;
        let existing = r#"{"mine":{"Stop":[{"command":"./mine.sh"}]}}"#;
        let plan = plan_agent_hooks(a, default_hook_preset(), None).unwrap();
        let out = merge_planned_hooks(a, existing.into(), plan).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        // PreToolUse is grouped with its matcher; Stop is a flat list of handlers.
        assert_eq!(v["ssh-app"]["PreToolUse"][0]["matcher"], "run_command");
        assert!(v["ssh-app"]["PreToolUse"][0]["hooks"][0]["command"].is_string());
        assert!(v["ssh-app"]["Stop"][0].get("hooks").is_none());
        assert!(v["ssh-app"]["Stop"][0]["command"]
            .as_str()
            .unwrap()
            .contains("Stop"));
        let events: Vec<_> = installed_hook_entries(a, out.clone())
            .into_iter()
            .map(|h| h.event)
            .collect();
        assert_eq!(events, ["PreToolUse", "Stop"]);
        // Reinstall does not stack, removal keeps the user's own flat handler.
        let plan = plan_agent_hooks(a, default_hook_preset(), None).unwrap();
        let again = merge_planned_hooks(a, out, plan).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&again).unwrap()["ssh-app"]["Stop"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let removed = remove_agent_hooks(a, again).unwrap();
        let v: Value = serde_json::from_str(&removed).unwrap();
        assert!(v.get("ssh-app").is_none());
        assert_eq!(v["mine"]["Stop"][0]["command"], "./mine.sh");
    }

    fn approval(kind: HookPresetKind, matcher: &str, events: &[&str]) -> HookPreset {
        HookPreset {
            kind,
            approval_matcher: matcher.into(),
            approval_events: events.iter().map(|e| e.to_string()).collect(),
            ..default_hook_preset()
        }
    }

    #[test]
    fn approval_matcher_and_events_are_configurable_and_validated() {
        use HookPresetKind::{NotifyOnly, PhoneApproval};
        let plan = |a, p| plan_agent_hooks(a, p, None);
        let c = AgentKind::ClaudeCode;
        let p = plan(
            c,
            approval(PhoneApproval, " Bash|Edit ", &["Stop", "PermissionRequest"]),
        )
        .unwrap();
        let got: Vec<_> = p
            .iter()
            .map(|h| (h.event.as_str(), h.matcher.as_deref()))
            .collect();
        assert_eq!(
            got,
            [("PermissionRequest", Some("Bash|Edit")), ("Stop", None)]
        );
        // Defaults are unchanged when nothing is chosen.
        let d = plan(c, default_hook_preset()).unwrap();
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].matcher.as_deref(), Some("*"));
        // Notify-only has no tool event, so the matcher is irrelevant and PermissionRequest is refused.
        assert_eq!(plan(c, approval(NotifyOnly, "Bash", &[])).unwrap().len(), 2);
        assert!(plan(c, approval(NotifyOnly, "", &["PermissionRequest"])).is_err());
        // Antigravity keeps its own default matcher and events.
        let a = plan(
            AgentKind::Antigravity,
            approval(PhoneApproval, "", &["PreToolUse"]),
        )
        .unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].matcher.as_deref(), Some(ANTIGRAVITY_TOOL_MATCHER));
        assert!(plan(
            AgentKind::Antigravity,
            approval(PhoneApproval, "", &["Notification"])
        )
        .is_err());
        // Hostile values are rejected, never written.
        for bad in ["Bash;rm", "a b", "x\ny", "\"", &"a".repeat(201)] {
            assert!(
                plan(c, approval(PhoneApproval, bad, &[])).is_err(),
                "{bad:?}"
            );
        }
        assert!(plan(c, approval(PhoneApproval, "", &["Stop\n", "Bogus"])).is_err());
        // Duplicates collapse.
        assert_eq!(
            plan(c, approval(PhoneApproval, "", &["Stop", "Stop"]))
                .unwrap()
                .len(),
            1
        );
    }

    fn custom(cmd: &str, events: &[&str], matcher: &str) -> HookPreset {
        HookPreset {
            kind: HookPresetKind::Custom,
            custom_command: cmd.into(),
            custom_events: events.iter().map(|e| e.to_string()).collect(),
            custom_matcher: matcher.into(),
            approval_matcher: String::new(),
            approval_events: vec![],
        }
    }

    #[test]
    fn presets_plan_expected_events() {
        let ev = |a, k| {
            let mut p = default_hook_preset();
            p.kind = k;
            plan_agent_hooks(a, p, None)
                .unwrap()
                .into_iter()
                .map(|h| h.event)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ev(AgentKind::ClaudeCode, HookPresetKind::PhoneApproval),
            ["PermissionRequest", "Notification", "Stop"]
        );
        assert_eq!(
            ev(AgentKind::ClaudeCode, HookPresetKind::NotifyOnly),
            ["Notification", "Stop"]
        );
        assert_eq!(
            ev(AgentKind::Antigravity, HookPresetKind::NotifyOnly),
            ["Stop"]
        );
    }

    #[test]
    fn custom_preset_is_marked_deduped_and_removable() {
        let p = custom(
            "/usr/local/bin/mine --flag",
            &["PreToolUse", "Stop", "Stop"],
            "Bash|Edit",
        );
        let plan = plan_agent_hooks(AgentKind::ClaudeCode, p.clone(), None).unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].matcher.as_deref(), Some("Bash|Edit"));
        assert_eq!(plan[1].matcher, None);
        assert!(plan[0].command.ends_with(" # ssh-app-hook-custom"));
        assert!(!hook_preset_uses_script(p));

        let merged =
            merge_planned_hooks(AgentKind::ClaudeCode, "{\"theme\":1}".into(), plan).unwrap();
        assert!(agent_hooks_installed(AgentKind::ClaudeCode, merged.clone()));
        let cleaned = remove_agent_hooks(AgentKind::ClaudeCode, merged).unwrap();
        assert!(!agent_hooks_installed(
            AgentKind::ClaudeCode,
            cleaned.clone()
        ));
        assert!(cleaned.contains("theme"));
    }

    #[test]
    fn custom_preset_validation() {
        let plan = |p| plan_agent_hooks(AgentKind::Antigravity, p, None);
        assert!(plan(custom("", &["Stop"], "")).is_err());
        assert!(plan(custom("x\ny", &["Stop"], "")).is_err());
        assert!(plan(custom("x", &[], "")).is_err());
        assert!(plan(custom("x", &["Notification"], "")).is_err());
        assert!(plan(custom("x", &["Stop"], "a b;")).is_err());
        let ok = plan(custom("x", &["PreToolUse"], "")).unwrap();
        assert_eq!(ok[0].matcher.as_deref(), Some("run_command"));
    }

    #[test]
    fn hooks_section_hides_foreign_settings_and_lists_installed() {
        let existing = r#"{"env":{"ANTHROPIC_API_KEY":"sk-secret"},"apiKeyHelper":"x",
            "hooks":{"Stop":[{"hooks":[{"type":"command","command":"theirs"}]}]}}"#;
        let plan = plan_agent_hooks(AgentKind::ClaudeCode, default_hook_preset(), None).unwrap();
        let merged = merge_planned_hooks(AgentKind::ClaudeCode, existing.into(), plan).unwrap();
        let section = hooks_section_of(AgentKind::ClaudeCode, merged.clone()).unwrap();
        assert!(!section.contains("sk-secret") && !section.contains("apiKeyHelper"));
        assert!(section.contains("theirs") && section.contains("PermissionRequest"));
        let installed = installed_hook_entries(AgentKind::ClaudeCode, merged);
        assert_eq!(installed.len(), 3);
        assert!(installed.iter().all(|h| h.command.contains(HOOK_MARKER)));
        assert!(!installed.iter().any(|h| h.command == "theirs"));
        assert!(installed_hook_entries(AgentKind::ClaudeCode, "not json".into()).is_empty());
    }

    #[test]
    fn antigravity_installed_entries_round_trip() {
        let plan = plan_agent_hooks(AgentKind::Antigravity, default_hook_preset(), None).unwrap();
        let merged =
            merge_planned_hooks(AgentKind::Antigravity, "{}".into(), plan.clone()).unwrap();
        assert_eq!(installed_hook_entries(AgentKind::Antigravity, merged), plan);
    }

    #[test]
    fn removal_keeps_foreign_siblings_and_lookalike_paths() {
        let existing = r#"{"hooks":{"Stop":[
          {"hooks":[{"type":"command","command":"theirs-sibling"},
                    {"type":"command","command":"python3 -I \"$HOME/.ssh-app/bin/ssh-app-hook\" claude Stop"}]},
          {"hooks":[{"type":"command","command":"/home/u/code/ssh-app-hook-tests/run.sh"}]},
          {"hooks":[]}]}}"#;
        let out = remove_agent_hooks(AgentKind::ClaudeCode, existing.into()).unwrap();
        assert!(out.contains("theirs-sibling"), "{out}");
        assert!(out.contains("ssh-app-hook-tests/run.sh"), "{out}");
        assert!(!out.contains("claude Stop"), "{out}");
        assert!(
            out.contains("\"hooks\": []"),
            "pre-existing empty entry stays: {out}"
        );
        assert!(!agent_hooks_installed(AgentKind::ClaudeCode, out));
    }

    #[test]
    fn antigravity_keeps_foreign_handlers_under_the_same_key() {
        let existing =
            r#"{"ssh-app":{"PreToolUse":[{"hooks":[{"command":"mine-own-named-hook"}]}]}}"#;
        let plan = plan_agent_hooks(AgentKind::Antigravity, default_hook_preset(), None).unwrap();
        let merged = merge_planned_hooks(AgentKind::Antigravity, existing.into(), plan).unwrap();
        assert!(merged.contains("mine-own-named-hook") && merged.contains("antigravity Stop"));
        let rm = remove_agent_hooks(AgentKind::Antigravity, merged).unwrap();
        assert!(rm.contains("mine-own-named-hook") && !rm.contains("antigravity"));
    }

    #[test]
    fn duplicate_keys_are_refused_not_silently_dropped() {
        let dup = r#"{"model":"a","hooks":{},"model":"b"}"#;
        let err = merge_agent_hooks(AgentKind::ClaudeCode, dup.into(), None).unwrap_err();
        assert!(err.to_string().contains("duplicate key"), "{err}");
        let nested = r#"{"hooks":{"Stop":[],"Stop":[]}}"#;
        assert!(remove_agent_hooks(AgentKind::ClaudeCode, nested.into()).is_err());
        // Same key in different objects and keys inside strings are fine.
        let ok = r#"{"a":{"x":1},"b":{"x":"{\"x\":1,\"x\":2}"},"c":[{"x":1},{"x":2}]}"#;
        assert!(merge_agent_hooks(AgentKind::ClaudeCode, ok.into(), None).is_ok());
    }

    fn plan(agent: AgentKind) -> Vec<PlannedHook> {
        plan_agent_hooks(agent, default_hook_preset(), None).unwrap()
    }

    #[test]
    fn qwen_uses_claude_style_grouped_hooks_in_its_settings() {
        assert_eq!(
            agent_hooks_config_path(AgentKind::QwenCode, None).unwrap(),
            "~/.qwen/settings.json"
        );
        let p = plan(AgentKind::QwenCode);
        let events: Vec<_> = p.iter().map(|h| h.event.as_str()).collect();
        assert_eq!(events, ["PermissionRequest", "Notification", "Stop"]);
        assert!(p[0]
            .command
            .ends_with("ssh-app-hook\" qwen PermissionRequest"));
        let merged = merge_planned_hooks(
            AgentKind::QwenCode,
            r#"{"theme":"dark","hooks":{"PreToolUse":[{"matcher":"x","hooks":[{"type":"command","command":"mine"}]}]}}"#.into(),
            p.clone(),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(
            v["hooks"]["PermissionRequest"][0]["hooks"][0]["type"],
            "command"
        );
        assert_eq!(
            installed_hook_entries(AgentKind::QwenCode, merged.clone()),
            p
        );
        let removed = remove_agent_hooks(AgentKind::QwenCode, merged).unwrap();
        let v: Value = serde_json::from_str(&removed).unwrap();
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "mine");
        assert!(v["hooks"].get("PermissionRequest").is_none());
    }

    #[test]
    fn crush_gets_flat_pre_tool_use_entries_with_an_anchored_matcher() {
        let p = plan(AgentKind::Crush);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].event, "PreToolUse");
        assert_eq!(p[0].matcher.as_deref(), Some("^bash$"));
        let merged = merge_planned_hooks(
            AgentKind::Crush,
            r#"{"models":{"a":1},"hooks":{"PreToolUse":[{"command":"mine","matcher":"^edit$"}]}}"#
                .into(),
            p.clone(),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&merged).unwrap();
        let list = v["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1]["matcher"], "^bash$");
        assert_eq!(list[1]["timeout"], HOOK_TIMEOUT_SECS);
        assert!(list[1].get("hooks").is_none() && list[1].get("type").is_none());
        // What is installed equals the plan: no drift is reported.
        let installed = installed_hook_entries(AgentKind::Crush, merged.clone());
        assert_eq!(installed, p);
        assert!(hooks_drift(p, installed).is_empty());
        // Removal takes only ours and keeps the user's own hook and the rest of the file.
        let v: Value =
            serde_json::from_str(&remove_agent_hooks(AgentKind::Crush, merged).unwrap()).unwrap();
        assert_eq!(v["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(v["models"]["a"], 1);
    }

    #[test]
    fn crush_and_opencode_matchers_are_tool_name_lists() {
        let preset = |m: &str| HookPreset {
            approval_matcher: m.into(),
            ..default_hook_preset()
        };
        let crush = plan_agent_hooks(AgentKind::Crush, preset("bash|edit"), None).unwrap();
        assert_eq!(crush[0].matcher.as_deref(), Some("^(bash|edit)$"));
        let all = plan_agent_hooks(AgentKind::Crush, preset("*"), None).unwrap();
        assert_eq!(all[0].matcher.as_deref(), Some(".*"));
        for bad in ["a.b", "a:b", "(x)", "$"] {
            assert!(
                plan_agent_hooks(AgentKind::Crush, preset(bad), None).is_err(),
                "{bad}"
            );
            assert!(
                plan_agent_hooks(AgentKind::OpenCode, preset(bad), None).is_err(),
                "{bad}"
            );
        }
        let custom = HookPreset {
            kind: HookPresetKind::Custom,
            custom_command: "true".into(),
            custom_events: vec!["PreToolUse".into()],
            ..default_hook_preset()
        };
        assert!(plan_agent_hooks(AgentKind::Crush, custom.clone(), None).is_err());
        assert!(plan_agent_hooks(AgentKind::OpenCode, custom, None).is_err());
        let notify = HookPreset {
            kind: HookPresetKind::NotifyOnly,
            ..default_hook_preset()
        };
        assert!(plan_agent_hooks(AgentKind::Crush, notify, None).is_err());
    }

    #[test]
    fn opencode_plugin_blocks_by_throwing_and_round_trips() {
        assert_eq!(
            agent_hooks_config_path(AgentKind::OpenCode, None).unwrap(),
            "~/.config/opencode/plugins/ssh-app.js"
        );
        let p = plan(AgentKind::OpenCode);
        let text = merge_planned_hooks(AgentKind::OpenCode, String::new(), p.clone()).unwrap();
        assert!(text.contains("\"tool.execute.before\""));
        assert!(text.contains("throw new Error"));
        assert!(text.contains("execFile(\"python3\""), "no shell: {text}");
        assert!(text.contains("join(homedir(), \".ssh-app/bin/ssh-app-hook\")"));
        assert!(text.contains("^(?:bash)$"));
        assert!(text.contains("session.idle"));
        assert_eq!(installed_hook_entries(AgentKind::OpenCode, text.clone()), p);
        // Idempotent, and removal leaves an inert plugin of ours (no tool hook, no entries).
        assert_eq!(
            merge_planned_hooks(AgentKind::OpenCode, text.clone(), p).unwrap(),
            text
        );
        let removed = remove_agent_hooks(AgentKind::OpenCode, text).unwrap();
        assert!(!removed.contains("tool.execute.before"));
        assert!(installed_hook_entries(AgentKind::OpenCode, removed).is_empty());
    }

    #[test]
    fn opencode_generator_refuses_hostile_entries_and_escapes_line_separators() {
        let ok = plan(AgentKind::OpenCode);
        let with = |f: &dyn Fn(&mut PlannedHook)| {
            let mut p = ok.clone();
            f(&mut p[0]);
            merge_planned_hooks(AgentKind::OpenCode, String::new(), p)
        };
        assert!(with(&|h| h.command.push_str("\u{2028}globalThis.x=1;//")).is_err());
        assert!(with(&|h| h.command.push_str("\u{2029}x")).is_err());
        assert!(with(&|h| h.command.push('\n')).is_err());
        assert!(with(&|h| h.command = "node evil.js".into()).is_err());
        assert!(with(&|h| h.event = "PostToolUse".into()).is_err());
        assert!(with(&|h| h.matcher = Some("(a+)+$".into())).is_err());
        assert!(with(&|h| h.matcher = Some("a\"+process.exit()+\"".into())).is_err());
        assert!(with(&|h| h.matcher = Some("bash|edit".into())).is_ok());
        // The header line stays one line whatever the (validated) content is.
        let text = merge_planned_hooks(AgentKind::OpenCode, String::new(), ok).unwrap();
        assert!(!text
            .lines()
            .next()
            .unwrap()
            .contains(['\u{2028}', '\u{2029}']));
    }

    #[test]
    fn opencode_never_overwrites_a_foreign_plugin_file() {
        let foreign = "export const Mine = async () => ({})\n";
        let p = plan(AgentKind::OpenCode);
        assert!(merge_planned_hooks(AgentKind::OpenCode, foreign.into(), p).is_err());
        assert!(remove_agent_hooks(AgentKind::OpenCode, foreign.into()).is_err());
        assert!(installed_hook_entries(AgentKind::OpenCode, foreign.into()).is_empty());
    }

    #[test]
    fn opencode_custom_hook_dir_is_used_as_an_absolute_path() {
        let p = plan_agent_hooks(
            AgentKind::OpenCode,
            default_hook_preset(),
            Some("/opt/hooks".into()),
        )
        .unwrap();
        let text = merge_planned_hooks(AgentKind::OpenCode, String::new(), p).unwrap();
        assert!(
            text.contains("const SCRIPT = \"/opt/hooks/ssh-app-hook\""),
            "{text}"
        );
    }

    #[test]
    fn agents_without_hooks_return_typed_errors_everywhere() {
        for a in [
            AgentKind::Cline,
            AgentKind::Aider,
            AgentKind::Goose,
            AgentKind::OllamaChat,
        ] {
            assert!(!a.has_hooks());
            assert!(agent_hooks_config_path(a, None).is_err());
            assert!(plan_agent_hooks(a, default_hook_preset(), None).is_err());
            assert!(merge_agent_hooks(a, "{}".into(), None).is_err());
            assert!(remove_agent_hooks(a, "{}".into()).is_err());
            assert!(hooks_section_of(a, "{}".into()).is_err());
            assert!(installed_hook_entries(a, "{}".into()).is_empty());
            assert!(!agent_hooks_installed(a, "{}".into()));
            assert!(hook_events_for(a).is_empty());
            assert!(crate::host_guard::host_guard_agent_support(a).is_empty());
        }
    }

    #[test]
    fn custom_command_must_keep_the_marker_a_comment() {
        let plan =
            |c: &str| plan_agent_hooks(AgentKind::ClaudeCode, custom(c, &["Stop"], ""), None);
        for bad in [
            "echo hi \\",
            "foo 'bar",
            "foo \"bar",
            "echo (",
            "echo )",
            "a |",
            "a &&",
            "echo $(x",
        ] {
            assert!(plan(bad).is_err(), "{bad:?} should be rejected");
        }
        for good in [
            "echo 'a b' \"c d\"",
            "x | y",
            "audit --log \\; true",
            "echo $(date)",
            "a &",
            "echo \\'",
        ] {
            assert!(plan(good).is_ok(), "{good:?} should be accepted");
        }
    }
}
