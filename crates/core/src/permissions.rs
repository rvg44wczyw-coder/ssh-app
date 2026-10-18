//! Permission rules of a coding agent (Claude Code: `permissions.allow|ask|deny|defaultMode` in
//! the profile's `settings.json`), edited from the phone with validation and a preview.
//!
//! Only those four fields are read or written; every other key is preserved and never exposed.

use crate::agents::{invalid, parse_object, render, AgentKind};
use crate::error::SshCoreError;
use serde_json::{json, Map, Value};

mod suggest;

pub use suggest::*;

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
    /// Default mode the template also sets (applied to the editor like the rules).
    pub default_mode: Option<String>,
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Files that decide what the agent may do and who is asked: its own settings, the hook configs
/// of every supported agent and the app's folder. An agent that can edit them can switch off the
/// phone approval, so they are denied for edits. (A shell command can still write them; the
/// `Bash` rules and the approval itself are the guard for that.)
const AGENT_CONFIG_DENY: &[&str] = &[
    "Edit(~/.claude/settings.json)",
    "Edit(~/.claude/settings.local.json)",
    "Edit(./.claude/settings.json)",
    "Edit(./.claude/settings.local.json)",
    "Edit(~/.gemini/config/hooks.json)",
    "Edit(~/.codex/hooks.json)",
    "Edit(~/.grok/hooks/**)",
    "Edit(~/.ssh-app/**)",
];

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
            default_mode: None,
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
            default_mode: None,
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
            default_mode: None,
        },
        PermissionTemplate {
            id: "protect-agent-config".into(),
            title: "Protect agent and hook settings".into(),
            description: "The agent cannot edit its own settings, the hook files of Claude Code, \
                          Antigravity, Codex and Grok, or the app's folder, so it cannot turn off \
                          phone approval."
                .into(),
            allow: vec![],
            ask: vec![],
            deny: strings(AGENT_CONFIG_DENY),
            default_mode: None,
        },
        PermissionTemplate {
            id: "project-work".into(),
            title: "Work in the project (accept edits + dev tools)".into(),
            description: "Edits inside the folder the session started in are accepted without asking, \
                          and the listed build/test/git commands run without asking. Pushing, deleting, \
                          sudo and secrets still need a decision or stay blocked."
                .into(),
            allow: strings(&[
                "Read",
                "Glob",
                "Grep",
                "Bash(git status:*)",
                "Bash(git diff:*)",
                "Bash(git log:*)",
                "Bash(git add:*)",
                "Bash(git commit:*)",
                "Bash(git switch:*)",
                "Bash(git branch:*)",
                "Bash(ls:*)",
                "Bash(make:*)",
                "Bash(cargo build:*)",
                "Bash(cargo test:*)",
                "Bash(cargo fmt:*)",
                "Bash(cargo clippy:*)",
                "Bash(npm run:*)",
                "Bash(npm test:*)",
                "Bash(xcodebuild:*)",
            ]),
            ask: strings(&[
                "Bash(rm:*)",
                "Bash(git push:*)",
                "Bash(git reset:*)",
                "Bash(sudo:*)",
            ]),
            deny: strings(&[
                "Read(./.env)",
                "Read(./.env.*)",
                "Read(~/.ssh/**)",
                "Read(**/*.pem)",
                "Read(**/*.key)",
            ])
            .into_iter()
            .chain(strings(AGENT_CONFIG_DENY))
            .collect(),
            default_mode: Some("acceptEdits".into()),
        },
    ]
}

fn shown(rule: &str) -> String {
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

/// What is in effect for an open session: its folder, the rules from the profile's settings and
/// from the folder's `.claude/settings*.json`, and the suggestions computed from them.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SessionPermissionsReport {
    /// The session's working folder (tmux pane), if the session exists.
    pub directory: Option<String>,
    /// `CLAUDE_CONFIG_DIR` of the Claude process running in this session, when it has one.
    pub detected_config_dir: Option<String>,
    pub user: Option<AgentPermissions>,
    pub project_shared: Option<AgentPermissions>,
    pub project_local: Option<AgentPermissions>,
    pub suggestions: Vec<PermissionSuggestion>,
}

const INSPECT_FILE_CAP: usize = 200_000;

