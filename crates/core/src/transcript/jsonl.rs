use super::*;

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
