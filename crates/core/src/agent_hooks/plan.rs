use super::*;

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
    let own_entries = |installed: &[String]| -> Result<Vec<PlannedHook>, SshCoreError> {
        validate_own(agent, &preset.own, installed)?;
        Ok(preset
            .own
            .iter()
            .map(|o| {
                let m = o.matcher.trim();
                let matcher = event_takes_matcher(&o.event).then_some(if m.is_empty() {
                    default_matcher
                } else {
                    m
                });
                planned(
                    &o.event,
                    matcher,
                    format!("{} # {HOOK_MARKER}-custom", o.command.trim()),
                )
            })
            .collect())
    };
    Ok(match preset.kind {
        HookPresetKind::Off => {
            if !preset.extras.is_empty() {
                return Err(invalid("Extra commands need the ssh-app hook"));
            }
            own_entries(&[])?
        }
        HookPresetKind::Custom if !preset.extras.is_empty() || !preset.own.is_empty() => {
            return Err(invalid(
                "A custom hook replaces everything; per-event extra and own commands do not combine with it",
            ))
        }
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
            validate_extras(agent, &preset.extras, &chosen)?;
            let mut entries: Vec<PlannedHook> = chosen
                .iter()
                .map(|event| {
                    let command = match preset.extras.iter().find(|x| &x.event == event) {
                        Some(x) => extra_command(&dir, name, event, x),
                        None => hook_command(&dir, name, event),
                    };
                    planned(
                        event,
                        event_takes_matcher(event).then_some(matcher.as_str()),
                        command,
                    )
                })
                .collect();
            entries.extend(own_entries(&chosen)?);
            entries
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

pub(super) fn entries_from(event: &str, list: &Value, out: &mut Vec<PlannedHook>) {
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

pub(crate) fn remote_extra_script_path(hook_dir: &str) -> String {
    remote_path_expr(&format!(
        "{}/{EXTRA_WRAPPER}",
        hook_dir.trim_end_matches('/')
    ))
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
