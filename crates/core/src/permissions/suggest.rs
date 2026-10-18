//! Suggestions: what to tighten or loosen in a rule set, computed from the rules in effect.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SuggestionKind {
    /// Safe to ignore; makes the setup nicer.
    Improve,
    /// The current setup is risky or does not do what it seems to.
    Risk,
}

/// One proposed change to the editor state, derived from what is already installed. Nothing is
/// written until the user applies the editor.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PermissionSuggestion {
    pub id: String,
    pub kind: SuggestionKind,
    pub title: String,
    pub detail: String,
    pub add_allow: Vec<String>,
    pub add_ask: Vec<String>,
    pub add_deny: Vec<String>,
    pub remove_allow: Vec<String>,
    pub default_mode: Option<String>,
}

/// Rules shown from a file we did not write (project settings): tolerant of anything, never an
/// error, only sane strings, capped. Returns `None` for text that is not a JSON object.
pub fn lenient_permissions(text: &str) -> Option<AgentPermissions> {
    let root: Value = serde_json::from_str(text).ok()?;
    let perms = root.get("permissions")?.as_object()?;
    let list = |key: &str| -> Vec<String> {
        perms
            .get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|r| !r.is_empty() && r.chars().count() <= MAX_RULE)
                    .filter(|r| !r.chars().any(char::is_control))
                    .take(MAX_RULES)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mode = perms
        .get("defaultMode")
        .and_then(Value::as_str)
        .filter(|m| KNOWN_MODES.contains(m))
        .map(str::to_string);
    Some(AgentPermissions {
        allow: list("allow"),
        ask: list("ask"),
        deny: list("deny"),
        default_mode: mode,
    })
}

/// `Bash(git status:*)` is covered by `Bash(git:*)`; both must be prefix rules of the same tool.
fn covered_by(rule: &str, broader: &str) -> bool {
    let prefix = |r: &str| -> Option<(String, String)> {
        let (tool, rest) = r.split_once('(')?;
        let spec = rest.strip_suffix(":*)")?;
        Some((tool.to_string(), spec.to_string()))
    };
    match (prefix(rule), prefix(broader)) {
        (Some((t1, s1)), Some((t2, s2))) => {
            t1 == t2 && s1 != s2 && s1.starts_with(&format!("{s2} "))
        }
        _ => false,
    }
}

