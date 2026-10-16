use crate::error::TranscriptError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const MAX_PAYLOAD_SIZE: usize = 5 * 1024 * 1024; // 5 MB

/// Regex for stripping ANSI escape sequences.
static ANSI_REGEX: OnceLock<Regex> = OnceLock::new();

fn get_ansi_regex() -> &'static Regex {
    ANSI_REGEX.get_or_init(|| {
        Regex::new(r"\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])").expect("Valid ANSI regex pattern")
    })
}

/// Strip ANSI escape codes from terminal / agent text.
#[uniffi::export]
pub fn strip_ansi_codes(input: String) -> String {
    get_ansi_regex().replace_all(&input, "").to_string()
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
}

/// Summarizes tool call arguments into a human-readable one-liner.
fn summarize_arguments(args: &serde_json::Value) -> String {
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
            if source == "TOOL" || step_type == "TOOL_RESPONSE" {
                let tool_content = val
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| strip_ansi_codes(s.to_string()));

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
                "SYSTEM" => TranscriptTurnType::System,
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

            let content = val
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| strip_ansi_codes(s.to_string()));

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
                    let args = tc.get("arguments").unwrap_or(&serde_json::Value::Null);
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
}
