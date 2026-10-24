use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HookPresetKind {
    /// Ask the phone to approve tool use (falls back to the agent's own prompt), plus notifications.
    PhoneApproval,
    /// Only Notification/Stop events are forwarded; nothing can be approved or blocked.
    NotifyOnly,
    /// The user's own command for the chosen events; the ssh-app hook script is not used.
    Custom,
    /// No ssh-app hook at all; only the per-event own commands (`HookPreset::own`) are installed.
    Off,
}

/// When an extra command runs relative to the ssh-app hook of the same event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HookExtraOrder {
    Before,
    After,
}

/// A command the user adds to one event of the ssh-app hook (`PhoneApproval` / `NotifyOnly`). The
/// agent runs a small wrapper that runs this command and the ssh-app hook in the chosen order,
/// with the event JSON on stdin of both.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HookExtra {
    pub event: String,
    pub command: String,
    pub order: HookExtraOrder,
    /// `Before` on a blocking event only: a failing command denies the action and skips the hook.
    pub stop_on_failure: bool,
    /// 1 to 60 seconds; a command still running then is killed and counts as failed.
    pub timeout_secs: u32,
}

/// The user's own command for one event, installed next to (never instead of) the other events'
/// ssh-app hooks. The event must not also be one of the ssh-app events of the preset.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HookOwn {
    pub event: String,
    pub command: String,
    /// Tool matcher for tool events; empty = the agent's default.
    pub matcher: String,
}

/// What an event is, for the hooks screen (texts live here so the phone does not repeat them).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HookEventInfo {
    pub title: String,
    pub summary: String,
    /// The event can approve or block an action (so "stop on failure" means something).
    pub can_block: bool,
    pub takes_matcher: bool,
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
    /// `PhoneApproval` / `NotifyOnly`: extra commands, at most one per event of the preset.
    pub extras: Vec<HookExtra>,
    /// Any kind but `Custom`: the user's own command for events the preset does not install.
    pub own: Vec<HookOwn>,
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
        extras: vec![],
        own: vec![],
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
    matches!(
        preset.kind,
        HookPresetKind::PhoneApproval | HookPresetKind::NotifyOnly
    )
}

/// Whether the preset needs the extra-command wrapper next to the hook script.
#[uniffi::export]
pub fn hook_preset_uses_extra(preset: HookPreset) -> bool {
    hook_preset_uses_script(preset.clone()) && !preset.extras.is_empty()
}

/// Events at which a hook can approve or block the agent's action.
pub(super) fn event_can_block(event: &str) -> bool {
    matches!(event, "PermissionRequest" | "PreToolUse")
}

/// Title, one-line meaning and capabilities of an event, shown on the hooks screen.
#[uniffi::export]
pub fn hook_event_info(agent: AgentKind, event: String) -> HookEventInfo {
    let (title, summary) = match event.as_str() {
        "PermissionRequest" => (
            "Permission request",
            "The agent asks permission for an action. With phone approval the answer comes from your phone.",
        ),
        "PreToolUse" => (
            "Before a tool runs",
            "Runs before every tool call and can approve or block it.",
        ),
        "PostToolUse" => (
            "After a tool ran",
            "Runs after a tool call finished. It cannot block; use it for logging or checks.",
        ),
        "UserPromptSubmit" => (
            "Prompt sent",
            "You sent a prompt. Used to add context or to record it.",
        ),
        "Notification" => (
            "Needs attention",
            "The agent is waiting for you or needs attention.",
        ),
        "Stop" => ("Turn finished", "The agent finished its turn."),
        "SubagentStop" => ("Subagent finished", "A subagent finished its work."),
        "SessionStart" => ("Session started", "A session began."),
        "SessionEnd" => ("Session ended", "A session ended."),
        "PreCompact" => (
            "Before compaction",
            "The agent is about to compact its context.",
        ),
        _ => ("Event", "An event of this agent."),
    };
    let _ = agent;
    HookEventInfo {
        title: title.to_string(),
        summary: summary.to_string(),
        can_block: event_can_block(&event),
        takes_matcher: event_takes_matcher(&event),
    }
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

/// 1-1000 characters on one line, for a user's own or extra command.
pub(super) fn validate_command_line(what: &str, command: &str) -> Result<(), SshCoreError> {
    let cmd = command.trim();
    if cmd.is_empty() || cmd.len() > 1000 || has_control(cmd) {
        return Err(invalid(format!(
            "{what} must be 1-1000 characters on a single line"
        )));
    }
    Ok(())
}

/// Extras belong to events the preset installs, at most one per event.
pub(super) fn validate_extras(
    agent: AgentKind,
    extras: &[HookExtra],
    installed_events: &[String],
) -> Result<(), SshCoreError> {
    if extras.is_empty() {
        return Ok(());
    }
    if matches!(agent, AgentKind::Crush | AgentKind::OpenCode) {
        return Err(invalid("Extra commands are not supported for this agent"));
    }
    let mut seen: Vec<&str> = vec![];
    for x in extras {
        if !installed_events.contains(&x.event) {
            return Err(invalid(format!(
                "An extra command needs the ssh-app hook on '{}'",
                crate::approval_queue::display_safe(&x.event.chars().take(40).collect::<String>())
            )));
        }
        if seen.contains(&x.event.as_str()) {
            return Err(invalid(format!(
                "Event '{}' has two extra commands",
                x.event
            )));
        }
        seen.push(&x.event);
        validate_command_line("An extra command", &x.command)?;
        if !(1..=60).contains(&x.timeout_secs) {
            return Err(invalid(
                "An extra command's timeout must be 1 to 60 seconds",
            ));
        }
        if x.stop_on_failure && (x.order != HookExtraOrder::Before || !event_can_block(&x.event)) {
            return Err(invalid(
                "Stopping on failure needs the command to run before the hook, on an event that can block",
            ));
        }
    }
    Ok(())
}

/// Own commands go on events the preset does not install; at most one per event.
pub(super) fn validate_own(
    agent: AgentKind,
    own: &[HookOwn],
    installed_events: &[String],
) -> Result<(), SshCoreError> {
    if own.is_empty() {
        return Ok(());
    }
    if matches!(agent, AgentKind::Crush | AgentKind::OpenCode) {
        return Err(invalid(
            "Own commands per event are not supported for this agent",
        ));
    }
    let allowed = hook_events_for(agent);
    let mut seen: Vec<&str> = vec![];
    for o in own {
        if !allowed.contains(&o.event) {
            return Err(invalid(format!(
                "{agent:?} has no hook event '{}'",
                o.event
            )));
        }
        if installed_events.contains(&o.event) {
            return Err(invalid(format!(
                "Event '{}' is handled by the ssh-app hook; choose one or the other",
                o.event
            )));
        }
        if seen.contains(&o.event.as_str()) {
            return Err(invalid(format!("Event '{}' has two own commands", o.event)));
        }
        seen.push(&o.event);
        validate_command_line("An own command", &o.command)?;
        if let Some(problem) = shell_syntax_problem(o.command.trim()) {
            return Err(invalid(format!(
                "Own command looks incomplete ({problem}); the marker comment appended to it would not be a comment"
            )));
        }
        let m = o.matcher.trim();
        if !m
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '*' | '|' | '.' | ':' | '-'))
        {
            return Err(invalid(
                "Matcher may only contain letters, digits and _ * | . : -",
            ));
        }
    }
    Ok(())
}
