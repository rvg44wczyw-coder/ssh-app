//! The agent hook: reads the agent's hook JSON on stdin, applies the host guard, asks the phone
//! through the queue and prints the agent-specific decision. No output means "no opinion" and the
//! agent applies its own policy (OpenCode is the documented exception, see below).

use crate::guard::{
    guard_match, guard_rules, guard_text, input_too_big, load_guard, GUARD_MAX_INPUT,
};
use crate::pyjson::{dumps, py_str};
use crate::queue::{clean, submit, summarize, Base, Paths};
use crate::sessions::{register_if_absent, register_session};
use serde_json::{json, Value};
use std::io::{Read, Write};

/// Agents that speak Claude's hook protocol (payload fields and reply shapes).
const CLAUDE_STYLE: [&str; 3] = ["claude", "codex", "qwen"];
const MAX_STDIN: u64 = 32 << 20;

fn allow_output(agent: &str, event: &str, reason: &str) -> Option<Value> {
    if CLAUDE_STYLE.contains(&agent) {
        if event == "PermissionRequest" {
            return Some(json!({"hookSpecificOutput": {
                "hookEventName": "PermissionRequest", "decision": {"behavior": "allow"}}}));
        }
        return Some(json!({"hookSpecificOutput": {
            "hookEventName": event, "permissionDecision": "allow",
            "permissionDecisionReason": reason}}));
    }
    match agent {
        "antigravity" | "opencode" => Some(json!({"decision": "allow", "reason": reason})),
        "crush" => Some(json!({"version": 1, "decision": "allow", "reason": reason})),
        _ => None,
    }
}

/// Hands the decision to the user even where the agent's mode would not ask.
fn ask_output(agent: &str, event: &str, reason: &str) -> Option<Value> {
    if agent == "claude" && event == "PreToolUse" {
        return Some(json!({"hookSpecificOutput": {
            "hookEventName": event, "permissionDecision": "ask",
            "permissionDecisionReason": reason}}));
    }
    (agent == "antigravity").then(|| json!({"decision": "ask", "reason": reason}))
}

fn deny_output(agent: &str, event: &str, reason: &str) -> Value {
    if agent == "crush" {
        return json!({"version": 1, "decision": "deny", "reason": reason});
    }
    if CLAUDE_STYLE.contains(&agent) {
        if event == "PermissionRequest" {
            return json!({"hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {"behavior": "deny", "message": reason}}});
        }
        return json!({"hookSpecificOutput": {
            "hookEventName": event, "permissionDecision": "deny",
            "permissionDecisionReason": reason}});
    }
    json!({"decision": "deny", "reason": reason})
}

fn emit(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(dumps(v).as_bytes());
    let _ = out.flush();
}

struct Call {
    tool: String,
    input: Value,
    cwd: String,
    session: String,
}

fn field(data: &Value, key: &str) -> String {
    py_str(data.get(key).unwrap_or(&json!("")))
}

fn input_of(data: &Value, key: &str) -> Value {
    data.get(key).cloned().unwrap_or_else(|| json!({}))
}

fn parse_call(agent: &str, data: &Value) -> Option<Call> {
    Some(match agent {
        // Codex and Qwen Code use Claude's payload; Crush documents the same field names.
        "claude" | "codex" | "qwen" | "crush" => Call {
            tool: field(data, "tool_name"),
            input: input_of(data, "tool_input"),
            cwd: field(data, "cwd"),
            session: field(data, "session_id"),
        },
        "antigravity" => {
            let call = data
                .get("toolCall")
                .filter(|c| c.is_object())
                .cloned()
                .unwrap_or(json!({}));
            let first = data
                .get("workspacePaths")
                .and_then(Value::as_array)
                .and_then(|a| a.first());
            Call {
                tool: field(&call, "name"),
                input: input_of(&call, "args"),
                cwd: first.map(py_str).unwrap_or_default(),
                session: field(data, "conversationId"),
            }
        }
        // Sent by the generated OpenCode plugin: {"tool","args","cwd","session"}.
        "opencode" => Call {
            tool: field(data, "tool"),
            input: input_of(data, "args"),
            cwd: field(data, "cwd"),
            session: field(data, "session"),
        },
        "grok" => Call {
            tool: field(data, "toolName"),
            input: input_of(data, "toolInput"),
            cwd: field(data, "cwd"),
            session: field(data, "sessionId"),
        },
        _ => return None,
    })
}

/// Whether the conversation's transcript ends with the model's reply (a finished turn). A transcript
/// that ends on a command result, an error or the user's own input means the agent stopped midway.
/// Unreadable or missing: assume it finished (no false alarm).
fn agy_ended_with_answer(session: &str) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    if session.is_empty()
        || !session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return true;
    }
    let Some(home) = std::env::var_os("HOME") else {
        return true;
    };
    let path = std::path::Path::new(&home)
        .join(".gemini/antigravity-cli/brain")
        .join(session)
        .join(".system_generated/logs/transcript.jsonl");
    let Ok(mut f) = std::fs::File::open(&path) else {
        return true;
    };
    let len = f.metadata().map_or(0, |m| m.len());
    // The last step can be long; its type and status come first in the line, so a window is enough.
    let window = 256 * 1024u64;
    if f.seek(SeekFrom::Start(len.saturating_sub(window))).is_err() {
        return true;
    }
    let mut raw = Vec::new();
    if f.take(window).read_to_end(&mut raw).is_err() {
        return true;
    }
    let text = String::from_utf8_lossy(&raw);
    let Some(last) = text
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{') && l.trim_end().ends_with('}'))
    else {
        return true;
    };
    last_step_is_answer(last)
}

