use super::*;

pub(super) fn list_from(map: &Map<String, Value>, list: &str) -> Result<Vec<String>, SshCoreError> {
    let Some(v) = map.get(list) else {
        return Ok(vec![]);
    };
    let arr = v
        .as_array()
        .ok_or_else(|| invalid(format!("permissions.{list} is not an array")))?;
    let mut raw = Vec::with_capacity(arr.len());
    for item in arr {
        raw.push(
            item.as_str()
                .ok_or_else(|| invalid(format!("permissions.{list} contains a non-string entry")))?
                .to_string(),
        );
    }
    // Unlike writing, reading refuses (instead of normalising) so nothing is altered unseen.
    for r in &raw {
        validate_rule(list, r)?;
    }
    Ok(raw.into_iter().map(|r| r.trim().to_string()).collect())
}

/// A rule found in the file that cannot be written back (shown escaped so a newline is visible).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InvalidPermissionRule {
    pub list: String,
    pub rule: String,
    pub reason: String,
}

/// The valid rules of the file plus the ones the editor cannot represent. Applying the editor
/// drops the invalid ones, so the caller must show them and ask first.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CheckedPermissions {
    pub permissions: AgentPermissions,
    pub invalid: Vec<InvalidPermissionRule>,
}

pub(super) fn escaped(rule: &str) -> String {
    let mut out = String::new();
    for c in rule.chars().take(200) {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:04x}}}", c as u32)),
            c => out.push(c),
        }
    }
    crate::approval_queue::display_safe(&out)
}

/// Like `permissions_from_settings`, but a rule that is not valid is reported instead of failing
/// the whole read, so a file with one bad entry can still be opened and repaired.
#[uniffi::export]
pub fn permissions_from_settings_checked(
    agent: AgentKind,
    config_json: String,
) -> Result<CheckedPermissions, SshCoreError> {
    require_claude(agent)?;
    let root = parse_object(&config_json)?;
    let Some(p) = root.get("permissions") else {
        return Ok(CheckedPermissions {
            permissions: AgentPermissions::default(),
            invalid: vec![],
        });
    };
    let map = p
        .as_object()
        .ok_or_else(|| invalid("`permissions` is not an object; refusing to read it"))?;
    let default_mode = match map.get("defaultMode") {
        None => None,
        Some(Value::String(m)) => Some(validate_mode(m)?),
        Some(_) => return Err(invalid("permissions.defaultMode is not a string")),
    };
    let mut bad = vec![];
    let mut lists: Vec<Vec<String>> = vec![];
    for list in ["allow", "ask", "deny"] {
        let mut good = vec![];
        if let Some(v) = map.get(list) {
            let arr = v
                .as_array()
                .ok_or_else(|| invalid(format!("permissions.{list} is not an array")))?;
            for item in arr {
                let r = item.as_str().ok_or_else(|| {
                    invalid(format!("permissions.{list} contains a non-string entry"))
                })?;
                match validate_rule(list, r) {
                    Ok(_) => good.push(r.trim().to_string()),
                    Err(_) => bad.push(InvalidPermissionRule {
                        list: list.to_string(),
                        rule: escaped(r),
                        reason: if r.chars().any(char::is_control) {
                            "contains control characters (for example a line break)".into()
                        } else {
                            "not a valid rule".into()
                        },
                    }),
                }
            }
        }
        lists.push(good);
    }
    let deny = lists.pop().unwrap_or_default();
    let ask = lists.pop().unwrap_or_default();
    let allow = lists.pop().unwrap_or_default();
    Ok(CheckedPermissions {
        permissions: AgentPermissions {
            allow,
            ask,
            deny,
            default_mode,
        },
        invalid: bad,
    })
}

/// Reads the four permission fields. Unparsable JSON, duplicate keys, wrong types and invalid
/// rules are errors (the editor replaces whole lists, so it must not load what it cannot save).
#[uniffi::export]
pub fn permissions_from_settings(
    agent: AgentKind,
    config_json: String,
) -> Result<AgentPermissions, SshCoreError> {
    require_claude(agent)?;
    let root = parse_object(&config_json)?;
    let Some(p) = root.get("permissions") else {
        return Ok(AgentPermissions::default());
    };
    let map = p
        .as_object()
        .ok_or_else(|| invalid("`permissions` is not an object; refusing to read it"))?;
    let default_mode = match map.get("defaultMode") {
        None => None,
        Some(Value::String(m)) => Some(validate_mode(m)?),
        Some(_) => return Err(invalid("permissions.defaultMode is not a string")),
    };
    Ok(AgentPermissions {
        allow: list_from(map, "allow")?,
        ask: list_from(map, "ask")?,
        deny: list_from(map, "deny")?,
        default_mode,
    })
}

/// New settings text with only the four fields replaced; empty lists remove their key and an
/// empty `permissions` object is removed. All other keys are preserved.
#[uniffi::export]
pub fn merge_permissions(
    agent: AgentKind,
    existing: String,
    permissions: AgentPermissions,
) -> Result<String, SshCoreError> {
    let new = validate_agent_permissions(agent, permissions)?;
    let mut root = parse_object(&existing)?;
    let mut section = match root.remove("permissions") {
        None => Map::new(),
        Some(Value::Object(m)) => m,
        Some(_) => {
            return Err(invalid(
                "`permissions` is not an object; refusing to modify it",
            ))
        }
    };
    for (key, rules) in [
        ("allow", &new.allow),
        ("ask", &new.ask),
        ("deny", &new.deny),
    ] {
        if rules.is_empty() {
            section.remove(key);
        } else {
            section.insert(key.into(), json!(rules));
        }
    }
    match &new.default_mode {
        Some(m) => {
            section.insert("defaultMode".into(), json!(m));
        }
        None => {
            section.remove("defaultMode");
        }
    }
    if !section.is_empty() {
        root.insert("permissions".into(), Value::Object(section));
    }
    render(root)
}

/// Only the `permissions` object, pretty-printed, so the preview never shows `env` or API keys.
#[uniffi::export]
pub fn permissions_section_of(
    agent: AgentKind,
    config_json: String,
) -> Result<String, SshCoreError> {
    require_claude(agent)?;
    let root = parse_object(&config_json)?;
    let section = root
        .get("permissions")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    serde_json::to_string_pretty(&section)
        .map_err(|e| invalid(format!("Failed to serialize permissions: {e}")))
}
