//! Rules kept on the host in `~/.ssh-app/guard.json` and enforced by the ssh-app hook script
//! itself, before anything reaches the phone: deny, ask and allow decisions with the same rule
//! shape as the agents' permission rules, plus the two other things a hook can do that nothing
//! used so far: add context to a session or prompt, and ask the agent to keep going when it stops.
//!
//! Everything here is typed by the user in the app and shown back to them; nothing the agent
//! or a repository writes ever becomes a rule or context. The script never rewrites tool input
//! (`updatedInput`): running a command the user never saw is not offered.

use crate::agents::{invalid, AgentKind};
use crate::error::SshCoreError;
use serde_json::{json, Value};

pub const GUARD_FILE: &str = "~/.ssh-app/guard.json";
const MAX_RULES: usize = 100;
const MAX_TEXT: usize = 2000;

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct HostGuard {
    /// Blocked on the host, without asking anyone.
    pub deny: Vec<String>,
    /// Always handed to the user (Claude: PreToolUse "ask"), even if the agent's mode would not ask.
    pub ask: Vec<String>,
    /// Allowed without a prompt (Claude, Antigravity); a `deny` rule always wins.
    pub allow: Vec<String>,
    /// Text added to the model's context when a session starts (Claude).
    pub session_start_context: String,
    /// Text added to the model's context with every prompt you send (Claude).
    pub prompt_context: String,
    /// When the agent stops, ask it to continue with this message, once per stop (Claude).
    pub stop_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostGuardPreset {
    pub id: String,
    pub title: String,
    pub description: String,
    pub guard: HostGuard,
}

fn rules(list: &str, items: &[String]) -> Result<Vec<String>, SshCoreError> {
    if items.len() > MAX_RULES {
        return Err(invalid(format!("At most {MAX_RULES} {list} rules")));
    }
    let mut out: Vec<String> = vec![];
    for r in items {
        let v = crate::permissions::validate_rule(list, r)?;
        if !out.contains(&v) {
            out.push(v);
        }
    }
    Ok(out)
}

fn text(what: &str, t: &str) -> Result<String, SshCoreError> {
    let t = t.trim();
    if t.chars().count() > MAX_TEXT {
        return Err(invalid(format!(
            "{what} may be at most {MAX_TEXT} characters"
        )));
    }
    if t.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err(invalid(format!("{what} contains control characters")));
    }
    Ok(t.to_string())
}

/// Trims, de-duplicates and validates; the result is what gets written.
#[uniffi::export]
pub fn validate_host_guard(guard: HostGuard) -> Result<HostGuard, SshCoreError> {
    Ok(HostGuard {
        deny: rules("deny", &guard.deny)?,
        ask: rules("ask", &guard.ask)?,
        allow: rules("allow", &guard.allow)?,
        session_start_context: text("Session context", &guard.session_start_context)?,
        prompt_context: text("Prompt context", &guard.prompt_context)?,
        stop_message: text("Stop message", &guard.stop_message)?,
    })
}

/// The JSON the hook script reads.
#[uniffi::export]
pub fn host_guard_to_json(guard: HostGuard) -> Result<String, SshCoreError> {
    let g = validate_host_guard(guard)?;
    let mut context = serde_json::Map::new();
    if !g.session_start_context.is_empty() {
        context.insert("SessionStart".into(), json!(g.session_start_context));
    }
    if !g.prompt_context.is_empty() {
        context.insert("UserPromptSubmit".into(), json!(g.prompt_context));
    }
    let mut v = json!({"v": 1, "deny": g.deny, "ask": g.ask, "allow": g.allow, "context": context});
    if !g.stop_message.is_empty() {
        v["on_stop"] = json!({"message": g.stop_message});
    }
    serde_json::to_string_pretty(&v).map_err(|e| invalid(format!("Could not write the guard: {e}")))
}