/// `command(...)` entries of Antigravity's `permissions.allow`, read from its settings file.
fn agy_allow_entries() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let path = std::path::Path::new(&home).join(".gemini/antigravity-cli/settings.json");
    let Some(raw) = crate::queue::read_small(&path, 1 << 20) else {
        return Vec::new();
    };
    serde_json::from_slice::<Value>(&raw)
        .ok()
        .and_then(|v| v["permissions"]["allow"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|e| e.as_str())
        .filter_map(|e| e.strip_prefix("command(")?.strip_suffix(')'))
        .map(String::from)
        .collect()
}

fn agy_allows(command: &str) -> bool {
    command_allowed_by(command, &agy_allow_entries())
}

/// A plain command (no chaining, pipes, substitution or redirection, nothing cut off) that equals an
/// allow entry or continues it after a space; an entry ending in `*` is a prefix. Anything compound
/// goes to the phone, where its parts can be seen.
fn command_allowed_by(command: &str, entries: &[String]) -> bool {
    let c = command.trim();
    if c.is_empty()
        || c.ends_with('…')
        || c.chars().any(|ch| {
            matches!(
                ch,
                ';' | '&' | '|' | '`' | '$' | '<' | '>' | '\n' | '\r' | '(' | ')' | '{' | '}'
            )
        })
    {
        return false;
    }
    entries.iter().any(|e| {
        let e = e.trim();
        if e.is_empty() {
            return false;
        }
        match e.strip_suffix('*') {
            Some(prefix) => !prefix.trim().is_empty() && c.starts_with(prefix.trim_end()),
            None => c == e || c.strip_prefix(e).is_some_and(|rest| rest.starts_with(' ')),
        }
    })
}

fn last_step_is_answer(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return true;
    };
    let kind = v["type"].as_str().unwrap_or("");
    let status = v["status"].as_str().unwrap_or("");
    let content = v["content"].as_str().unwrap_or("");
    kind == "PLANNER_RESPONSE" && status == "DONE" && !content.trim().is_empty()
}

fn context_reply(p: &Paths, event: &str, key: &str, agent: &str) {
    let guard = load_guard(p);
    let text = guard_text(guard.get("context").and_then(|c| c.get(key)));
    if !text.is_empty() && agent == "claude" {
        emit(&json!({"hookSpecificOutput": {
            "hookEventName": event, "additionalContext": text}}));
    }
}

