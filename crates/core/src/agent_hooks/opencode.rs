//! OpenCode support: the generated plugin that forwards tool calls to the hook script.

use super::*;

const OPENCODE_MARKER: &str = "// ssh-app-hook managed plugin; entries=";

/// The planned entries recorded in the plugin's first line (ours only; anything else: none).
pub(super) fn opencode_entries(text: &str) -> Vec<PlannedHook> {
    let Some(line) = text.lines().next() else {
        return vec![];
    };
    let Some(json_text) = line.strip_prefix(OPENCODE_MARKER) else {
        return vec![];
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json_text) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|i| {
            Some(PlannedHook {
                event: i.get("event")?.as_str()?.to_string(),
                matcher: i.get("matcher").and_then(Value::as_str).map(str::to_string),
                command: i.get("command")?.as_str()?.to_string(),
            })
        })
        .collect()
}

/// Script path (as `("home", "rest")` or `("abs", "/path")`) taken from a planned command of the
/// form `python3 -I "<path>" ...`; the hook dir was validated to contain no quote or `$` besides
/// the leading `$HOME`.
fn script_location(command: &str) -> Option<(&'static str, String)> {
    let path = command.split('"').nth(1)?;
    match path.strip_prefix("$HOME/") {
        Some(rest) => Some(("home", rest.to_string())),
        None => path.starts_with('/').then(|| ("abs", path.to_string())),
    }
}

/// What the generator accepts: our two events, a matcher from the validated charset and a command
/// shaped like our own hook command. Everything is checked again here because
/// `merge_planned_hooks` is an exported API and the text ends up as JavaScript.
fn validate_opencode_entry(h: &PlannedHook) -> Result<(), SshCoreError> {
    let matcher_ok = h.matcher.as_deref().is_none_or(|m| {
        !m.is_empty()
            && m.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '|' | '-' | '*'))
    });
    if !matches!(h.event.as_str(), "PreToolUse" | "Stop")
        || !matcher_ok
        || !h.command.starts_with("python3 -I \"")
        || h.command.chars().any(char::is_control)
        || h.command
            .chars()
            .any(|c| matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(invalid("Refusing to write a malformed OpenCode hook entry"));
    }
    Ok(())
}

/// The OpenCode plugin text for `planned` (docs: plugins live in `~/.config/opencode/plugins/`;
/// `tool.execute.before` blocks a call by throwing). Generated from validated data only, string
/// values are JSON-escaped; the script is run with `execFile` (no shell). Any failure to ask the
/// phone (missing script, crash, timeout) throws, i.e. the tool call is blocked, not allowed.
/// `existing` must be empty or one of our plugins: a foreign file is never overwritten.
pub(super) fn opencode_plugin(
    existing: &str,
    planned: &[PlannedHook],
) -> Result<String, SshCoreError> {
    if !existing.trim().is_empty() && !existing.starts_with(OPENCODE_MARKER) {
        return Err(invalid(
            "The OpenCode plugin file exists and is not ours; refusing to overwrite it",
        ));
    }
    planned.iter().try_for_each(validate_opencode_entry)?;
    let entries: Vec<Value> = planned
        .iter()
        .map(|h| json!({"event": h.event, "matcher": h.matcher, "command": h.command}))
        .collect();
    let lit = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
    let location = planned
        .iter()
        .find_map(|h| script_location(&h.command))
        .unwrap_or(("home", format!(".ssh-app/bin/{HOOK_MARKER}")));
    let script = match location.0 {
        "home" => format!("join(homedir(), {})", lit(&location.1)),
        _ => lit(&location.1),
    };
    let tool_matcher = planned.iter().find(|h| h.event == "PreToolUse").map(|h| {
        h.matcher
            .clone()
            .unwrap_or_else(|| OPENCODE_TOOL_MATCHER.into())
    });
    let has_stop = planned.iter().any(|h| h.event == "Stop");
    let mut out = format!(
        "{OPENCODE_MARKER}{}\n// Written by SSH Mobile; edit it through the app. Asks the phone before shell commands run.\n",
        // U+2028/U+2029 end a `//` comment in JavaScript and serde_json leaves them raw.
        serde_json::to_string(&entries)
            .map_err(|e| invalid(format!("Failed to serialize: {e}")))?
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
    );
    out.push_str(
        r#"import { execFile } from "node:child_process"
import { homedir } from "node:os"
import { join } from "node:path"

"#,
    );
    out.push_str(&format!("const SCRIPT = {script}\n"));
    out.push_str(
        r#"
// Resolves with the script's JSON answer; rejects on any failure, which blocks the tool call.
function ask(event, payload) {
  return new Promise((resolve, reject) => {
    const child = execFile("python3", ["-I", SCRIPT, "opencode", event], { timeout: 135000, maxBuffer: 65536 }, (err, stdout) => {
      if (err) return reject(err)
      try { resolve(stdout.trim() ? JSON.parse(stdout) : {}) } catch (e) { reject(e) }
    })
    child.stdin.on("error", () => {})
    child.stdin.end(JSON.stringify(payload))
  })
}

export const SshAppPlugin = async ({ directory }) => ({
"#,
    );
    if let Some(m) = tool_matcher {
        let pattern = if m == "*" {
            ".".to_string()
        } else {
            format!("^(?:{m})$")
        };
        out.push_str(&format!(
            r#"  "tool.execute.before": async (input, output) => {{
    if (!new RegExp({}).test(String(input.tool))) return
    const r = await ask("PreToolUse", {{ tool: input.tool, args: output.args, cwd: directory, session: input.sessionID }})
    if (r.decision !== "allow") throw new Error(r.reason || "Blocked: no approval from ssh-app")
  }},
"#,
            lit(&pattern)
        ));
    }
    if has_stop {
        out.push_str(
            r#"  event: async ({ event }) => {
    if (event.type === "session.idle") {
      try { await ask("Stop", { cwd: directory, session: event.properties?.sessionID }) } catch {}
    }
  },
"#,
        );
    }
    out.push_str("})\n");
    Ok(out)
}