/// Reads the file text from the host; empty text is an empty guard. Anything unreadable is an
/// error, so the app never overwrites a file it could not understand.
#[uniffi::export]
pub fn host_guard_from_json(text: String) -> Result<HostGuard, SshCoreError> {
    if text.trim().is_empty() {
        return Ok(HostGuard::default());
    }
    let v: Value =
        serde_json::from_str(&text).map_err(|_| invalid("guard.json is not valid JSON"))?;
    let obj = v
        .as_object()
        .ok_or_else(|| invalid("guard.json must be an object"))?;
    let list = |key: &str| -> Result<Vec<String>, SshCoreError> {
        match obj.get(key) {
            None => Ok(vec![]),
            Some(Value::Array(a)) => a
                .iter()
                .map(|x| {
                    x.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| invalid(format!("guard.json: '{key}' must hold strings")))
                })
                .collect(),
            Some(_) => Err(invalid(format!("guard.json: '{key}' must be a list"))),
        }
    };
    let ctx = |event: &str| {
        obj.get("context")
            .and_then(|c| c.get(event))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let stop = obj
        .get("on_stop")
        .and_then(|c| c.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    validate_host_guard(HostGuard {
        deny: list("deny")?,
        ask: list("ask")?,
        allow: list("allow")?,
        session_start_context: ctx("SessionStart"),
        prompt_context: ctx("UserPromptSubmit"),
        stop_message: stop,
    })
}

/// Things to read twice before applying.
#[uniffi::export]
pub fn host_guard_warnings(guard: HostGuard) -> Vec<String> {
    let mut out = vec![];
    for r in &guard.allow {
        let bare = r.trim();
        if matches!(bare, "Bash" | "Bash(*)" | "*" | "Edit" | "Write") {
            out.push(format!(
                "Allow rule \"{bare}\" lets the agent do that without ever asking, including things you did not think of."
            ));
        }
    }
    for r in &guard.allow {
        if guard.deny.contains(r) || guard.ask.contains(r) {
            out.push(format!(
                "\"{r}\" is in allow and also in deny/ask; deny wins, ask is kept."
            ));
        }
    }
    if !guard.stop_message.is_empty() {
        out.push("The stop message makes the agent continue after every stop (once per stop). A message like \"keep going\" can run up cost; ask for something that ends, such as \"run the tests and stop when they pass\".".into());
    }
    if !guard.session_start_context.is_empty() || !guard.prompt_context.is_empty() {
        out.push("Context text is added to what the model reads, with your authority. Keep it to facts and instructions you would type yourself.".into());
    }
    if !guard.ask.is_empty() || !guard.allow.is_empty() {
        out.push("Ask and allow rules are applied by the hook before tool use: the profile's hooks need the PreToolUse (or PermissionRequest) event.".into());
    }
    out
}

/// An agent session the hook script registered on the host (`SessionStart`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentSessionInfo {
    pub agent: String,
    pub session: String,
    pub cwd: String,
    /// tmux session the agent runs in; empty when it was not started inside tmux.
    pub tmux: String,
    pub started: u64,
}

pub(crate) const SESSIONS_COMMAND: &str =
    "sh -c 'python3 -I \"$HOME/.ssh-app/bin/ssh-app-hook\" ctl sessions'";

