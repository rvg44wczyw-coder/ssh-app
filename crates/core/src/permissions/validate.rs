use super::*;

pub(super) fn shown(rule: &str) -> String {
    crate::approval_queue::display_safe(&rule.chars().take(80).collect::<String>())
}

/// `Tool` or `Tool(specifier)`; returns the trimmed rule.
pub(crate) fn validate_rule(list: &str, rule: &str) -> Result<String, SshCoreError> {
    let r = rule.trim();
    let bad = |why: &str| invalid(format!("Invalid {list} rule \"{}\": {why}", shown(r)));
    if r.is_empty() || r.chars().count() > MAX_RULE {
        return Err(bad("must be 1-400 characters"));
    }
    if r.chars().any(char::is_control) {
        return Err(bad("contains control characters"));
    }
    let (name, spec) = match r.find('(') {
        Some(i) => (&r[..i], Some(&r[i..])),
        None => (r, None),
    };
    let mut chars = name.chars();
    let name_ok = name.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !name_ok {
        return Err(bad("tool name must be letters, digits and _"));
    }
    if let Some(spec) = spec {
        let inner = spec
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .filter(|s| !s.is_empty());
        match inner {
            Some(i) if i.chars().count() <= MAX_SPEC => {}
            _ => {
                return Err(bad(
                    "expected Tool(specifier) with a specifier of 1-300 characters",
                ))
            }
        }
    }
    Ok(r.to_string())
}

pub(super) fn validate_list(list: &str, rules: &[String]) -> Result<Vec<String>, SshCoreError> {
    if rules.len() > MAX_RULES {
        return Err(invalid(format!("Too many {list} rules (max {MAX_RULES})")));
    }
    let mut out: Vec<String> = Vec::with_capacity(rules.len());
    for rule in rules {
        let r = validate_rule(list, rule)?;
        if !out.contains(&r) {
            out.push(r);
        }
    }
    Ok(out)
}

pub(super) fn validate_mode(mode: &str) -> Result<String, SshCoreError> {
    let m = mode.trim();
    if m.is_empty() || m.len() > 32 || !m.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(invalid(format!(
            "Invalid default mode \"{}\": letters only, up to 32",
            shown(m)
        )));
    }
    Ok(m.to_string())
}

pub(super) fn require_claude(agent: AgentKind) -> Result<(), SshCoreError> {
    match agent {
        AgentKind::ClaudeCode => Ok(()),
        other => Err(unsupported_permissions(other)),
    }
}

pub(super) fn unsupported_permissions(agent: AgentKind) -> SshCoreError {
    invalid(format!(
        "Permission rules are only supported for ClaudeCode for now (not {agent:?})"
    ))
}

/// Normalises (trim, dedupe keeping order) and validates; invalid entries are errors, never dropped.
#[uniffi::export]
pub fn validate_agent_permissions(
    agent: AgentKind,
    permissions: AgentPermissions,
) -> Result<AgentPermissions, SshCoreError> {
    require_claude(agent)?;
    Ok(AgentPermissions {
        allow: validate_list("allow", &permissions.allow)?,
        ask: validate_list("ask", &permissions.ask)?,
        deny: validate_list("deny", &permissions.deny)?,
        default_mode: permissions
            .default_mode
            .as_deref()
            .filter(|m| !m.trim().is_empty())
            .map(validate_mode)
            .transpose()?,
    })
}

pub(super) const KNOWN_MODES: [&str; 4] = ["default", "acceptEdits", "plan", "bypassPermissions"];
pub(super) const BROAD: [&str; 3] = ["Bash", "Bash(*)", "Bash(:*)"];

/// Non-blocking warnings shown in the preview.
#[uniffi::export]
pub fn permission_warnings(permissions: AgentPermissions) -> Vec<String> {
    let mut w = vec![];
    for rule in &permissions.allow {
        if permissions.deny.contains(rule) {
            w.push(format!(
                "\"{}\" is in both allow and deny: deny wins, the allow copy does nothing.",
                shown(rule)
            ));
        } else if permissions.ask.contains(rule) {
            w.push(format!(
                "\"{}\" is in both allow and ask: ask takes precedence over allow.",
                shown(rule)
            ));
        }
        if BROAD.contains(&rule.as_str()) {
            w.push(format!(
                "\"{}\" allows every shell command without asking.",
                shown(rule)
            ));
        }
    }
    for rule in &permissions.ask {
        if permissions.deny.contains(rule) {
            w.push(format!(
                "\"{}\" is in both ask and deny: deny wins.",
                shown(rule)
            ));
        }
    }
    match permissions.default_mode.as_deref() {
        Some("bypassPermissions") => w.push(
            "Default mode bypassPermissions: the agent stops asking, so nothing is sent to your phone."
                .into(),
        ),
        Some(m) if !KNOWN_MODES.contains(&m) => w.push(format!(
            "Unknown default mode \"{}\": the agent may ignore it.",
            shown(m)
        )),
        _ => {}
    }
    w
}
