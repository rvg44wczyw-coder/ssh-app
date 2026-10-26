use super::*;

/// The `type` values of the lines of a Codex rollout file (`~/.codex/sessions/Y/M/D/rollout-*.jsonl`):
/// `{timestamp, ordinal, type, payload}`.
const CODEX_TYPES: [&str; 7] = [
    "session_meta",
    "event_msg",
    "response_item",
    "turn_context",
    "world_state",
    "token_usage_record",
    "compacted",
];

/// A Codex rollout line: an object with a `payload` object and one of Codex's own record types. Checked
/// before the other shapes: its `type` alone would pass for a Claude Code or Antigravity record.
pub(super) fn is_codex_record(val: &serde_json::Value) -> bool {
    val.get("payload").is_some_and(|p| p.is_object())
        && val
            .get("type")
            .and_then(|t| t.as_str())
            .is_some_and(|t| CODEX_TYPES.contains(&t))
}

/// Text of an item's `content` array (`{type: "text" | "Text" | "input_text" | "output_text", text}`).
fn item_text(item: &serde_json::Value) -> Option<String> {
    let parts: Vec<&str> = item
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
        .collect();
    let text = strip_ansi_codes(parts.join("\n"));
    (!text.trim().is_empty()).then_some(text)
}

/// The text of a reasoning item's summary (empty when Codex keeps it encrypted).
fn reasoning_text(item: &serde_json::Value) -> Option<String> {
    let parts: Vec<String> = item
        .get("summary")?
        .as_array()?
        .iter()
        .filter_map(|s| {
            s.as_str()
                .or_else(|| s.get("text").and_then(|t| t.as_str()))
                .map(String::from)
        })
        .collect();
    let text = strip_ansi_codes(parts.join("\n"));
    (!text.trim().is_empty()).then_some(text)
}

/// The command a `CommandExecution` ran, as the person would type it: Codex's own parse of it, else the
/// last element of the argv (`["/bin/zsh", "-lc", "sleep 120"]`).
fn command_text(item: &serde_json::Value) -> String {
    let parsed = item
        .get("parsed_cmd")
        .and_then(|p| p.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|c| c.get("cmd").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join(" && ")
        })
        .filter(|s| !s.is_empty());
    let text = parsed.or_else(|| {
        item.get("command")
            .and_then(|c| c.as_array())
            .and_then(|a| a.last())
            .and_then(|v| v.as_str())
            .map(String::from)
    });
    strip_ansi_codes(text.unwrap_or_default())
}

/// One line of a Codex rollout. Only the `event_msg` records that carry the conversation are read: what
/// the person typed (`UserMessage`), what the agent said (`AgentMessage`), its reasoning summary, the
/// commands it ran (`CommandExecution`, the call and its result in one) and the end of a turn
/// (`task_complete`). The model's own side (`response_item`: instructions, the JavaScript wrapper of the
/// tools, encrypted reasoning), the context and token records are bookkeeping. Account and user ids in
/// `session_meta` are never read.
pub(super) fn parse_codex_record(
    val: &serde_json::Value,
    turns: &mut Vec<TranscriptTurn>,
    auto_step: &mut u64,
) {
    if val.get("type").and_then(|t| t.as_str()) != Some("event_msg") {
        return;
    }
    let Some(p) = val.get("payload") else { return };
    let timestamp = val
        .get("timestamp")
        .and_then(|v| v.as_str())
        .map(String::from);
    let mut push = |turns: &mut Vec<TranscriptTurn>,
                    key: &str,
                    turn_type,
                    content: Option<String>,
                    thinking: Option<String>,
                    tool_calls: Vec<TranscriptToolCall>,
                    label: Option<String>| {
        let step_index = *auto_step;
        *auto_step += 1;
        let id = if key.is_empty() {
            format!("turn_{step_index}")
        } else {
            format!("turn_codex_{key}")
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

    match p.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "item_completed" => {
            let Some(item) = p.get("item") else { return };
            let key = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
            match item.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "UserMessage" => {
                    if let Some(text) = item_text(item) {
                        push(
                            turns,
                            key,
                            TranscriptTurnType::User,
                            Some(text),
                            None,
                            vec![],
                            None,
                        );
                    }
                }
                "AgentMessage" => {
                    if let Some(text) = item_text(item) {
                        push(
                            turns,
                            key,
                            TranscriptTurnType::Assistant,
                            Some(text),
                            None,
                            vec![],
                            None,
                        );
                    }
                }
                "Reasoning" => {
                    if let Some(text) = reasoning_text(item) {
                        push(
                            turns,
                            key,
                            TranscriptTurnType::Assistant,
                            None,
                            Some(text),
                            vec![],
                            None,
                        );
                    }
                }
                "CommandExecution" => {
                    let command = command_text(item);
                    let status = item.get("status").and_then(|s| s.as_str()).unwrap_or("");
                    let exit = item.get("exit_code").and_then(|c| c.as_i64());
                    let running = matches!(status, "inProgress" | "in_progress" | "running");
                    let failed = !running && (status == "failed" || exit.is_some_and(|c| c != 0));
                    let output = item
                        .get("aggregated_output")
                        .and_then(|o| o.as_str())
                        .map(|o| clip(&strip_ansi_codes(o.to_string()), 20_000));
                    // Codex writes the folder as a `file://` URL.
                    let cwd = item.get("cwd").and_then(|c| c.as_str()).unwrap_or("");
                    let cwd =
                        strip_ansi_codes(cwd.strip_prefix("file://").unwrap_or(cwd).to_string());
                    let args = serde_json::json!({ "command": command, "cwd": cwd });
                    let call = TranscriptToolCall {
                        call_id: key.to_string(),
                        tool_name: "Bash".to_string(),
                        arguments_summary: summarize_arguments(&args),
                        full_arguments_json: serde_json::to_string_pretty(&args).ok(),
                        result: if running { None } else { output },
                        is_error: failed,
                        status: if running {
                            TranscriptStatus::Running
                        } else if failed {
                            TranscriptStatus::Error
                        } else {
                            TranscriptStatus::Done
                        },
                    };
                    push(
                        turns,
                        key,
                        TranscriptTurnType::Assistant,
                        None,
                        None,
                        vec![call],
                        None,
                    );
                }
                _ => {}
            }
        }
        "task_complete" => {
            let ms = p.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            push(
                turns,
                "",
                TranscriptTurnType::System,
                None,
                None,
                vec![],
                Some(duration_label(ms)),
            );
        }
        "turn_aborted" => {
            let ms = p.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            push(
                turns,
                "",
                TranscriptTurnType::System,
                None,
                None,
                vec![],
                Some(format!("Interrupted after {}s", ms / 1000)),
            );
        }
        _ => {}
    }
}