/// Looks at what is installed (`current`, the profile's own settings) together with what the
/// project adds (`context`, read-only: shared and local project settings) and proposes changes:
/// a full "work in the project" setup, smaller alternatives, and fixes for risky or dead rules.
#[uniffi::export]
pub fn suggest_permission_changes(
    current: AgentPermissions,
    context: Vec<AgentPermissions>,
) -> Vec<PermissionSuggestion> {
    let all = |pick: fn(&AgentPermissions) -> &Vec<String>| -> Vec<String> {
        let mut v = pick(&current).clone();
        for c in &context {
            v.extend(pick(c).iter().cloned());
        }
        v
    };
    let (allow, ask, deny) = (all(|p| &p.allow), all(|p| &p.ask), all(|p| &p.deny));
    let mode = std::iter::once(&current.default_mode)
        .chain(context.iter().map(|c| &c.default_mode))
        .flatten()
        .last()
        .cloned()
        .or_else(|| current.default_mode.clone());
    let missing = |have: &[String], want: &[String]| -> Vec<String> {
        want.iter().filter(|r| !have.contains(r)).cloned().collect()
    };
    let template = |id: &str| {
        permission_rule_templates()
            .into_iter()
            .find(|t| t.id == id)
            .expect("built-in template")
    };
    let mut out = vec![];
    let bypass = mode.as_deref() == Some("bypassPermissions");

    if bypass {
        out.push(PermissionSuggestion {
            id: "unattended".into(),
            kind: SuggestionKind::Risk,
            title: "Stop running unattended".into(),
            detail: "The default mode is bypassPermissions: every tool runs without asking and nothing reaches your phone. Accept edits keeps file edits automatic but lets rules and approvals decide the rest.".into(),
            add_allow: vec![], add_ask: vec![], add_deny: vec![], remove_allow: vec![],
            default_mode: Some("acceptEdits".into()),
        });
    }

    let broad: Vec<String> = current
        .allow
        .iter()
        .filter(|r| BROAD.contains(&r.as_str()) || r.as_str() == "*")
        .cloned()
        .collect();
    if !broad.is_empty() {
        out.push(PermissionSuggestion {
            id: "broad-allow".into(),
            kind: SuggestionKind::Risk,
            title: "Replace the catch-all command rule".into(),
            detail: "A rule like Bash allows every shell command. Swap it for the build, test and git commands you actually use.".into(),
            add_allow: missing(&allow, &template("project-work").allow),
            add_ask: vec![], add_deny: vec![],
            remove_allow: broad,
            default_mode: None,
        });
    }

    let work = template("project-work");
    let (a, k, d) = (
        missing(&allow, &work.allow),
        missing(&ask, &work.ask),
        missing(&deny, &work.deny),
    );
    let mode_wanted = !bypass && mode.as_deref() != Some("acceptEdits");
    if !(a.is_empty() && k.is_empty() && d.is_empty() && !mode_wanted) {
        let n = a.len() + k.len() + d.len();
        let nothing = n == work.allow.len() + work.ask.len() + work.deny.len();
        out.push(PermissionSuggestion {
            id: "project-work".into(),
            kind: SuggestionKind::Improve,
            title: if nothing {
                work.title.clone()
            } else {
                format!("Complete \"{}\" ({n} rules missing)", work.title)
            },
            detail: format!(
                "Accept edits in the folder the session started in, run the listed dev commands without asking, keep push/delete/sudo as questions and secrets blocked.{}",
                if context.is_empty() { "" } else { " Rules already coming from the project are not repeated." }
            ),
            add_allow: a,
            add_ask: k,
            add_deny: d,
            remove_allow: vec![],
            default_mode: if mode_wanted { work.default_mode.clone() } else { None },
        });
    }

    // Smaller alternatives to the full set.
    let secrets = missing(&deny, &template("protect-secrets").deny);
    if !secrets.is_empty() {
        out.push(PermissionSuggestion {
            id: "secrets".into(),
            kind: SuggestionKind::Improve,
            title: "Only block secrets".into(),
            detail: "Smaller step: .env files, SSH keys and key material are never read.".into(),
            add_allow: vec![],
            add_ask: vec![],
            add_deny: secrets,
            remove_allow: vec![],
            default_mode: None,
        });
    }
    let config = missing(&deny, &template("protect-agent-config").deny);
    if !config.is_empty() {
        out.push(PermissionSuggestion {
            id: "agent-config".into(),
            kind: SuggestionKind::Improve,
            title: "Protect agent and hook settings".into(),
            detail: "The agent can no longer edit its own settings or the hook files, so it cannot switch off phone approval.".into(),
            add_allow: vec![],
            add_ask: vec![],
            add_deny: config,
            remove_allow: vec![],
            default_mode: None,
        });
    }
    let destructive = missing(&ask, &template("ask-destructive").ask);
    if !destructive.is_empty() && !allow.iter().any(|r| BROAD.contains(&r.as_str())) {
        out.push(PermissionSuggestion {
            id: "destructive".into(),
            kind: SuggestionKind::Improve,
            title: "Only ask before destructive commands".into(),
            detail: "Smaller step: rm, git push, git reset and sudo always need a decision.".into(),
            add_allow: vec![],
            add_ask: destructive,
            add_deny: vec![],
            remove_allow: vec![],
            default_mode: None,
        });
    }
    let reads = missing(&allow, &template("read-only").allow);
    if !reads.is_empty() && !bypass {
        out.push(PermissionSuggestion {
            id: "read-only".into(),
            kind: SuggestionKind::Improve,
            title: "Only allow reading".into(),
            detail: "Smaller step: reading files and git inspection never prompts; edits and other commands still ask.".into(),
            add_allow: reads, add_ask: vec![], add_deny: vec![], remove_allow: vec![], default_mode: None,
        });
    }

    // Clean-up of what is already there.
    let redundant: Vec<String> = current
        .allow
        .iter()
        .filter(|r| allow.iter().any(|b| covered_by(r, b)))
        .cloned()
        .collect();
    let dead: Vec<String> = current
        .allow
        .iter()
        .filter(|r| deny.contains(r) || ask.contains(r))
        .cloned()
        .collect();
    for (id, title, detail, rules) in [
        (
            "redundant",
            "Remove rules a broader rule already covers",
            "These allow rules do nothing: another allow rule matches the same commands.",
            redundant,
        ),
        (
            "dead-allow",
            "Remove allow rules that are overridden",
            "These rules are also in deny or ask, which win, so the allow copy does nothing.",
            dead,
        ),
    ] {
        if !rules.is_empty() {
            out.push(PermissionSuggestion {
                id: id.into(),
                kind: SuggestionKind::Improve,
                title: title.into(),
                detail: detail.into(),
                add_allow: vec![],
                add_ask: vec![],
                add_deny: vec![],
                remove_allow: rules,
                default_mode: None,
            });
        }
    }
    out
}
