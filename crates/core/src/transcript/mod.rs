use crate::error::SshCoreError;
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
            // A subagent call (`Task` / `Agent`): its kind and short description, not the whole
            // prompt it carries.
            if let Some(desc) = map.get("description").and_then(|v| v.as_str()) {
                if map.contains_key("prompt") || map.contains_key("subagent_type") {
                    return match map.get("subagent_type").and_then(|v| v.as_str()) {
                        Some(kind) => format!("{kind}: {desc}"),
                        None => desc.to_string(),
                    };
                }
            }
            // Fallback: compact JSON of first few keys
            serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
        }
        serde_json::Value::String(s) => s.clone(),
        _ => serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string()),
    }
}

mod subagents;
pub use subagents::*;
mod path;
pub use path::*;
mod claude;
use claude::*;
mod jsonl;
pub use jsonl::*;
#[cfg(test)]
mod tests;
