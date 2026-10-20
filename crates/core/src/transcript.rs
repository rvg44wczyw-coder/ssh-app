use crate::error::TranscriptError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const MAX_PAYLOAD_SIZE: usize = 5 * 1024 * 1024; // 5 MB

/// Regex for stripping ANSI escape sequences.
static ANSI_REGEX: OnceLock<Regex> = OnceLock::new();

fn get_ansi_regex() -> &'static Regex {
    ANSI_REGEX.get_or_init(|| {
        // OSC strings (title, hyperlinks) up to their terminator first, then the short forms and CSI.
        Regex::new(r"\x1B\][^\x07\x1B]*(?:\x07|\x1B\\)?|\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])")
            .expect("Valid ANSI regex pattern")
    })
}

/// Strip ANSI escape codes from terminal / agent text, then every other control character except
/// line breaks and tabs, and the bidi override / isolate characters that can reorder what is shown.
#[uniffi::export]
pub fn strip_ansi_codes(input: String) -> String {
    get_ansi_regex()
        .replace_all(&input, "")
        .chars()
        .filter(|c| {
            (!c.is_control() || matches!(c, '\n' | '\t'))
                && !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        })
        .collect()
}

/// Execution status of an agent turn or individual tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, uniffi::Enum)]
pub enum TranscriptStatus {
    Running,
    #[default]
    Done,
    Error,
}

/// Type of conversation turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum TranscriptTurnType {
    User,
    Assistant,
    System,
    ApprovalRequest,
}

/// Represents an individual tool call executed by the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct TranscriptToolCall {
    pub call_id: String,
    pub tool_name: String,
    pub arguments_summary: String,
    pub full_arguments_json: Option<String>,
    pub result: Option<String>,
    pub is_error: bool,
    pub status: TranscriptStatus,
}

/// Represents a single normalized conversation turn in the structured transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct TranscriptTurn {
    pub id: String,
    pub step_index: u64,
    pub turn_type: TranscriptTurnType,
    pub timestamp: Option<String>,
    pub content: Option<String>,
    pub thinking: Option<String>,
    pub tool_calls: Vec<TranscriptToolCall>,
    pub status: TranscriptStatus,
    /// Short title of a service entry (hook, attachment, turn duration, checkpoint ...). Such turns
    /// are `System` and the app shows them folded; `content` holds the details.
    pub label: Option<String>,
}

/// Summarizes tool call arguments into a human-readable one-liner.
fn summarize_arguments(args: &serde_json::Value) -> String {
    let full = summarize_arguments_full(args);
    let first = full.lines().next().unwrap_or("");
    let mut out = clip(first, 160);
    if full.lines().nth(1).is_some() && !out.ends_with('…') {
        out.push('…');
    }
    out
}

fn summarize_arguments_full(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(map) => {
            if let Some(cmd) = map.get("CommandLine").and_then(|v| v.as_str()) {
                return cmd.to_string();
            }
            if let Some(cmd) = map.get("command").and_then(|v| v.as_str()) {
                return cmd.to_string();
            }
            if let Some(file) = map.get("TargetFile").and_then(|v| v.as_str()) {
                return file.to_string();
            }
            if let Some(path) = map.get("AbsolutePath").and_then(|v| v.as_str()) {
                return path.to_string();
            }
            if let Some(path) = map.get("path").and_then(|v| v.as_str()) {
                return path.to_string();
            }
            if let Some(query) = map.get("query").and_then(|v| v.as_str()) {
                return query.to_string();
            }
            if let Some(url) = map
                .get("Url")
                .or_else(|| map.get("url"))
                .and_then(|v| v.as_str())
            {
                return url.to_string();
            }
            // Fallback: compact JSON of first few keys
            serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
        }
        serde_json::Value::String(s) => s.clone(),
        _ => serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string()),
    }
}

