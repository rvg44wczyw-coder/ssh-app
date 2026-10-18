//! Claude Code `statusLine` wrapping (records the rate limits, keeps the user's own command).

use super::*;

// ---- Claude Code status line (plan limits) -------------------------------------------------
//
// Claude Code hands the real plan limits (`rate_limits`: percent used and reset time of the
// 5-hour and 7-day windows) only to its `statusLine` command. The wrapper below lets the host
// script record them under `~/.ssh-app/run/limits/` and then runs whatever status line the user
// had before (kept hex-encoded in our own command line, so nothing else is stored anywhere).

const STATUSLINE_MAX_PREV: usize = 2000;

fn statusline_command(hook_dir: &str, prev: Option<&str>) -> String {
    let base = hook_command(hook_dir, "claude", "statusline");
    match prev {
        Some(p) => format!("{base} {}", hex::encode(p)),
        None => base,
    }
}

/// `Some(previous command)` (possibly none) when `value` is our wrapper, `None` otherwise.
fn our_statusline_prev(value: &Value) -> Option<Option<String>> {
    let command = value.get("command").and_then(Value::as_str)?;
    if !is_our_command(command) {
        return None;
    }
    let mut parts = command.split(' ').skip_while(|p| *p != "claude");
    if (parts.next(), parts.next()) != (Some("claude"), Some("statusline")) {
        return None;
    }
    Some(
        parts
            .next()
            .and_then(|h| hex::decode(h).ok())
            .and_then(|b| String::from_utf8(b).ok()),
    )
}

/// Installs the status-line wrapper into a Claude `settings.json` (idempotent). An existing
/// command status line is kept and run by the wrapper; any other kind is refused.
#[uniffi::export]
pub fn merge_claude_statusline(
    existing: String,
    hook_dir: Option<String>,
) -> Result<String, SshCoreError> {
    let dir = hook_dir.unwrap_or_else(|| DEFAULT_HOOK_DIR.to_string());
    validate_hook_dir(&dir)?;
    let mut map = parse_object(&existing)?;
    let mut entry = Map::new();
    let prev = match map.get("statusLine") {
        None | Some(Value::Null) => None,
        Some(v) => match our_statusline_prev(v) {
            Some(prev) => prev,
            None => {
                let command = v
                    .get("command")
                    .and_then(Value::as_str)
                    .filter(|_| v.get("type").and_then(Value::as_str) == Some("command"))
                    .ok_or_else(|| {
                        invalid("`statusLine` is not a command; refusing to replace it")
                    })?;
                if command.len() > STATUSLINE_MAX_PREV || command.chars().any(char::is_control) {
                    return Err(invalid(
                        "Your status line command is too long or odd to wrap",
                    ));
                }
                Some(command.to_string())
            }
        },
    };
    if let Some(old) = map.get("statusLine").and_then(Value::as_object) {
        for (k, v) in old {
            if k != "type" && k != "command" {
                entry.insert(k.clone(), v.clone());
            }
        }
    }
    entry.insert("type".into(), json!("command"));
    entry.insert(
        "command".into(),
        json!(statusline_command(&dir, prev.as_deref())),
    );
    map.insert("statusLine".into(), Value::Object(entry));
    render(map)
}

/// Removes our wrapper and puts the user's previous status line back (or drops the key).
#[uniffi::export]
pub fn remove_claude_statusline(existing: String) -> Result<String, SshCoreError> {
    let mut map = parse_object(&existing)?;
    let Some(prev) = map.get("statusLine").and_then(our_statusline_prev) else {
        return render(map);
    };
    match prev {
        Some(command) => {
            if let Some(Value::Object(o)) = map.get_mut("statusLine") {
                o.insert("command".into(), json!(command));
            }
        }
        None => {
            map.remove("statusLine");
        }
    }
    render(map)
}

#[uniffi::export]
pub fn claude_statusline_installed(existing: String) -> bool {
    parse_object(&existing)
        .ok()
        .and_then(|m| m.get("statusLine").and_then(our_statusline_prev))
        .is_some()
}
