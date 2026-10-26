use super::*;

/// The hook dir is embedded in a command the *agent's shell* executes (inside double quotes), so
/// it must not contain anything the shell would interpret (`$`, backticks, quotes, `\`, ...).
pub(super) fn validate_hook_dir(dir: &str) -> Result<(), SshCoreError> {
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

pub(super) fn validate_dir(dir: &str) -> Result<(), SshCoreError> {
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

/// Where the hooks go: the agent's user-level file (every session of the agent) or one project's
/// own file (only sessions started in that folder). Spec 61.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HookScope {
    /// The agent's config directory (`None` = its default one).
    User { config_dir: Option<String> },
    /// A project folder on the host; `user_config_dir` is where the agent's user-level hooks would
    /// be, to refuse a project install that would double an all-sessions one.
    Project {
        folder: String,
        user_config_dir: Option<String>,
    },
}

/// Remote path of the project-level file the hooks live in, from each agent's docs: Claude Code
/// `settings.local.json` (personal, not committed by convention), Antigravity `.agents/hooks.json`,
/// Codex `<repo>/.codex/hooks.json` (loaded only when the project layer is trusted, then reviewed
/// with `/hooks`), Grok `<project>/.grok/hooks/` (any `.json` file) (needs `/hooks-trust`; the installer owns
/// `ssh-app.json`), Qwen Code `.qwen/settings.json`, OpenCode `.opencode/plugins/`. Crush's project
/// file is not confirmed, so it stays user-level.
#[uniffi::export]
pub fn agent_project_hooks_path(agent: AgentKind, folder: String) -> Result<String, SshCoreError> {
    let file = match agent {
        AgentKind::ClaudeCode => ".claude/settings.local.json",
        AgentKind::Antigravity => ".agents/hooks.json",
        AgentKind::Codex => ".codex/hooks.json",
        AgentKind::Grok => ".grok/hooks/ssh-app.json",
        AgentKind::QwenCode => ".qwen/settings.json",
        AgentKind::OpenCode => ".opencode/plugins/ssh-app.js",
        other => {
            return Err(invalid(format!(
                "Project-level hooks are not available for {other:?} yet; use all sessions"
            )))
        }
    };
    let folder = folder.trim();
    validate_dir(folder)?;
    if !(folder == "~" || folder.starts_with("~/") || folder.starts_with('/'))
        || folder.split('/').any(|c| c == "..")
    {
        return Err(invalid(
            "Project folder must be absolute or start with ~/, without ..",
        ));
    }
    Ok(format!("{}/{file}", folder.trim_end_matches('/')))
}

/// The config file of `scope` for `agent`.
#[uniffi::export]
pub fn agent_hooks_target_path(agent: AgentKind, scope: HookScope) -> Result<String, SshCoreError> {
    match scope {
        HookScope::User { config_dir } => agent_hooks_config_path(agent, config_dir),
        HookScope::Project { folder, .. } => agent_project_hooks_path(agent, folder),
    }
}

/// A project install is refused while the same agent's user-level hooks are present: both levels
/// would register, and every request would reach the phone twice.
pub fn check_project_install(
    agent: AgentKind,
    user_level_config: String,
) -> Result<(), SshCoreError> {
    if agent_hooks_installed(agent, user_level_config) {
        return Err(invalid(
            "The hooks are already active for every session of this agent; remove them there first",
        ));
    }
    Ok(())
}

pub(super) fn no_hooks(agent: AgentKind) -> SshCoreError {
    invalid(format!(
        "{agent:?} has no hook mechanism ssh-app can use; it asks in its own terminal"
    ))
}

pub(super) fn hook_command(hook_dir: &str, agent: &str, event: &str) -> String {
    let dir = hook_dir.trim_end_matches('/');
    let path = match dir.strip_prefix("~/") {
        Some(rest) => format!("$HOME/{rest}/{HOOK_MARKER}"),
        None => format!("{dir}/{HOOK_MARKER}"),
    };
    format!("python3 -I \"{path}\" {agent} {event}")
}

pub const EXTRA_WRAPPER: &str = "ssh-app-hook-extra";

fn hook_dir_path(hook_dir: &str, file: &str) -> String {
    let dir = hook_dir.trim_end_matches('/');
    match dir.strip_prefix("~/") {
        Some(rest) => format!("$HOME/{rest}/{file}"),
        None => format!("{dir}/{file}"),
    }
}

/// The command written for an event that has an extra command: the wrapper runs the user's
/// command (base64, so no quoting can break out of it) and the ordinary hook in the chosen order.
pub(super) fn extra_command(hook_dir: &str, agent: &str, event: &str, x: &HookExtra) -> String {
    use base64::Engine;
    let wrapper = hook_dir_path(hook_dir, EXTRA_WRAPPER);
    let cmd = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(x.command.trim());
    let order = match x.order {
        HookExtraOrder::Before => "before",
        HookExtraOrder::After => "after",
    };
    format!(
        "python3 -I \"{wrapper}\" --cmd {cmd} --order {order} --stop {} --timeout {} --agent {agent} --event {event} -- {}",
        u8::from(x.stop_on_failure),
        x.timeout_secs,
        hook_command(hook_dir, agent, event)
    )
}

/// A readable form of a command ssh-app wrote, for the install preview: the wrapper's base64 is
/// decoded, an own command loses its marker comment, anything else is returned as it is.
#[uniffi::export]
pub fn hook_command_summary(command: String) -> String {
    use base64::Engine;
    if let Some(own) = command.strip_suffix(&format!(" # {HOOK_MARKER}-custom")) {
        return format!("own command: {own}");
    }
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let value = |flag: &str| {
        tokens
            .iter()
            .position(|t| *t == flag)
            .and_then(|i| tokens.get(i + 1).copied())
    };
    let is_wrapper = tokens.get(2).is_some_and(|t| t.contains(EXTRA_WRAPPER));
    if let (true, Some(b64)) = (is_wrapper, value("--cmd")) {
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(b64)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_else(|| "(unreadable)".to_string());
        let stop = if value("--stop") == Some("1") {
            ", stops on failure"
        } else {
            ""
        };
        return format!(
            "ssh-app hook + extra command ({} the hook{stop}, {} s): {decoded}",
            value("--order").unwrap_or("before"),
            value("--timeout").unwrap_or("10"),
        );
    }
    command
}

/// Exactly the commands we write: the script invocation (`python3 -I ".../ssh-app-hook" <agent>
/// <Event>`) or a custom command ending in our marker comment. A substring match would also
/// delete a user's unrelated hook that merely mentions the marker in a path.
pub(super) fn is_our_command(command: &str) -> bool {
    command.ends_with(&format!(" # {HOOK_MARKER}-custom"))
        || (command.starts_with("python3 -I \"") && command.contains(&format!("/{HOOK_MARKER}\" ")))
}

pub(super) fn is_our_handler(h: &Value) -> bool {
    h.get("command")
        .and_then(Value::as_str)
        .map(is_our_command)
        .unwrap_or(false)
}

/// Key that holds `{event: [entries]}` in the agent's config.
pub(super) fn container_key(agent: AgentKind) -> Result<&'static str, SshCoreError> {
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
pub(super) fn find_duplicate_key(text: &str) -> Option<String> {
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

pub(super) fn handler(command: String) -> Value {
    json!({ "type": "command", "command": command, "timeout": handler_timeout(&command) })
}

/// The agent's time limit for a hook: the hook's own wait, plus the extra command's time when the
/// wrapper runs one first (otherwise the agent would cut the wrapper off during the hook's wait).
fn handler_timeout(command: &str) -> u64 {
    let mut tokens = command.split_whitespace();
    let extra = if command.contains(EXTRA_WRAPPER) {
        tokens
            .by_ref()
            .skip_while(|t| *t != "--timeout")
            .nth(1)
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(0, |v| v.min(60))
    } else {
        0
    };
    HOOK_TIMEOUT_SECS + extra
}

/// Removes only our handlers from `<key>.<event>` (keeping everyone else's, including siblings in
/// the same entry) and drops entries/events/containers that become empty.
pub(super) fn strip_container(map: &mut Map<String, Value>, key: &str) {
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
