use super::*;

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

pub(super) fn event_takes_matcher(event: &str) -> bool {
    matches!(event, "PermissionRequest" | "PreToolUse" | "PostToolUse")
}

/// Best-effort check that appending ` # marker` really appends a comment: no trailing backslash,
/// no dangling operator, balanced quotes and parentheses.
pub(super) fn shell_syntax_problem(cmd: &str) -> Option<&'static str> {
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

pub(super) fn validate_custom_hook(
    agent: AgentKind,
    preset: &HookPreset,
) -> Result<(), SshCoreError> {
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
pub(super) fn agent_matcher(agent: AgentKind, m: &str) -> Result<String, SshCoreError> {
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
pub(super) fn validate_approval_matcher(raw: &str) -> Result<Option<String>, SshCoreError> {
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
pub(super) fn validate_approval_events(
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