/// Validates whether a remote path is safe to read.
pub fn validate_transcript_path(path: &str) -> Result<(), TranscriptError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(TranscriptError::InvalidPath {
            path: path.to_string(),
            reason: "Path cannot be empty".to_string(),
        });
    }
    if trimmed.starts_with('-') {
        return Err(TranscriptError::InvalidPath {
            path: path.to_string(),
            reason: "Path cannot start with '-'".to_string(),
        });
    }
    if trimmed.contains("..") {
        return Err(TranscriptError::InvalidPath {
            path: path.to_string(),
            reason: "Path traversal ('..') is prohibited".to_string(),
        });
    }
    if trimmed.chars().any(|c| c.is_control()) || trimmed.contains('\'') {
        return Err(TranscriptError::InvalidPath {
            path: path.to_string(),
            reason: "Path contains illegal characters".to_string(),
        });
    }
    // Block sensitive system and credential directories (both prefix and substring)
    let forbidden_prefixes = [
        "/etc/",
        "/var/",
        "/dev/",
        "/proc/",
        "/sys/",
        "/private/etc/",
        "/private/var/",
        "~/.ssh/",
        "~/.gnupg/",
        "~/.aws/",
    ];
    for prefix in forbidden_prefixes {
        if trimmed.starts_with(prefix) {
            return Err(TranscriptError::InvalidPath {
                path: path.to_string(),
                reason: format!("Access to '{}' is prohibited", prefix),
            });
        }
    }
    let forbidden_segments = ["/.ssh/", "/.gnupg/", "/.aws/", "/Library/Keychains/"];
    for segment in forbidden_segments {
        if trimmed.contains(segment) {
            return Err(TranscriptError::InvalidPath {
                path: path.to_string(),
                reason: format!("Access to '{}' is prohibited", segment),
            });
        }
    }
    Ok(())
}

/// Antigravity wraps the user's request in `<USER_REQUEST>…</USER_REQUEST>`.
fn strip_agy_user_wrapper(c: &str) -> String {
    let t = c.trim();
    t.strip_prefix("<USER_REQUEST>")
        .map(|r| r.strip_suffix("</USER_REQUEST>").unwrap_or(r))
        .unwrap_or(t)
        .trim()
        .to_string()
}

/// Antigravity tool results start with "Created At: …\nCompleted At: …\n\n"; the header is noise.
fn strip_agy_result_header(c: &str) -> String {
    if c.starts_with("Created At:") {
        if let Some((_, rest)) = c.split_once("\n\n") {
            return rest.trim_start_matches('\n').to_string();
        }
    }
    c.to_string()
}

/// Antigravity stores argument values as JSON-encoded strings (`"\"ls -la\""`); unwrap them.
fn unquote_values(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => serde_json::Value::Object(
            m.iter()
                .map(|(k, x)| (k.clone(), unquote_values(x)))
                .collect(),
        ),
        serde_json::Value::String(s) if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') => {
            serde_json::from_str::<String>(s)
                .map(serde_json::Value::String)
                .unwrap_or_else(|_| v.clone())
        }
        _ => v.clone(),
    }
}

/// Claude Code's own record: a lower-case `type` and no Antigravity / role-format keys.
fn is_claude_record(val: &serde_json::Value) -> bool {
    let Some(t) = val.get("type").and_then(|v| v.as_str()) else {
        return false;
    };
    val.get("step_index").is_none()
        && val.get("source").is_none()
        && val.get("role").is_none()
        && !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_lowercase() || c == '-' || c == '_')
}