/// Runs the hook for `args = [agent, event, ...]`; always exit code 0 (no opinion on any error).
pub fn run(p: &Paths, args: &[String]) -> i32 {
    let agent = args.first().map_or("", String::as_str);
    let event = args.get(1).map_or("", String::as_str);
    let mut raw = Vec::new();
    let _ = std::io::stdin()
        .lock()
        .take(MAX_STDIN + 1)
        .read_to_end(&mut raw);
    let too_big = raw.len() as u64 > MAX_STDIN;
    let data = if too_big {
        json!({})
    } else {
        serde_json::from_slice::<Value>(&raw)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}))
    };
    let Some(call) = parse_call(agent, &data) else {
        return 0;
    };
    let summary = summarize(&call.input);
    let base = Base {
        agent,
        event,
        tool: &call.tool,
        command: &summary,
        cwd: &call.cwd,
        session: &call.session,
    };

    if event != "SessionStart" && event != "SessionEnd" {
        register_if_absent(p, agent, &call.session, &call.cwd);
    }
    if event == "SessionStart" || event == "SessionEnd" {
        register_session(p, agent, event, &call.session, &call.cwd);
        if event == "SessionStart" {
            context_reply(p, "SessionStart", "SessionStart", agent);
        }
        return 0;
    }
    if event == "UserPromptSubmit" {
        // Text the user wrote in the app is added to the prompt's context. Never the prompt itself.
        context_reply(p, "UserPromptSubmit", "UserPromptSubmit", agent);
        return 0;
    }
    if event == "PermissionRequest" || event == "PreToolUse" {
        permission(p, agent, event, &call, &base, too_big);
        return 0;
    }
    // Stop: the user may have asked for one more round (once per stop; the flag below is what
    // keeps this from looping when the agent stops again after that round).
    if event == "Stop" && agent == "claude" && data.get("stop_hook_active") != Some(&json!(true)) {
        let guard = load_guard(p);
        let message = guard_text(guard.get("on_stop").and_then(|c| c.get("message")));
        if !message.is_empty() {
            emit(&json!({"decision": "block", "reason": message}));
            return 0;
        }
    }
    // Notification / Stop and other passive events: fire-and-forget. An Antigravity stop whose
    // transcript does not end with the model's own answer is reported as a failure, not as "finished".
    if agent == "antigravity" && event == "Stop" && !agy_ended_with_answer(&call.session) {
        let failed = Base {
            event: "StopFailed",
            ..base
        };
        submit(p, "notify", &failed, false);
        emit(&json!({"decision": "stop"}));
        return 0;
    }
    submit(p, "notify", &base, false);
    if agent == "antigravity" && event == "Stop" {
        emit(&json!({"decision": "stop"}));
    }
    0
}

fn permission(p: &Paths, agent: &str, event: &str, call: &Call, base: &Base, too_big: bool) {
    let guard = load_guard(p);
    let (deny, ask, allow) = (
        guard_rules(&guard, "deny"),
        guard_rules(&guard, "ask"),
        guard_rules(&guard, "allow"),
    );
    // A payload beyond the read cap was never parsed: the phone would be asked about a blank
    // request, so it is refused outright (nothing legitimate is that large).
    if too_big
        || (input_too_big(&call.input) && !(deny.is_empty() && ask.is_empty() && allow.is_empty()))
    {
        // Rules cannot be checked reliably (and cheaply) on this much text: fail closed.
        emit(&deny_output(
            agent,
            event,
            &format!("Blocked: the tool input is longer than {GUARD_MAX_INPUT} characters, too long to check against the host guard rules"),
        ));
        return;
    }
    if let Some(hit) = guard_match(&deny, &call.tool, &call.input, &call.cwd) {
        let reason = format!("Blocked by a host guard rule: {}", clean(&hit, 120));
        emit(&deny_output(agent, event, &reason));
        return;
    }
    if let Some(hit) = guard_match(&ask, &call.tool, &call.input, &call.cwd) {
        let reason = format!("A host guard rule asks you first: {}", clean(&hit, 120));
        if let Some(out) = ask_output(agent, event, &reason) {
            emit(&out);
            return;
        }
    } else if let Some(hit) = guard_match(&allow, &call.tool, &call.input, &call.cwd) {
        let reason = format!("Allowed by a host guard rule: {}", clean(&hit, 120));
        if let Some(out) = allow_output(agent, event, &reason) {
            emit(&out);
            return;
        }
    }
    // Antigravity's own allow list (`permissions.allow` in its settings): a plain command on it runs
    // without asking in the terminal, so the phone is not asked either. No answer from the hook
    // leaves the decision to the agent's own flow.
    if agent == "antigravity"
        && call.tool == "run_command"
        && guard_match(&ask, &call.tool, &call.input, &call.cwd).is_none()
        && agy_allows(base.command)
    {
        return;
    }
    let approved = submit(p, "approval", base, true);
    match agent {
        // Grok can only block: deny is explicit, everything else (including a phone approval)
        // lets the tool continue into Grok's own flow; failures fail open on its side anyway.
        "grok" => {
            if approved == Some(false) {
                emit(&json!({"decision": "deny", "reason": "Denied from ssh-app on your phone"}));
            }
        }
        // Exit 0 with a decision; no answer lets Crush run its own permission flow.
        "crush" => match approved {
            Some(true) => {
                emit(&allow_output(agent, event, "Approved from ssh-app").unwrap_or_default())
            }
            Some(false) => emit(&deny_output(
                agent,
                event,
                "Denied from ssh-app on your phone",
            )),
            None => {}
        },
        // The plugin throws on deny. OpenCode allows most tools by default, so no answer must
        // not mean "go ahead".
        "opencode" => emit(&match approved {
            Some(true) => json!({"decision": "allow", "reason": "Approved from ssh-app"}),
            Some(false) => {
                json!({"decision": "deny", "reason": "Denied from ssh-app on your phone"})
            }
            None => {
                json!({"decision": "deny", "reason": "No answer from your phone in time (or the ssh-app approval queue is not set up); blocked"})
            }
        }),
        a if CLAUDE_STYLE.contains(&a) => {
            let Some(approved) = approved else {
                return; // no opinion: the agent shows its normal permission prompt
            };
            if event == "PreToolUse" {
                emit(&json!({"hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": if approved { "allow" } else { "deny" },
                    "permissionDecisionReason": if approved { "Approved from ssh-app" } else { "Denied from ssh-app on your phone" }}}));
            } else {
                let decision = if approved {
                    json!({"behavior": "allow"})
                } else {
                    json!({"behavior": "deny", "message": "Denied from ssh-app on your phone"})
                };
                emit(&json!({"hookSpecificOutput": {
                    "hookEventName": "PermissionRequest", "decision": decision}}));
            }
        }
        _ => emit(&match approved {
            // "allow" would auto-approve (agy docs); "ask" hands the decision back to the user.
            None => {
                json!({"decision": "ask", "reason": "ssh-app broker unavailable; asking you in the terminal"})
            }
            Some(true) => json!({"decision": "allow", "reason": "Approved from ssh-app"}),
            Some(false) => {
                json!({"decision": "deny", "reason": "Denied from ssh-app on your phone"})
            }
        }),
    }
}