/// One exec that prints the pane's folder and the three settings files between markers. The
/// folder is only used when it has no control characters; files are read, never written, and only
/// when they are regular files (a FIFO planted as a project's settings file would block `head`).
pub(crate) fn build_inspect_permissions_command(
    session: &str,
    user_file_expr: Option<&str>,
    tag: &str,
) -> String {
    let sess = crate::tmux::shell_quote(&format!("={session}:"));
    // Without an explicit profile folder, the running Claude's own CLAUDE_CONFIG_DIR is used
    // (read from the environment of its process in this pane), else the default ~/.claude.
    let user_file = match user_file_expr {
        Some(expr) => format!("uf={expr}"),
        None => "uf=\"${cfg:-$HOME/.claude}/settings.json\"".to_string(),
    };
    // The section markers carry a per-call random tag: the settings files (a project's file is
    // untrusted) cannot contain a marker they could not know in advance.
    format!(
        "PATH=\"/opt/homebrew/bin:/usr/local/bin:$PATH\"; export PATH; \
         p=$(tmux display-message -p -t {sess} '#{{pane_current_path}}' 2>/dev/null); \
         case \"$p\" in *[![:print:]]*) p='';; esac; \
         cfg=''; pp=$(tmux display-message -p -t {sess} '#{{pane_pid}}' 2>/dev/null); \
         if [ -n \"$pp\" ]; then \
           for pid in $(ps -Ao pid=,ppid= | awk -v root=\"$pp\" '{{par[$1]=$2}} END{{for(x in par){{q=x;n=0;while((q in par)&&q!=root&&n<50){{q=par[q];n++}} if(q==root)print x}}}}'); do \
             c=$(ps -p \"$pid\" -o comm= 2>/dev/null); \
             case \"${{c##*/}}\" in claude*) ;; *) continue;; esac; \
             full=$(ps eww -p \"$pid\" -o command= 2>/dev/null); av=$(ps -p \"$pid\" -o command= 2>/dev/null); \
             env_part=${{full#\"$av\"}}; \
             v=$(printf '%s\\n' \"$env_part\" | tr ' ' '\\n' | sed -n 's/^CLAUDE_CONFIG_DIR=//p' | head -n 1); \
             case \"$v\" in /*) [ -d \"$v\" ] && {{ cfg=$v; break; }};; esac; \
           done; \
         fi; \
         case \"$cfg\" in *[![:print:]]*) cfg='';; esac; \
         {user_file}; \
         printf '==={tag}:DIR===\\n%s\\n==={tag}:CFG===\\n%s\\n==={tag}:USER===\\n' \"$p\" \"$cfg\"; \
         if [ -f \"$uf\" ]; then head -c {INSPECT_FILE_CAP} \"$uf\" 2>/dev/null; fi; \
         printf '\\n==={tag}:SHARED===\\n'; \
         if [ -f \"$p/.claude/settings.json\" ]; then head -c {INSPECT_FILE_CAP} \"$p/.claude/settings.json\" 2>/dev/null; fi; \
         printf '\\n==={tag}:LOCAL===\\n'; \
         if [ -f \"$p/.claude/settings.local.json\" ]; then head -c {INSPECT_FILE_CAP} \"$p/.claude/settings.local.json\" 2>/dev/null; fi; \
         printf '\\n==={tag}:END===\\n'"
    )
}