fn text_of_blocks(content: &serde_json::Value) -> Option<String> {
    match content {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(blocks) => {
            let parts: Vec<String> = blocks
                .iter()
                .filter_map(|b| match b.get("type").and_then(|v| v.as_str()) {
                    Some("text") => b.get("text").and_then(|v| v.as_str()).map(String::from),
                    Some("image") => Some("[image]".to_string()),
                    _ => None,
                })
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('…');
        t
    }
}

fn join_opt(prev: Option<String>, add: Option<String>) -> Option<String> {
    match (prev, add) {
        (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
        (a, None) => a,
        (None, b) => b,
    }
}

fn duration_label(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 60 {
        format!("Worked for {}m {}s", s / 60, s % 60)
    } else {
        format!("Worked for {s}s")
    }
}

/// Names for Claude Code's `attachment` records (what it adds to the model's context) and the
/// one line worth showing for each. `None`: nothing to show.
fn describe_attachment(att: &serde_json::Value) -> Option<(String, Option<String>)> {
    let str_of = |k: &str| att.get(k).and_then(|v| v.as_str()).map(String::from);
    let names_of = |k: &str| -> Option<String> {
        att.get(k).and_then(|v| v.as_array()).map(|a| {
            a.iter()
                .filter_map(|x| {
                    x.as_str()
                        .or_else(|| x.get("path").and_then(|p| p.as_str()))
                        .or_else(|| x.get("name").and_then(|p| p.as_str()))
                        .or_else(|| x.get("uri").and_then(|p| p.as_str()))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    };
    let kind = att.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let non_empty = |o: Option<String>| o.filter(|s| !s.trim().is_empty());
    Some(match kind {
        "total_tokens_reminder" => {
            let t = str_of("text").unwrap_or_default();
            let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
            let value = match digits.parse::<u64>() {
                Ok(n) => {
                    let s = n.to_string();
                    let mut out = String::new();
                    for (i, c) in s.chars().enumerate() {
                        if i > 0 && (s.len() - i) % 3 == 0 {
                            out.push(',');
                        }
                        out.push(c);
                    }
                    format!("{out} tokens left")
                }
                Err(_) => t,
            };
            ("Tokens left".into(), Some(value))
        }
        "queued_command" => ("Message sent while working".into(), str_of("prompt")),
        "silent_turn_reminder" => ("Progress reminder".into(), str_of("text")),
        "hook_success" => (
            format!(
                "Hook: {}",
                str_of("hookName").unwrap_or_else(|| "hook".into())
            ),
            non_empty(str_of("stdout")).or_else(|| str_of("command")),
        ),
        "hook_system_message" => ("Hook message".into(), non_empty(str_of("content"))),
        "environment" => (
            "Environment".into(),
            att.get("snapshot")
                .and_then(|s| s.get("workingDirectory"))
                .and_then(|v| v.as_str())
                .map(String::from),
        ),
        "date" => ("Date".into(), str_of("date")),
        "model" => ("Model".into(), str_of("text")),
        "edited_text_file" => ("File changed".into(), str_of("filename")),
        "file" => (
            "File read".into(),
            str_of("displayPath").or_else(|| str_of("filename")),
        ),
        "compact_file_reference" => (
            "File reference".into(),
            str_of("displayPath").or_else(|| str_of("filename")),
        ),
        "instructions" => ("Instructions".into(), names_of("files")),
        "diagnostics" => ("Diagnostics".into(), names_of("files")),
        "skill_listing" => ("Skills available".into(), non_empty(names_of("names"))),
        "invoked_skills" => ("Skills used".into(), names_of("skills")),
        "deferred_tools_record" | "deferred_tools_delta" => {
            ("Deferred tools".into(), non_empty(names_of("addedNames")))
        }
        "agent_listing_delta" => ("Agents available".into(), non_empty(names_of("addedTypes"))),
        "mcp_instructions_delta" => ("MCP servers".into(), non_empty(names_of("addedNames"))),
        "command_permissions" => (
            "Command permissions".into(),
            non_empty(names_of("allowedTools")),
        ),
        "session_context" => ("Session context".into(), None),
        "prompt_snapshot" => ("Prompt snapshot".into(), None),
        // Bookkeeping with nothing to read.
        "credential_org" | "thinking_drop" | "remote_session_change" => return None,
        other => (
            {
                let mut n = other.replace('_', " ");
                if let Some(f) = n.get_mut(0..1) {
                    f.make_ascii_uppercase();
                }
                n
            },
            non_empty(str_of("text").or_else(|| str_of("content"))),
        ),
    })
}

/// One line of a Claude Code session file. Bookkeeping records (API telemetry, mode, titles,
/// snapshots) are dropped; messages become User / Assistant turns; hooks, attachments, meta
/// messages, durations and compaction markers become labelled `System` turns the app folds.
fn parse_claude_record(
    val: &serde_json::Value,
    turns: &mut Vec<TranscriptTurn>,
    auto_step: &mut u64,
    last_msg_id: &mut Option<String>,
) {
    if val.get("isSidechain").and_then(|v| v.as_bool()) == Some(true) {
        return;
    }
    let kind = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let timestamp = val
        .get("timestamp")
        .and_then(|v| v.as_str())
        .map(String::from);
    let mut push = |turns: &mut Vec<TranscriptTurn>,
                    turn_type,
                    content: Option<String>,
                    thinking: Option<String>,
                    tool_calls: Vec<TranscriptToolCall>,
                    label: Option<String>| {
        let step_index = *auto_step;
        *auto_step += 1;
        // The record's own uuid keeps a turn's id (and what the app remembers about it, like
        // "expanded") the same when the 500-line window of the live tail slides.
        let id = match val.get("uuid").and_then(|v| v.as_str()) {
            Some(u) if !u.is_empty() => format!("turn_{u}"),
            _ => format!("turn_{}", step_index),
        };
        turns.push(TranscriptTurn {
            id,
            step_index,
            turn_type,
            timestamp: timestamp.clone(),
            content,
            thinking,
            tool_calls,
            status: TranscriptStatus::Done,
            label,
        });
    };

    match kind {
        "user" | "assistant" => {
            let Some(msg) = val.get("message").filter(|m| m.is_object()) else {
                return;
            };
            let content_val = msg.get("content").unwrap_or(&serde_json::Value::Null);
            let mut thinking: Option<String> = None;
            let mut tool_calls = Vec::new();
            if let serde_json::Value::Array(blocks) = content_val {
                for block in blocks {
                    match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                        "thinking" => {
                            if let Some(th) = block.get("thinking").and_then(|v| v.as_str()) {
                                thinking = join_opt(thinking, Some(strip_ansi_codes(th.into())));
                            }
                        }
                        "tool_use" => {
                            let input = block.get("input").unwrap_or(&serde_json::Value::Null);
                            tool_calls.push(TranscriptToolCall {
                                call_id: block
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("tool_call")
                                    .to_string(),
                                tool_name: block
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("tool")
                                    .to_string(),
                                arguments_summary: summarize_arguments(input),
                                full_arguments_json: serde_json::to_string_pretty(input).ok(),
                                result: None,
                                is_error: false,
                                status: TranscriptStatus::Running,
                            });
                        }
                        "tool_result" => {
                            let Some(id) = block.get("tool_use_id").and_then(|v| v.as_str()) else {
                                continue;
                            };
                            let res = block
                                .get("content")
                                .and_then(text_of_blocks)
                                .map(strip_ansi_codes);
                            let is_err = block
                                .get("is_error")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if let Some(tc) = turns
                                .iter_mut()
                                .rev()
                                .flat_map(|t| t.tool_calls.iter_mut())
                                .find(|tc| tc.call_id == id)
                            {
                                tc.result = res;
                                tc.is_error = is_err;
                                tc.status = if is_err {
                                    TranscriptStatus::Error
                                } else {
                                    TranscriptStatus::Done
                                };
                            }
                        }
                        _ => {}
                    }
                }
            }
            let text = text_of_blocks(content_val).map(strip_ansi_codes);
            let text = text.filter(|t| !t.trim().is_empty());

            if kind == "user" {
                let Some(text) = text else { return };
                *last_msg_id = None;
                if val.get("isMeta").and_then(|v| v.as_bool()) == Some(true) {
                    push(
                        turns,
                        TranscriptTurnType::System,
                        Some(text),
                        None,
                        vec![],
                        Some("Service message".to_string()),
                    );
                } else {
                    push(
                        turns,
                        TranscriptTurnType::User,
                        Some(text),
                        None,
                        vec![],
                        None,
                    );
                }
                return;
            }

            // Claude writes each content block of one reply as its own record: join them.
            let msg_id = msg.get("id").and_then(|v| v.as_str()).map(String::from);
            if msg_id.is_some() && msg_id == *last_msg_id {
                if let Some(last) = turns
                    .iter_mut()
                    .rev()
                    .find(|t| t.turn_type == TranscriptTurnType::Assistant)
                {
                    last.content = join_opt(last.content.take(), text);
                    last.thinking = join_opt(last.thinking.take(), thinking);
                    last.tool_calls.extend(tool_calls);
                    return;
                }
            }
            if text.is_none() && thinking.is_none() && tool_calls.is_empty() {
                return;
            }
            *last_msg_id = msg_id;
            push(
                turns,
                TranscriptTurnType::Assistant,
                text,
                thinking,
                tool_calls,
                None,
            );
        }
        "system" => {
            let sub = val.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
            let text = val
                .get("content")
                .and_then(|v| v.as_str())
                .map(|c| strip_ansi_codes(c.to_string()))
                .filter(|c| !c.trim().is_empty());
            let (label, content) = match sub {
                "turn_duration" => (
                    duration_label(val.get("durationMs").and_then(|v| v.as_u64()).unwrap_or(0)),
                    None,
                ),
                "stop_hook_summary" => {
                    let infos: Vec<String> = val
                        .get("hookInfos")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|h| h.get("command").and_then(|c| c.as_str()))
                                .map(String::from)
                                .collect()
                        })
                        .unwrap_or_default();
                    let errors: Vec<String> = val
                        .get("hookErrors")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().map(|e| e.to_string()).collect())
                        .unwrap_or_default();
                    let mut body = infos.join("\n");
                    if !errors.is_empty() {
                        body.push_str("\nErrors:\n");
                        body.push_str(&errors.join("\n"));
                    }
                    ("Hook".to_string(), Some(body).filter(|b| !b.is_empty()))
                }
                "compact_boundary" => ("Context compacted".to_string(), text),
                "away_summary" => ("Summary while away".to_string(), text),
                "local_command" => ("Command output".to_string(), text),
                "" => ("System".to_string(), text),
                other => (other.replace('_', " "), text),
            };
            push(
                turns,
                TranscriptTurnType::System,
                content,
                None,
                vec![],
                Some(label),
            );
        }
        "attachment" => {
            let att = val.get("attachment").unwrap_or(&serde_json::Value::Null);
            // A message the person typed while the agent was working is their message, not a note
            // (a queued command of the agent's own, e.g. a background task, stays a note).
            let typed = att.get("type").and_then(|v| v.as_str()) == Some("queued_command")
                && att
                    .get("origin")
                    .and_then(|o| o.get("kind"))
                    .and_then(|k| k.as_str())
                    == Some("human");
            let prompt = att.get("prompt").and_then(|p| p.as_str()).unwrap_or("");
            if typed && !prompt.trim().is_empty() {
                push(
                    turns,
                    TranscriptTurnType::User,
                    Some(clip(&strip_ansi_codes(prompt.to_string()), 4000)),
                    None,
                    vec![],
                    None,
                );
            } else if let Some((label, body)) = describe_attachment(att) {
                push(
                    turns,
                    TranscriptTurnType::System,
                    body.map(|b| clip(&strip_ansi_codes(b), 1500)),
                    None,
                    vec![],
                    Some(label),
                );
            }
        }
        // api-request*, mode, last-prompt, ai-title, queue-operation, file-history-* ...
        _ => {}
    }
}

/// Parses a JSONL transcript payload into a normalized sequence of `TranscriptTurn`s.
#[uniffi::export]
pub fn parse_transcript_jsonl(raw_jsonl: String) -> Result<Vec<TranscriptTurn>, TranscriptError> {
    if raw_jsonl.len() > MAX_PAYLOAD_SIZE {
        return Err(TranscriptError::PayloadTooLarge {
            bytes: raw_jsonl.len() as u64,
        });
    }

    let mut turns: Vec<TranscriptTurn> = Vec::new();
    let mut auto_step: u64 = 1;
    let mut last_msg_id: Option<String> = None;

    for line in raw_jsonl.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let val: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => {
                // Skip malformed lines (e.g. partially written lines in active log append)
                continue;
            }
        };

        if !val.is_object() {
            continue;
        }

        // 0. Claude Code's own files: `type` in lower case and the message under `message`.
        if is_claude_record(&val) {
            parse_claude_record(&val, &mut turns, &mut auto_step, &mut last_msg_id);
            continue;
        }

        // 1. Check for Antigravity format (step_index, source, type)
        if val.get("step_index").is_some()
            || val.get("source").is_some()
            || val.get("type").is_some()
        {
            let step_index = val
                .get("step_index")
                .and_then(|v| v.as_u64())
                .unwrap_or(auto_step);
            auto_step = auto_step.max(step_index + 1);

            let source = val.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let step_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let status_str = val.get("status").and_then(|v| v.as_str()).unwrap_or("DONE");
            let timestamp = val
                .get("created_at")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let status = match status_str.to_uppercase().as_str() {
                "RUNNING" | "PENDING" => TranscriptStatus::Running,
                "ERROR" | "FAILED" => TranscriptStatus::Error,
                _ => TranscriptStatus::Done,
            };

            // If this is a tool response, try to link it to the last Assistant turn's tool call
            if source == "TOOL" || step_type == "TOOL_RESPONSE" || step_type == "GENERIC" {
                let tool_content = val
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| strip_agy_result_header(&strip_ansi_codes(s.to_string())));

                if let Some(last_turn) = turns
                    .iter_mut()
                    .rev()
                    .find(|t| t.turn_type == TranscriptTurnType::Assistant)
                {
                    if let Some(last_tool) = last_turn.tool_calls.last_mut() {
                        if last_tool.result.is_none() {
                            last_tool.result = tool_content;
                            last_tool.status = status;
                            last_tool.is_error = status == TranscriptStatus::Error;
                            continue;
                        }
                    }
                }
            }

            let turn_type = match step_type {
                "USER_INPUT" => TranscriptTurnType::User,
                "APPROVAL_REQUEST" => TranscriptTurnType::ApprovalRequest,
                "SYSTEM" | "SYSTEM_MESSAGE" | "CHECKPOINT" | "ERROR_MESSAGE" | "GENERIC" => {
                    TranscriptTurnType::System
                }
                _ => {
                    if source == "USER_EXPLICIT" {
                        TranscriptTurnType::User
                    } else if source == "MODEL" || step_type == "PLANNER_RESPONSE" {
                        TranscriptTurnType::Assistant
                    } else {
                        TranscriptTurnType::System
                    }
                }
            };

            let label = match step_type {
                "SYSTEM_MESSAGE" => Some("System message".to_string()),
                "CHECKPOINT" => Some("Checkpoint (context compacted)".to_string()),
                "ERROR_MESSAGE" => Some("Error".to_string()),
                "GENERIC" => Some("Tool output".to_string()),
                "SYSTEM" => Some("System".to_string()),
                _ => None,
            };
            let mut content = val
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| strip_ansi_codes(s.to_string()));
            if turn_type == TranscriptTurnType::User {
                content = content.map(|c| strip_agy_user_wrapper(&c));
            }
            if content.is_none() || label.as_deref() == Some("Error") {
                if let Some(e) = val.get("error").and_then(|v| v.as_str()) {
                    content = Some(strip_ansi_codes(e.to_string()));
                }
            }

            let thinking = val
                .get("thinking")
                .and_then(|v| v.as_str())
                .map(|s| strip_ansi_codes(s.to_string()));

            let mut tool_calls = Vec::new();
            if let Some(tc_array) = val.get("tool_calls").and_then(|v| v.as_array()) {
                for (idx, tc) in tc_array.iter().enumerate() {
                    let name = tc
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown_tool")
                        .to_string();
                    let call_id = tc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("tc_{}_{}", step_index, idx));
                    let raw_args = tc
                        .get("args")
                        .or_else(|| tc.get("arguments"))
                        .unwrap_or(&serde_json::Value::Null);
                    let args = &unquote_values(raw_args);
                    let summary = summarize_arguments(args);
                    let full_args = serde_json::to_string_pretty(args).ok();

                    tool_calls.push(TranscriptToolCall {
                        call_id,
                        tool_name: name,
                        arguments_summary: summary,
                        full_arguments_json: full_args,
                        result: None,
                        is_error: false,
                        status: TranscriptStatus::Running,
                    });
                }
            }

            turns.push(TranscriptTurn {
                id: format!("turn_{}", step_index),
                step_index,
                turn_type,
                timestamp,
                content,
                thinking,
                tool_calls,
                status,
                label,
            });
        }
        // 2. Check for Claude Code / generic role-based format
        else if let Some(role) = val.get("role").and_then(|v| v.as_str()) {
            let turn_type = match role {
                "user" => TranscriptTurnType::User,
                "assistant" => TranscriptTurnType::Assistant,
                "system" => TranscriptTurnType::System,
                _ => TranscriptTurnType::Assistant,
            };

            let timestamp = val
                .get("timestamp")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let mut text_content: Option<String> = None;
            let mut thinking_content: Option<String> = None;
            let mut tool_calls = Vec::new();

            if let Some(content_val) = val.get("content") {
                match content_val {
                    serde_json::Value::String(s) => {
                        text_content = Some(strip_ansi_codes(s.clone()));
                    }
                    serde_json::Value::Array(blocks) => {
                        for block in blocks {
                            let block_type =
                                block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match block_type {
                                "text" => {
                                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                                        let clean = strip_ansi_codes(t.to_string());
                                        text_content = match text_content {
                                            Some(prev) => Some(format!("{}\n{}", prev, clean)),
                                            None => Some(clean),
                                        };
                                    }
                                }
                                "thinking" => {
                                    if let Some(th) = block.get("thinking").and_then(|v| v.as_str())
                                    {
                                        let clean = strip_ansi_codes(th.to_string());
                                        thinking_content = match thinking_content {
                                            Some(prev) => Some(format!("{}\n{}", prev, clean)),
                                            None => Some(clean),
                                        };
                                    }
                                }
                                "tool_use" => {
                                    let id = block
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("tool_call")
                                        .to_string();
                                    let name = block
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("tool")
                                        .to_string();
                                    let input =
                                        block.get("input").unwrap_or(&serde_json::Value::Null);
                                    let summary = summarize_arguments(input);
                                    let full_args = serde_json::to_string_pretty(input).ok();

                                    tool_calls.push(TranscriptToolCall {
                                        call_id: id,
                                        tool_name: name,
                                        arguments_summary: summary,
                                        full_arguments_json: full_args,
                                        result: None,
                                        is_error: false,
                                        status: TranscriptStatus::Running,
                                    });
                                }
                                "tool_result" => {
                                    if let Some(tool_id) =
                                        block.get("tool_use_id").and_then(|v| v.as_str())
                                    {
                                        let res_text = block
                                            .get("content")
                                            .and_then(|v| v.as_str())
                                            .map(|s| strip_ansi_codes(s.to_string()));
                                        let is_err = block
                                            .get("is_error")
                                            .and_then(|v| v.as_bool())
                                            .unwrap_or(false);

                                        // Attach to matching previous tool call
                                        for prev_turn in turns.iter_mut().rev() {
                                            if let Some(tc) = prev_turn
                                                .tool_calls
                                                .iter_mut()
                                                .find(|tc| tc.call_id == tool_id)
                                            {
                                                tc.result = res_text.clone();
                                                tc.is_error = is_err;
                                                tc.status = if is_err {
                                                    TranscriptStatus::Error
                                                } else {
                                                    TranscriptStatus::Done
                                                };
                                                break;
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }

            let step_index = auto_step;
            auto_step += 1;

            turns.push(TranscriptTurn {
                id: format!("turn_{}", step_index),
                step_index,
                turn_type,
                timestamp,
                content: text_content,
                thinking: thinking_content,
                tool_calls,
                status: TranscriptStatus::Done,
                label: None,
            });
        }
    }

    Ok(turns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi_codes() {
        let colored = "\x1B[32mSuccess!\x1B[0m\x1B[1m Bold\x1B[0m";
        let cleaned = strip_ansi_codes(colored.to_string());
        assert_eq!(cleaned, "Success! Bold");
    }

    #[test]
    fn test_parse_antigravity_jsonl() {
        let jsonl = r#"
{"step_index": 1, "source": "USER_EXPLICIT", "type": "USER_INPUT", "status": "DONE", "created_at": "2026-10-08T15:00:00Z", "content": "build the project"}
{"step_index": 2, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "created_at": "2026-10-08T15:00:01Z", "thinking": "I need to run cargo build", "content": "Running build now...", "tool_calls": [{"name": "run_command", "arguments": {"CommandLine": "cargo build"}}]}
{"step_index": 3, "source": "TOOL", "type": "TOOL_RESPONSE", "status": "DONE", "content": "\u001b[32mFinished\u001b[0m dev [unoptimized + debuginfo]"}
"#;

        let turns = parse_transcript_jsonl(jsonl.to_string()).expect("Should parse successfully");
        assert_eq!(turns.len(), 2);

        // Turn 1: User
        assert_eq!(turns[0].step_index, 1);
        assert_eq!(turns[0].turn_type, TranscriptTurnType::User);
        assert_eq!(turns[0].content.as_deref(), Some("build the project"));

        // Turn 2: Assistant with thinking and tool call with attached result
        assert_eq!(turns[1].step_index, 2);
        assert_eq!(turns[1].turn_type, TranscriptTurnType::Assistant);
        assert_eq!(
            turns[1].thinking.as_deref(),
            Some("I need to run cargo build")
        );
        assert_eq!(turns[1].content.as_deref(), Some("Running build now..."));
        assert_eq!(turns[1].tool_calls.len(), 1);

        let tc = &turns[1].tool_calls[0];
        assert_eq!(tc.tool_name, "run_command");
        assert_eq!(tc.arguments_summary, "cargo build");
        assert_eq!(
            tc.result.as_deref(),
            Some("Finished dev [unoptimized + debuginfo]")
        );
        assert_eq!(tc.status, TranscriptStatus::Done);
    }

    #[test]
    fn test_parse_claude_code_jsonl() {
        let jsonl = r#"
{"role": "user", "content": "What is the status?"}
{"role": "assistant", "content": [{"type": "thinking", "thinking": "Let me check git status"}, {"type": "text", "text": "Checking repository..."}, {"type": "tool_use", "id": "call_git", "name": "Bash", "input": {"command": "git status -s"}}]}
{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "call_git", "content": "M crates/core/src/lib.rs", "is_error": false}]}
"#;

        let turns = parse_transcript_jsonl(jsonl.to_string()).expect("Should parse successfully");
        assert_eq!(turns.len(), 3);

        assert_eq!(turns[0].turn_type, TranscriptTurnType::User);
        assert_eq!(turns[0].content.as_deref(), Some("What is the status?"));

        assert_eq!(turns[1].turn_type, TranscriptTurnType::Assistant);
        assert_eq!(
            turns[1].thinking.as_deref(),
            Some("Let me check git status")
        );
        assert_eq!(turns[1].content.as_deref(), Some("Checking repository..."));
        assert_eq!(turns[1].tool_calls.len(), 1);

        let tc = &turns[1].tool_calls[0];
        assert_eq!(tc.call_id, "call_git");
        assert_eq!(tc.arguments_summary, "git status -s");
        assert_eq!(tc.result.as_deref(), Some("M crates/core/src/lib.rs"));
        assert_eq!(tc.status, TranscriptStatus::Done);
    }

    #[test]
    fn test_parse_claude_code_session_file() {
        let jsonl = r#"
{"type":"mode","mode":"normal"}
{"type":"api-request","id":"x"}
{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"t1"}
{"type":"user","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>x</local-command-caveat>"}}
{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"thinking","thinking":"hmm"}]}}
{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"Running"}]}}
{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"tool_use","id":"c1","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":[{"type":"text","text":"a.txt"}]}]}}
{"type":"assistant","isSidechain":true,"message":{"id":"s","role":"assistant","content":"sub"}}
{"type":"system","subtype":"turn_duration","durationMs":58030}
{"type":"attachment","attachment":{"type":"edited_text_file","filename":"/a/b"}}
{"type":"attachment","attachment":{"type":"total_tokens_reminder","text":"<total_tokens>14964699 tokens left</total_tokens>"}}
{"type":"attachment","attachment":{"type":"credential_org","organizationUuid":"x"}}
"#;
        let turns = parse_transcript_jsonl(jsonl.to_string()).unwrap();
        assert_eq!(turns.len(), 6);
        assert_eq!(turns[0].turn_type, TranscriptTurnType::User);
        assert_eq!(turns[0].content.as_deref(), Some("hi"));
        assert_eq!(turns[1].label.as_deref(), Some("Service message"));
        assert_eq!(turns[2].turn_type, TranscriptTurnType::Assistant);
        assert_eq!(turns[2].thinking.as_deref(), Some("hmm"));
        assert_eq!(turns[2].content.as_deref(), Some("Running"));
        assert_eq!(turns[2].tool_calls.len(), 1);
        assert_eq!(turns[2].tool_calls[0].result.as_deref(), Some("a.txt"));
        assert_eq!(turns[2].tool_calls[0].status, TranscriptStatus::Done);
        assert_eq!(turns[3].label.as_deref(), Some("Worked for 58s"));
        assert_eq!(turns[4].label.as_deref(), Some("File changed"));
        assert_eq!(turns[4].content.as_deref(), Some("/a/b"));
        assert_eq!(turns[5].label.as_deref(), Some("Tokens left"));
        assert_eq!(turns[5].content.as_deref(), Some("14,964,699 tokens left"));
    }

    #[test]
    fn test_parse_antigravity_real_shapes() {
        let jsonl = r#"
{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"<USER_REQUEST>\nbuild it\n</USER_REQUEST>"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"run_command","args":{"CommandLine":"\"ls -la\""}}]}
{"step_index":2,"source":"MODEL","type":"GENERIC","status":"DONE","content":"Created At: a\nCompleted At: b\n\nThe command exited with code 0.\nOutput:\nx"}
{"step_index":3,"source":"SYSTEM","type":"CHECKPOINT","content":"Resuming"}
{"step_index":4,"source":"SYSTEM","type":"ERROR_MESSAGE","error":"API error 503"}
"#;
        let turns = parse_transcript_jsonl(jsonl.to_string()).unwrap();
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[0].content.as_deref(), Some("build it"));
        assert_eq!(turns[1].tool_calls[0].arguments_summary, "ls -la");
        assert!(turns[1].tool_calls[0]
            .result
            .as_deref()
            .unwrap()
            .starts_with("The command exited"));
        assert_eq!(
            turns[2].label.as_deref(),
            Some("Checkpoint (context compacted)")
        );
        assert_eq!(turns[3].content.as_deref(), Some("API error 503"));
    }

    #[test]
    fn test_payload_too_large() {
        let large_payload = "a".repeat(MAX_PAYLOAD_SIZE + 10);
        let result = parse_transcript_jsonl(large_payload);
        assert!(matches!(
            result,
            Err(TranscriptError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn test_validate_transcript_path() {
        assert!(validate_transcript_path("/Users/test/transcript.jsonl").is_ok());
        assert!(validate_transcript_path("~/.gemini/transcript.jsonl").is_ok());
        assert!(validate_transcript_path("/etc/shadow").is_err());
        assert!(validate_transcript_path("~/.ssh/id_rsa").is_err());
        assert!(validate_transcript_path("/Users/test/.ssh/id_ed25519").is_err());
        assert!(validate_transcript_path("/Users/test/../../etc/passwd").is_err());
        assert!(validate_transcript_path("-f /etc/passwd").is_err());
        assert!(validate_transcript_path("path/with/'quotes").is_err());
        assert!(validate_transcript_path("").is_err());
    }

    #[test]
    fn test_malformed_lines_are_skipped() {
        let jsonl = "not a json line\n{\"step_index\": 1, \"source\": \"USER_EXPLICIT\", \"type\": \"USER_INPUT\", \"status\": \"DONE\", \"content\": \"hello\"}\n{broken json";
        let turns = parse_transcript_jsonl(jsonl.to_string()).expect("Should parse valid lines");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content.as_deref(), Some("hello"));
    }

    #[test]
    fn a_message_typed_while_the_agent_works_is_a_user_turn() {
        let raw = concat!(
            r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"собери билд","origin":{"kind":"human"},"humanTurn":true}}"#,
            "\n",
            r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"task done","origin":{"kind":"task-notification"}}}"#,
            "\n"
        );
        let turns = parse_transcript_jsonl(raw.to_string()).unwrap();
        assert_eq!(turns[0].turn_type, TranscriptTurnType::User);
        assert_eq!(turns[0].content.as_deref(), Some("собери билд"));
        assert_eq!(turns[1].turn_type, TranscriptTurnType::System);
    }
}