#[cfg(test)]
mod stop_tests {
    use super::{command_allowed_by, last_step_is_answer};

    #[test]
    fn only_a_finished_model_reply_counts_as_an_answer() {
        assert!(last_step_is_answer(
            r#"{"step_index":9,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Done."}"#
        ));
        // ends on a failed command (the case that sent a false "finished")
        assert!(!last_step_is_answer(
            r#"{"step_index":161,"source":"MODEL","type":"GENERIC","status":"DONE","content":"Command exited with code 1."}"#
        ));
        // a reply that is only thinking, an error status, the user's own input
        assert!(!last_step_is_answer(
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","thinking":"x"}"#
        ));
        assert!(!last_step_is_answer(
            r#"{"type":"PLANNER_RESPONSE","status":"ERROR","content":"x"}"#
        ));
        assert!(!last_step_is_answer(
            r#"{"type":"USER_INPUT","status":"DONE","content":"hi"}"#
        ));
        // not JSON: no false alarm
        assert!(last_step_is_answer("garbage"));
    }

    #[test]
    fn a_plain_command_on_the_agy_allow_list_skips_the_phone() {
        let allow = vec![
            "grep".to_string(),
            "cargo test -p ssh-core".to_string(),
            "git status*".to_string(),
        ];
        assert!(command_allowed_by("grep -rn foo src", &allow));
        assert!(command_allowed_by("cargo test -p ssh-core --lib", &allow));
        assert!(command_allowed_by("git status -s", &allow));
        // not on the list, or only sharing letters with an entry
        assert!(!command_allowed_by("grepx foo", &allow));
        assert!(!command_allowed_by("cargo build", &allow));
        // compound, substituted, redirected or cut off: the phone sees it
        assert!(!command_allowed_by("grep foo && rm -rf ~", &allow));
        assert!(!command_allowed_by("grep $(whoami)", &allow));
        assert!(!command_allowed_by("grep foo > out", &allow));
        assert!(!command_allowed_by("grep foo | sh", &allow));
        assert!(!command_allowed_by("grep a…", &allow));
        assert!(!command_allowed_by("", &allow));
    }
}