/// A fresh unguessable marker tag for one inspect call.
pub(crate) fn new_inspect_tag() -> String {
    format!(
        "{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    )
}

pub(crate) fn parse_inspect_permissions_output(out: &str, tag: &str) -> SessionPermissionsReport {
    let section = |name: &str, next: &str| -> String {
        let start = format!("==={tag}:{name}===\n");
        let Some(i) = out.find(&start) else {
            return String::new();
        };
        let rest = &out[i + start.len()..];
        let end = rest
            .find(&format!("==={tag}:{next}==="))
            .unwrap_or(rest.len());
        rest[..end].trim().to_string()
    };
    let dir = section("DIR", "CFG");
    let cfg = section("CFG", "USER");
    let user = lenient_permissions(&section("USER", "SHARED"));
    let shared = lenient_permissions(&section("SHARED", "LOCAL"));
    let local = lenient_permissions(&section("LOCAL", "END"));
    let current = user.clone().unwrap_or_default();
    let context: Vec<AgentPermissions> = shared.iter().chain(local.iter()).cloned().collect();
    SessionPermissionsReport {
        directory: Some(dir).filter(|d| !d.is_empty() && !d.chars().any(char::is_control)),
        detected_config_dir: Some(cfg).filter(|d| {
            !d.is_empty() && d.chars().count() <= 300 && !d.chars().any(char::is_control)
        }),
        suggestions: suggest_permission_changes(current, context),
        user,
        project_shared: shared,
        project_local: local,
    }
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

fn escaped(rule: &str) -> String {
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
    fn agent_config_is_protected_by_template_and_project_work() {
        let all = permission_rule_templates();
        let guard = all.iter().find(|t| t.id == "protect-agent-config").unwrap();
        for must in [
            "Edit(~/.claude/settings.json)",
            "Edit(~/.gemini/config/hooks.json)",
            "Edit(~/.codex/hooks.json)",
            "Edit(~/.grok/hooks/**)",
            "Edit(~/.ssh-app/**)",
        ] {
            assert!(guard.deny.contains(&must.to_string()), "{must}");
        }
        assert!(guard.allow.is_empty() && guard.ask.is_empty());
        let work = all.iter().find(|t| t.id == "project-work").unwrap();
        for rule in &guard.deny {
            assert!(work.deny.contains(rule), "project-work lacks {rule}");
        }
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
                    default_mode: t.default_mode.clone(),
                },
            )
            .unwrap();
            assert_eq!(v.default_mode, t.default_mode, "{}", t.id);
            assert_eq!(
                (v.allow, v.ask, v.deny),
                (t.allow, t.ask, t.deny),
                "{}",
                t.id
            );
            assert!(!t.title.is_empty() && !t.description.is_empty());
        }
    }

    #[test]
    fn suggestions_follow_what_is_installed() {
        let none = AgentPermissions::default();
        let ids = |p: AgentPermissions, ctx: Vec<AgentPermissions>| {
            suggest_permission_changes(p, ctx)
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
        };
        // Empty setup: the full set plus the smaller alternatives.
        let fresh = ids(none.clone(), vec![]);
        for id in [
            "project-work",
            "secrets",
            "agent-config",
            "destructive",
            "read-only",
        ] {
            assert!(fresh.contains(&id.to_string()), "{id}: {fresh:?}");
        }
        // Everything from the template already installed: nothing left to suggest.
        let work = permission_rule_templates()
            .into_iter()
            .find(|t| t.id == "project-work")
            .unwrap();
        let done = AgentPermissions {
            allow: work.allow.clone(),
            ask: work.ask.clone(),
            deny: work.deny.clone(),
            default_mode: work.default_mode.clone(),
        };
        assert!(
            ids(done.clone(), vec![]).is_empty(),
            "{:?}",
            ids(done.clone(), vec![])
        );
        // The same rules arriving from the project are not repeated, only the mode is missing.
        let from_project = AgentPermissions {
            default_mode: None,
            ..done.clone()
        };
        let s = suggest_permission_changes(none.clone(), vec![from_project]);
        let pw = s
            .iter()
            .find(|s| s.id == "project-work")
            .expect("mode suggestion");
        assert!(pw.add_allow.is_empty() && pw.add_ask.is_empty() && pw.add_deny.is_empty());
        assert_eq!(pw.default_mode.as_deref(), Some("acceptEdits"));
        // Risky and dead rules.
        let risky = AgentPermissions {
            allow: vec![
                "Bash".into(),
                "Bash(git:*)".into(),
                "Bash(git status:*)".into(),
                "Bash(rm:*)".into(),
            ],
            ask: vec!["Bash(rm:*)".into()],
            deny: vec![],
            default_mode: Some("bypassPermissions".into()),
        };
        let s = suggest_permission_changes(risky, vec![]);
        let get = |id: &str| {
            s.iter()
                .find(|x| x.id == id)
                .unwrap_or_else(|| panic!("{id}"))
        };
        assert_eq!(
            get("unattended").default_mode.as_deref(),
            Some("acceptEdits")
        );
        assert_eq!(get("broad-allow").remove_allow, vec!["Bash".to_string()]);
        assert_eq!(
            get("redundant").remove_allow,
            vec!["Bash(git status:*)".to_string()]
        );
        assert_eq!(
            get("dead-allow").remove_allow,
            vec!["Bash(rm:*)".to_string()]
        );
        // Every suggested rule is itself valid.
        for s in suggest_permission_changes(none, vec![]) {
            validate_agent_permissions(
                C,
                AgentPermissions {
                    allow: s.add_allow,
                    ask: s.add_ask,
                    deny: s.add_deny,
                    default_mode: s.default_mode,
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn lenient_reader_keeps_sane_rules_and_ignores_the_rest() {
        let p = lenient_permissions(r#"{"permissions":{"allow":["Read","a\nb",5,"Bash(ls:*)"],"defaultMode":"nonsense"},"env":{"K":"secret"}}"#).unwrap();
        assert_eq!(p.allow, vec!["Read".to_string(), "Bash(ls:*)".to_string()]);
        assert_eq!(p.default_mode, None);
        assert!(lenient_permissions("not json").is_none());
        assert!(lenient_permissions("{}").is_none());
    }

    #[test]
    fn inspect_output_is_split_into_the_report() {
        let out = "==={T}:DIR===\n/work/app\n==={T}:CFG===\n/home/u/.claude-work\n==={T}:USER===\n{\"permissions\":{\"allow\":[\"Read\"],\"defaultMode\":\"plan\"}}\n==={T}:SHARED===\n\n==={T}:LOCAL===\n{\"permissions\":{\"allow\":[\"Bash(ls:*)\"]}}\n==={T}:END===\n".replace("{T}", "tag1");
        let r = parse_inspect_permissions_output(&out, "tag1");
        assert_eq!(r.directory.as_deref(), Some("/work/app"));
        assert_eq!(
            r.detected_config_dir.as_deref(),
            Some("/home/u/.claude-work")
        );
        assert_eq!(
            r.user.as_ref().unwrap().default_mode.as_deref(),
            Some("plan")
        );
        assert!(r.project_shared.is_none());
        assert_eq!(
            r.project_local.unwrap().allow,
            vec!["Bash(ls:*)".to_string()]
        );
        assert!(!r.suggestions.is_empty());
        // Nothing found at all still yields a report (no folder, no files).
        let empty = parse_inspect_permissions_output("", "tag1");
        assert!(empty.directory.is_none() && empty.user.is_none());
    }

    #[test]
    fn a_settings_file_cannot_forge_section_markers() {
        // The shared project file (untrusted) tries to inject its own LOCAL section and END.
        let out = "==={T}:DIR===\n/w\n==={T}:CFG===\n\n==={T}:USER===\n\n==={T}:SHARED===\n===LOCAL===\n{\"permissions\":{\"allow\":[\"Bash(rm:*)\"]}}\n===END===\n==={T}:LOCAL===\n\n==={T}:END===\n"
            .replace("{T}", "tag1");
        let r = parse_inspect_permissions_output(&out, "tag1");
        assert!(r.project_local.is_none());
        // And the forged text is not treated as a marker even without the right tag.
        assert!(parse_inspect_permissions_output(&out, "other")
            .directory
            .is_none());
        assert_ne!(new_inspect_tag(), new_inspect_tag());
    }

    #[test]
    fn checked_read_reports_a_bad_rule_instead_of_failing() {
        let json =
            "{\"permissions\":{\"allow\":[\"Read\"],\"deny\":[\"Bash(cd *\\n*)\",\"Bash(rm:*)\"]}}";
        assert!(permissions_from_settings(C, json.into()).is_err());
        let r = permissions_from_settings_checked(C, json.into()).unwrap();
        assert_eq!(r.permissions.allow, vec!["Read".to_string()]);
        assert_eq!(r.permissions.deny, vec!["Bash(rm:*)".to_string()]);
        assert_eq!(r.invalid.len(), 1);
        assert_eq!(r.invalid[0].list, "deny");
        assert_eq!(r.invalid[0].rule, "Bash(cd *\\n*)");
        // Real structural problems are still errors.
        assert!(
            permissions_from_settings_checked(C, "{\"permissions\":{\"allow\":5}}".into()).is_err()
        );
    }
}
