//! Agent skills of a project (feature 53): the request the app sends to the pinned host helper's
//! `skills` call and the typed answer. The helper does the work and owns every path rule; this
//! side only validates what it puts into the request and parses what comes back.

use crate::agents::AgentKind;
use crate::error::SshCoreError;
use serde_json::{json, Value};

/// Largest skill the app sends or opens (the helper enforces the same limit).
pub const MAX_SKILL_BYTES: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SkillSummary {
    pub name: String,
    /// `project` (editable) or `user` (read only).
    pub scope: String,
    pub description: String,
    pub bytes: u64,
    /// Estimated tokens of the whole file (about four bytes each).
    pub tokens: u64,
    pub enabled: bool,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SkillDocument {
    /// The whole `SKILL.md`, front matter included.
    pub content: String,
    /// Other files in the skill's folder (names only).
    pub files: Vec<String>,
}

pub(crate) fn agent_name(agent: AgentKind) -> Result<&'static str, SshCoreError> {
    match agent {
        AgentKind::ClaudeCode => Ok("claude"),
        AgentKind::Antigravity => Ok("antigravity"),
        _ => Err(SshCoreError::InvalidInput {
            reason: "this agent has no skills".into(),
        }),
    }
}

fn bad(reason: &str) -> SshCoreError {
    SshCoreError::InvalidInput {
        reason: reason.into(),
    }
}

/// Skill names as the helper accepts them: lowercase letters, digits and hyphens, up to 64.
pub fn valid_skill_name(n: &str) -> bool {
    let b = n.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// The request line for the helper; every field is checked here as well.
pub(crate) fn build_request(
    op: &str,
    agent: AgentKind,
    project: &str,
    name: Option<&str>,
    extra: Value,
) -> Result<String, SshCoreError> {
    if !project.starts_with('/') || project.chars().any(|c| c == '\0' || c == '\n') {
        return Err(bad("the project folder must be an absolute path"));
    }
    let mut req = json!({"op": op, "agent": agent_name(agent)?, "project": project});
    if let Some(n) = name {
        if op != "restore" && !valid_skill_name(n) {
            return Err(bad(
                "a skill name is lowercase letters, digits and hyphens (up to 64)",
            ));
        }
        req["name"] = json!(n);
    }
    if let (Some(extra), Some(obj)) = (extra.as_object(), req.as_object_mut()) {
        for (k, v) in extra {
            if k == "content" && v.as_str().is_some_and(|c| c.len() > MAX_SKILL_BYTES) {
                return Err(bad("the skill is too large"));
            }
            obj.insert(k.clone(), v.clone());
        }
    }
    Ok(req.to_string())
}

/// The shell command that pipes `request` to the pinned helper's `skills` call.
pub(crate) fn build_command(sha256: &str, request: &str) -> String {
    let script = format!(
        "h={path}; if [ ! -f \"$h\" ] || [ -L \"$h\" ] || [ \"{sha}\" != '{pin}' ]; then echo HELPER_ABSENT; \
         else printf %s {req} | \"$h\" skills 2>/dev/null; fi",
        path = crate::usage::HELPER_PATH_EXPR,
        sha = crate::hostprobe::SHA256_OF_H,
        pin = sha256,
        req = crate::tmux::shell_quote(request),
    );
    crate::hostprobe::login_sh_script(&script, &[], 20)
}

/// The reply object of the helper, or the reason it refused.
pub(crate) fn parse_reply(out: &str) -> Result<Value, SshCoreError> {
    for line in out.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        return match v.get("ok").and_then(Value::as_bool) {
            Some(true) => Ok(v),
            Some(false) => Err(SshCoreError::InvalidInput {
                reason: crate::approval_queue::display_safe(
                    &v.get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("refused")
                        .chars()
                        .take(200)
                        .collect::<String>(),
                ),
            }),
            None => continue,
        };
    }
    Err(SshCoreError::IoError {
        reason: "the host helper is missing or too old: update it from the app's host setup".into(),
    })
}

pub(crate) fn parse_summaries(v: &Value) -> Vec<SkillSummary> {
    let text = |s: &Value, k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    v.get("skills")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|s| SkillSummary {
            name: text(s, "name"),
            scope: text(s, "scope"),
            description: crate::approval_queue::display_safe(&text(s, "description")),
            bytes: s.get("bytes").and_then(Value::as_u64).unwrap_or(0),
            tokens: s.get("tokens").and_then(Value::as_u64).unwrap_or(0),
            enabled: s.get("enabled").and_then(Value::as_bool).unwrap_or(true),
            read_only: s.get("read_only").and_then(Value::as_bool).unwrap_or(true),
        })
        .filter(|s| valid_skill_name(&s.name))
        .collect()
}

pub(crate) fn parse_document(v: &Value) -> SkillDocument {
    SkillDocument {
        content: v
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        files: v
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(crate::approval_queue::display_safe)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn names_and_agents_are_checked_before_anything_is_sent() {
        for n in ["", "../x", "A", "a b", "-x", &"a".repeat(65)] {
            assert!(
                build_request("read", AgentKind::ClaudeCode, "/p", Some(n), json!({})).is_err()
            );
        }
        assert!(build_request("read", AgentKind::Codex, "/p", Some("ok"), json!({})).is_err());
        assert!(build_request("list", AgentKind::ClaudeCode, "rel", None, json!({})).is_err());
        assert!(build_request("list", AgentKind::ClaudeCode, "/p\nx", None, json!({})).is_err());
        let big = "a".repeat(MAX_SKILL_BYTES + 1);
        assert!(build_request(
            "write",
            AgentKind::ClaudeCode,
            "/p",
            Some("ok"),
            json!({"content": big})
        )
        .is_err());
        let ok = build_request(
            "create",
            AgentKind::Antigravity,
            "/p",
            Some("ok"),
            json!({"content": "x"}),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&ok).unwrap();
        assert_eq!(
            (v["agent"].as_str(), v["op"].as_str()),
            (Some("antigravity"), Some("create"))
        );
    }

    #[test]
    fn a_hostile_request_stays_one_quoted_argument() {
        let req = build_request(
            "write",
            AgentKind::ClaudeCode,
            "/p",
            Some("ok"),
            json!({"content": "'; rm -rf ~; echo '"}),
        )
        .unwrap();
        let cmd = build_command(PIN, &req);
        assert!(cmd.contains("HELPER_ABSENT") && cmd.contains("skills"));
        // A real shell hands the request over byte for byte: nothing in it is interpreted.
        let echoed = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf %s {}", crate::tmux::shell_quote(&req)))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&echoed.stdout), req);
    }

    #[test]
    fn replies_are_parsed_and_refusals_are_shown() {
        let ok = parse_reply("noise\n{\"ok\":true,\"skills\":[{\"name\":\"a-b\",\"scope\":\"project\",\"description\":\"d\",\"bytes\":9,\"tokens\":3,\"enabled\":true,\"read_only\":false},{\"name\":\"../x\"}]}\n").unwrap();
        let s = parse_summaries(&ok);
        assert_eq!(s.len(), 1, "a bad name from the host is dropped");
        assert_eq!((s[0].name.as_str(), s[0].tokens), ("a-b", 3));
        let e = parse_reply("{\"ok\":false,\"error\":\"no such skill\"}").unwrap_err();
        assert!(matches!(e, SshCoreError::InvalidInput { reason } if reason == "no such skill"));
        assert!(matches!(
            parse_reply("HELPER_ABSENT").unwrap_err(),
            SshCoreError::IoError { .. }
        ));
    }
}
