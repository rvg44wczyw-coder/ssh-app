//! Permission rules of a coding agent (Claude Code: `permissions.allow|ask|deny|defaultMode` in
//! the profile's `settings.json`), edited from the phone with validation and a preview.
//!
//! Only those four fields are read or written; every other key is preserved and never exposed.

use crate::agents::{invalid, parse_object, render, AgentKind};
use crate::error::SshCoreError;
use serde_json::{json, Map, Value};

const MAX_RULES: usize = 200;
const MAX_RULE: usize = 400;
const MAX_SPEC: usize = 300;

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct AgentPermissions {
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
    /// `None` leaves (or removes) the agent's default mode.
    pub default_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PermissionTemplate {
    pub id: String,
    pub title: String,
    pub description: String,
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Built-in starting points; applying one only adds its rules to the editor.
#[uniffi::export]
pub fn permission_rule_templates() -> Vec<PermissionTemplate> {
    vec![
        PermissionTemplate {
            id: "read-only".into(),
            title: "Read-only tools without asking".into(),
            description: "Reading files and basic git inspection never prompts.".into(),
            allow: strings(&[
                "Read",
                "Glob",
                "Grep",
                "Bash(git status:*)",
                "Bash(git diff:*)",
                "Bash(git log:*)",
                "Bash(ls:*)",
            ]),
            ask: vec![],
            deny: vec![],
        },
        PermissionTemplate {
            id: "ask-destructive".into(),
            title: "Always ask for destructive commands".into(),
            description: "Deleting, pushing, resetting and sudo always need a decision.".into(),
            allow: vec![],
            ask: strings(&[
                "Bash(rm:*)",
                "Bash(git push:*)",
                "Bash(git reset:*)",
                "Bash(sudo:*)",
            ]),
            deny: vec![],
        },
        PermissionTemplate {
            id: "protect-secrets".into(),
            title: "Never read secrets".into(),
            description: "Blocks .env files, SSH keys and key material.".into(),
            allow: vec![],
            ask: vec![],
            deny: strings(&[
                "Read(./.env)",
                "Read(./.env.*)",
                "Read(~/.ssh/**)",
                "Read(**/*.pem)",
                "Read(**/*.key)",
            ]),
        },
    ]
}

fn shown(rule: &str) -> String {
    crate::approval_queue::display_safe(&rule.chars().take(80).collect::<String>())
}

/// `Tool` or `Tool(specifier)`; returns the trimmed rule.
fn validate_rule(list: &str, rule: &str) -> Result<String, SshCoreError> {
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

fn validate_list(list: &str, rules: &[String]) -> Result<Vec<String>, SshCoreError> {
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

fn validate_mode(mode: &str) -> Result<String, SshCoreError> {
    let m = mode.trim();
    if m.is_empty() || m.len() > 32 || !m.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(invalid(format!(
            "Invalid default mode \"{}\": letters only, up to 32",
            shown(m)
        )));
    }
    Ok(m.to_string())
}

fn require_claude(agent: AgentKind) -> Result<(), SshCoreError> {
    match agent {
        AgentKind::ClaudeCode => Ok(()),
        other => Err(unsupported_permissions(other)),
    }
}

fn unsupported_permissions(agent: AgentKind) -> SshCoreError {
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

const KNOWN_MODES: [&str; 4] = ["default", "acceptEdits", "plan", "bypassPermissions"];
const BROAD: [&str; 3] = ["Bash", "Bash(*)", "Bash(:*)"];

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

fn list_from(map: &Map<String, Value>, list: &str) -> Result<Vec<String>, SshCoreError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn p(allow: &[&str], ask: &[&str], deny: &[&str], mode: Option<&str>) -> AgentPermissions {
        AgentPermissions {
            allow: strings(allow),
            ask: strings(ask),
            deny: strings(deny),
            default_mode: mode.map(str::to_string),
        }
    }
    const C: AgentKind = AgentKind::ClaudeCode;

    #[test]
    fn rule_syntax_is_validated_and_never_silently_dropped() {
        for ok in [
            "Read",
            "Bash(git push:*)",
            "WebFetch(domain:example.com)",
            "mcp__srv__tool",
            "Read(~/.ssh/**)",
            "Bash(echo (a))",
        ] {
            assert!(validate_rule("allow", ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            " ",
            "Bash(",
            "Bash()",
            "Bash(x",
            "(x)",
            "1Bash",
            "Ba sh",
            "Bash(x)y",
            "Bash(a\nb)",
            "Bash(a\u{0}b)",
            "*",
            "Bash;rm",
        ] {
            assert!(validate_rule("allow", bad).is_err(), "{bad:?}");
        }
        assert!(validate_rule("allow", &format!("Bash({})", "a".repeat(301))).is_err());
        assert!(validate_rule("allow", &"A".repeat(65)).is_err());
        let err =
            validate_agent_permissions(C, p(&["Read", "bad rule"], &[], &[], None)).unwrap_err();
        assert!(format!("{err}").contains("bad rule"), "{err}");
        let many = AgentPermissions {
            allow: (0..201).map(|i| format!("T{i}")).collect(),
            ..Default::default()
        };
        assert!(validate_agent_permissions(C, many).is_err());
    }

    #[test]
    fn normalises_dedupes_and_checks_modes() {
        let v =
            validate_agent_permissions(C, p(&[" Read ", "Read", "Grep"], &[], &[], Some(" plan ")))
                .unwrap();
        assert_eq!(v.allow, ["Read", "Grep"]);
        assert_eq!(v.default_mode.as_deref(), Some("plan"));
        assert_eq!(
            validate_agent_permissions(C, p(&[], &[], &[], Some("  ")))
                .unwrap()
                .default_mode,
            None
        );
        assert!(validate_agent_permissions(C, p(&[], &[], &[], Some("a b"))).is_err());
        assert!(validate_agent_permissions(C, p(&[], &[], &[], Some("x\ny"))).is_err());
        assert!(
            validate_agent_permissions(AgentKind::Antigravity, p(&[], &[], &[], None)).is_err()
        );
    }

    #[test]
    fn warnings_cover_conflicts_broad_rules_and_modes() {
        let w = permission_warnings(p(
            &["Bash", "Read", "A"],
            &["A"],
            &["Read"],
            Some("bypassPermissions"),
        ));
        let all = w.join("\n");
        assert!(all.contains("allows every shell command"), "{all}");
        assert!(all.contains("both allow and deny"), "{all}");
        assert!(all.contains("both allow and ask"), "{all}");
        assert!(all.contains("bypassPermissions"), "{all}");
        assert!(permission_warnings(p(&["Read"], &[], &[], Some("plan"))).is_empty());
        assert!(permission_warnings(p(&[], &[], &[], Some("zzz")))
            .join("")
            .contains("Unknown default mode"));
        assert!(permission_warnings(p(&[], &["X"], &["X"], None))
            .join("")
            .contains("ask and deny"));
    }

    #[test]
    fn merge_preserves_everything_else_and_removes_empties() {
        let existing = r#"{"env":{"API_KEY":"sk-secret"},"model":"opus","permissions":{"allow":["Old"],"additionalDirectories":["../x"],"futureKey":1},"hooks":{"Stop":[]}}"#;
        let out = merge_permissions(
            C,
            existing.into(),
            p(&["Read"], &[], &["Read(./.env)"], Some("acceptEdits")),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["env"]["API_KEY"], "sk-secret");
        assert_eq!(v["model"], "opus");
        assert_eq!(v["hooks"]["Stop"], json!([]));
        assert_eq!(v["permissions"]["allow"], json!(["Read"]));
        assert_eq!(v["permissions"]["deny"], json!(["Read(./.env)"]));
        assert_eq!(v["permissions"]["defaultMode"], "acceptEdits");
        assert_eq!(v["permissions"]["additionalDirectories"], json!(["../x"]));
        assert_eq!(v["permissions"]["futureKey"], 1);
        assert!(v["permissions"].get("ask").is_none());
        // Emptying everything removes our keys, keeps the foreign ones, then the object itself.
        let cleared = merge_permissions(C, out.clone(), AgentPermissions::default()).unwrap();
        let v: Value = serde_json::from_str(&cleared).unwrap();
        assert_eq!(
            v["permissions"],
            json!({"additionalDirectories":["../x"],"futureKey":1})
        );
        let only = merge_permissions(
            C,
            r#"{"permissions":{"allow":["A"]}}"#.into(),
            AgentPermissions::default(),
        )
        .unwrap();
        assert!(serde_json::from_str::<Value>(&only)
            .unwrap()
            .get("permissions")
            .is_none());
        // A missing/empty file becomes a fresh object.
        let fresh = merge_permissions(C, "".into(), p(&["Read"], &[], &[], None)).unwrap();
        assert!(fresh.ends_with("}\n") && fresh.contains("\"Read\""));
    }

    #[test]
    fn merge_and_read_refuse_unsafe_files() {
        let np = p(&["Read"], &[], &[], None);
        for bad in [
            "not json",
            "[1]",
            r#"{"permissions":[]}"#,
            r#"{"permissions":{},"permissions":{}}"#,
            r#"{"a":{"x":1,"x":2}}"#,
        ] {
            assert!(
                merge_permissions(C, bad.into(), np.clone()).is_err(),
                "{bad}"
            );
        }
        for bad in [
            r#"{"permissions":{"allow":"Read"}}"#,
            r#"{"permissions":{"allow":[1]}}"#,
            r#"{"permissions":{"allow":["bad rule"]}}"#,
            r#"{"permissions":{"deny":["Read\n(x)"]}}"#,
            r#"{"permissions":{"defaultMode":3}}"#,
            r#"{"permissions":{"defaultMode":"a b"}}"#,
            r#"{"permissions":5}"#,
        ] {
            assert!(permissions_from_settings(C, bad.into()).is_err(), "{bad}");
        }
        assert!(permissions_from_settings(AgentKind::Codex, "{}".into()).is_err());
        assert!(merge_permissions(AgentKind::Grok, "{}".into(), np).is_err());
    }

    #[test]
    fn reads_the_four_fields_and_shows_only_the_permissions_section() {
        let json = r#"{"env":{"K":"secret"},"permissions":{"allow":["Read"," Grep "],"ask":["Bash(rm:*)"],"deny":[],"defaultMode":"plan"}}"#;
        let got = permissions_from_settings(C, json.into()).unwrap();
        assert_eq!(
            got,
            p(&["Read", "Grep"], &["Bash(rm:*)"], &[], Some("plan"))
        );
        assert_eq!(
            permissions_from_settings(C, "".into()).unwrap(),
            AgentPermissions::default()
        );
        assert_eq!(
            permissions_from_settings(C, "{}".into()).unwrap(),
            AgentPermissions::default()
        );
        let section = permissions_section_of(C, json.into()).unwrap();
        assert!(
            section.contains("\"allow\"")
                && !section.contains("secret")
                && !section.contains("env")
        );
        assert_eq!(permissions_section_of(C, "{}".into()).unwrap(), "{}");
    }

    #[test]
    fn templates_are_valid_rules() {
        for t in permission_rule_templates() {
            let v = validate_agent_permissions(
                C,
                AgentPermissions {
                    allow: t.allow.clone(),
                    ask: t.ask.clone(),
                    deny: t.deny.clone(),
                    default_mode: None,
                },
            )
            .unwrap();
            assert_eq!(
                (v.allow, v.ask, v.deny),
                (t.allow, t.ask, t.deny),
                "{}",
                t.id
            );
            assert!(!t.title.is_empty() && !t.description.is_empty());
        }
    }
}