/// Parses `ctl sessions`; anything malformed is an empty list (an older script has no such command).
#[uniffi::export]
pub fn parse_agent_sessions(output: String) -> Vec<AgentSessionInfo> {
    let Ok(v) = serde_json::from_str::<Value>(output.trim()) else {
        return vec![];
    };
    let Some(items) = v.get("items").and_then(Value::as_array) else {
        return vec![];
    };
    let field = |i: &Value, k: &str| -> String {
        crate::approval_queue::display_safe(
            &i.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(200)
                .collect::<String>(),
        )
    };
    items
        .iter()
        .take(100)
        .filter_map(|i| {
            let session = field(i, "session");
            (!session.is_empty()).then(|| AgentSessionInfo {
                agent: field(i, "agent"),
                session,
                cwd: field(i, "cwd"),
                tmux: field(i, "tmux"),
                started: i.get("ts").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect()
}

/// Which parts of the guard each agent acts on.
#[uniffi::export]
pub fn host_guard_agent_support(agent: AgentKind) -> Vec<String> {
    let s: &[&str] = match agent {
        AgentKind::ClaudeCode => &["deny", "ask", "allow", "context", "stop message"],
        AgentKind::Codex => &["deny", "allow"],
        AgentKind::Antigravity => &["deny", "ask", "allow"],
        AgentKind::Grok => &["deny"],
    };
    s.iter().map(|x| (*x).to_string()).collect()
}

fn preset(id: &str, title: &str, description: &str, guard: HostGuard) -> HostGuardPreset {
    HostGuardPreset {
        id: id.into(),
        title: title.into(),
        description: description.into(),
        guard,
    }
}

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

/// Starting points for the guard editor; many, small, and combinable (the editor merges them).
#[uniffi::export]
pub fn host_guard_presets() -> Vec<HostGuardPreset> {
    let deny = |d: &[&str]| HostGuard {
        deny: v(d),
        ..Default::default()
    };
    let ask = |a: &[&str]| HostGuard {
        ask: v(a),
        ..Default::default()
    };
    let allow = |a: &[&str]| HostGuard {
        allow: v(a),
        ..Default::default()
    };
    vec![
        preset(
            "no-destructive",
            "Block destructive commands",
            "rm, git reset --hard, git clean, git push --force and dd are refused.",
            deny(&[
                "Bash(rm:*)",
                "Bash(git reset --hard:*)",
                "Bash(git clean:*)",
                "Bash(git push --force:*)",
                "Bash(git push -f:*)",
                "Bash(dd:*)",
                "Bash(mkfs:*)",
            ]),
        ),
        preset(
            "no-secrets",
            "Block secrets",
            "Reading or changing .env files, SSH keys and key material is refused.",
            deny(&[
                "Read(./.env)",
                "Read(./.env.*)",
                "Read(~/.ssh/**)",
                "Read(**/*.pem)",
                "Read(**/*.key)",
                "Edit(~/.ssh/**)",
                "Edit(./.env)",
                "Edit(./.env.*)",
            ]),
        ),
        preset(
            "no-agent-config-edits",
            "Protect agent and hook settings",
            "The agent cannot edit its own settings, hook files or the ssh-app folder.",
            deny(&[
                "Edit(~/.claude/settings.json)",
                "Edit(~/.claude/settings.local.json)",
                "Edit(./.claude/settings.json)",
                "Edit(./.claude/settings.local.json)",
                "Edit(~/.gemini/config/hooks.json)",
                "Edit(~/.codex/hooks.json)",
                "Edit(~/.grok/hooks/**)",
                "Edit(~/.ssh-app/**)",
            ]),
        ),
        preset(
            "no-network-shell",
            "Block shell downloads",
            "curl, wget, nc and scp from the shell are refused.",
            deny(&[
                "Bash(curl:*)",
                "Bash(wget:*)",
                "Bash(nc:*)",
                "Bash(scp:*)",
            ]),
        ),
        preset(
            "no-package-installs",
            "Block package installs",
            "npm/pip/brew/cargo install and apt are refused.",
            deny(&[
                "Bash(npm install:*)",
                "Bash(npm i:*)",
                "Bash(pip install:*)",
                "Bash(pip3 install:*)",
                "Bash(brew install:*)",
                "Bash(cargo install:*)",
                "Bash(apt:*)",
                "Bash(apt-get:*)",
            ]),
        ),
        preset(
            "no-sudo",
            "Block sudo",
            "Anything with sudo is refused.",
            deny(&["Bash(sudo:*)"]),
        ),
        preset(
            "no-git-writes",
            "Block git history changes",
            "commit, push, rebase, reset, merge and tag are refused (reading git stays possible).",
            deny(&[
                "Bash(git commit:*)",
                "Bash(git push:*)",
                "Bash(git rebase:*)",
                "Bash(git reset:*)",
                "Bash(git tag:*)",
                "Bash(git merge:*)",
            ]),
        ),
        preset(
            "ask-network",
            "Always ask before the network",
            "curl, wget, WebFetch and WebSearch are always handed to you.",
            ask(&["Bash(curl:*)", "Bash(wget:*)", "WebFetch", "WebSearch"]),
        ),
        preset(
            "ask-git-push",
            "Always ask before pushing",
            "git push is always handed to you, whatever the agent's mode.",
            ask(&["Bash(git push:*)"]),
        ),
        preset(
            "allow-read-only",
            "Allow reading without prompts",
            "Read, Glob, Grep and basic git/ls inspection run without asking.",
            allow(&[
                "Read",
                "Glob",
                "Grep",
                "Bash(git status:*)",
                "Bash(git diff:*)",
                "Bash(git log:*)",
                "Bash(ls:*)",
                "Bash(pwd:*)",
            ]),
        ),
        preset(
            "allow-dev-loop",
            "Allow build and test commands",
            "cargo build/test/fmt, make, npm test and xcodebuild run without asking.",
            allow(&[
                "Bash(cargo build:*)",
                "Bash(cargo test:*)",
                "Bash(cargo fmt:*)",
                "Bash(cargo clippy:*)",
                "Bash(make:*)",
                "Bash(npm test:*)",
                "Bash(npm run:*)",
                "Bash(xcodebuild:*)",
            ]),
        ),
        preset(
            "context-conventions",
            "Remind the agent of the house rules",
            "Adds a short reminder to every prompt.",
            HostGuard {
                prompt_context: "Follow the repository's conventions, keep changes small, run the tests before saying you are done, and never commit secrets.".into(),
                ..Default::default()
            },
        ),
        preset(
            "context-session",
            "Brief the agent when a session starts",
            "Adds a short briefing when a Claude session starts.",
            HostGuard {
                session_start_context: "You are working over SSH from a phone: keep answers short, avoid long outputs, and ask before anything destructive.".into(),
                ..Default::default()
            },
        ),
        preset(
            "stop-run-tests",
            "Make the agent run the tests before stopping",
            "When the agent stops, it is asked once to run the tests and fix failures before finishing.",
            HostGuard {
                stop_message: "Before you finish: run the project's tests. If something fails, fix it; if everything passes, stop.".into(),
                ..Default::default()
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(deny: &[&str]) -> HostGuard {
        HostGuard {
            deny: v(deny),
            ..Default::default()
        }
    }

    #[test]
    fn round_trips_through_the_script_json() {
        let guard = HostGuard {
            deny: v(&["Bash(rm:*)", "Read(~/.ssh/**)"]),
            ask: v(&["WebFetch"]),
            allow: v(&["Read"]),
            session_start_context: "hello".into(),
            prompt_context: "world".into(),
            stop_message: "run tests".into(),
        };
        let json = host_guard_to_json(guard.clone()).unwrap();
        assert_eq!(host_guard_from_json(json.clone()).unwrap(), guard);
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["deny"][0], "Bash(rm:*)");
        assert_eq!(parsed["context"]["SessionStart"], "hello");
        assert_eq!(parsed["context"]["UserPromptSubmit"], "world");
        assert_eq!(parsed["on_stop"]["message"], "run tests");
    }

    #[test]
    fn empty_file_is_an_empty_guard_and_garbage_is_an_error() {
        assert_eq!(
            host_guard_from_json("  ".into()).unwrap(),
            HostGuard::default()
        );
        for bad in [
            "nope",
            "[]",
            r#"{"deny":"x"}"#,
            r#"{"deny":[1]}"#,
            r#"{"deny":["Bash(("]}"#,
        ] {
            assert!(host_guard_from_json(bad.into()).is_err(), "{bad}");
        }
    }

    #[test]
    fn rules_and_texts_are_validated() {
        assert!(validate_host_guard(g(&["Bad Rule"])).is_err());
        assert!(validate_host_guard(g(&["Bash(rm:*) "])).is_ok());
        assert!(validate_host_guard(HostGuard {
            prompt_context: "a\u{7}b".into(),
            ..Default::default()
        })
        .is_err());
        assert!(validate_host_guard(HostGuard {
            stop_message: "x".repeat(MAX_TEXT + 1),
            ..Default::default()
        })
        .is_err());
        let many: Vec<String> = (0..=MAX_RULES).map(|i| format!("Tool{i}")).collect();
        assert!(validate_host_guard(HostGuard {
            deny: many,
            ..Default::default()
        })
        .is_err());
        let dup = validate_host_guard(g(&["Bash(rm:*)", "Bash(rm:*)"])).unwrap();
        assert_eq!(dup.deny.len(), 1);
    }

    #[test]
    fn warnings_cover_the_risky_choices() {
        let w = host_guard_warnings(HostGuard {
            allow: v(&["Bash"]),
            stop_message: "go".into(),
            prompt_context: "x".into(),
            ..Default::default()
        });
        assert!(w.iter().any(|x| x.contains("without ever asking")));
        assert!(w.iter().any(|x| x.contains("stop message")));
        assert!(w.iter().any(|x| x.contains("Context text")));
        assert!(host_guard_warnings(g(&["Bash(rm:*)"])).is_empty());
    }

    #[test]
    fn presets_are_valid_and_numerous() {
        let all = host_guard_presets();
        assert!(all.len() >= 12);
        let mut ids = std::collections::HashSet::new();
        for p in all {
            assert!(ids.insert(p.id.clone()), "{}", p.id);
            let ok =
                validate_host_guard(p.guard.clone()).unwrap_or_else(|e| panic!("{}: {e}", p.id));
            assert_eq!(ok, p.guard, "{}", p.id);
        }
    }

    #[test]
    fn sessions_output_is_parsed_defensively() {
        let ok = r#"{"host_time":5,"items":[{"agent":"claude","session":"abc","cwd":"/w","tmux":"term-1","ts":3},{"agent":"claude","session":"","cwd":"","tmux":"","ts":1}]}"#;
        let l = parse_agent_sessions(ok.into());
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].tmux.as_str(), l[0].started), ("term-1", 3));
        for bad in ["", "Traceback", "{}", r#"{"items":3}"#] {
            assert!(parse_agent_sessions(bad.into()).is_empty(), "{bad}");
        }
    }

    #[test]
    fn support_table_matches_the_script() {
        assert_eq!(
            host_guard_agent_support(AgentKind::Grok),
            vec!["deny".to_string()]
        );
        assert!(
            host_guard_agent_support(AgentKind::ClaudeCode).contains(&"stop message".to_string())
        );
    }
}
