use super::*;

/// The `type` values of the lines of Grok's `chat_history.jsonl`
/// (`~/.grok/sessions/<url-encoded folder>/<session uuid>/`): one record per message, no timestamps.
const GROK_TYPES: [&str; 5] = ["system", "user", "assistant", "reasoning", "tool_result"];

/// A Grok line: one of its record types and none of the fields the other agents' records carry (Claude's
/// `message`, Antigravity's `source` and `step_index`, Codex's `payload`).
pub(super) fn is_grok_record(val: &serde_json::Value) -> bool {
    let Some(t) = val.get("type").and_then(|t| t.as_str()) else {
        return false;
    };
    GROK_TYPES.contains(&t)
        && ["message", "source", "step_index", "payload"]
            .iter()
            .all(|k| val.get(k).is_none())
        && match t {
            "tool_result" => val.get("tool_call_id").is_some(),
            "reasoning" => val.get("summary").is_some(),
            "assistant" => val.get("model_id").is_some() || val.get("tool_calls").is_some(),
            "user" => val.get("content").is_some_and(|c| c.is_array()),
            // Claude's `system` records carry a `subtype`; Grok's is its prompt text
            _ => val.get("subtype").is_none() && val.get("content").is_some_and(|c| c.is_string()),
        }
}

/// What the person typed: Grok wraps it in `<user_query>…</user_query>`. The messages it adds itself
/// (`synthetic_reason`, the `<user_info>` block) are not the conversation.
fn user_text(val: &serde_json::Value) -> Option<String> {
    if val.get("synthetic_reason").is_some() {
        return None;
    }
    let parts: Vec<&str> = val
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
        .collect();
    let raw = parts.join("\n");
    if raw.trim_start().starts_with("<user_info>") {
        return None;
    }
    let inner = match (raw.find("<user_query>"), raw.rfind("</user_query>")) {
        (Some(a), Some(b)) if a + 12 <= b => &raw[a + 12..b],
        _ => raw.as_str(),
    };
    let text = strip_ansi_codes(inner.trim().to_string());
    (!text.is_empty()).then_some(text)
}

/// The name the app shows for one of Grok's tools (the family names the other agents' calls use).
fn tool_name(name: &str) -> String {
    match name {
        "run_terminal_command" => "Bash",
        "read_file" => "Read",
        "list_dir" => "LS",
        "grep" => "Grep",
        "search_replace" => "Edit",
        "write" => "Write",
        "web_fetch" => "WebFetch",
        "spawn_subagent" => "Task",
        other => return strip_ansi_codes(other.to_string()),
    }
    .to_string()
}

/// The arguments with the path and the query under the names `summarize_arguments` looks for.
fn call_arguments(raw: &str) -> serde_json::Value {
    let mut args: serde_json::Value = serde_json::from_str(raw).unwrap_or(serde_json::Value::Null);
    if let Some(map) = args.as_object_mut() {
        for (from, to) in [
            ("target_file", "path"),
            ("target_directory", "path"),
            ("pattern", "query"),
            ("file_path", "path"),
        ] {
            if !map.contains_key(to) {
                if let Some(v) = map.get(from).cloned() {
                    map.insert(to.to_string(), v);
                }
            }
        }
    }
    args
}

/// One line of Grok's chat history. The model's system prompt is not read; the reasoning is shown by its
/// summary only (the rest is encrypted); a `tool_result` goes to the call it names.
pub(super) fn parse_grok_record(
    val: &serde_json::Value,
    turns: &mut Vec<TranscriptTurn>,
    auto_step: &mut u64,
) {
    let kind = val.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let mut push = |turns: &mut Vec<TranscriptTurn>,
                    turn_type,
                    content: Option<String>,
                    thinking: Option<String>,
                    tool_calls: Vec<TranscriptToolCall>| {
        let step_index = *auto_step;
        *auto_step += 1;
        turns.push(TranscriptTurn {
            id: format!("turn_grok_{step_index}"),
            step_index,
            turn_type,
            timestamp: None,
            content,
            thinking,
            tool_calls,
            status: TranscriptStatus::Done,
            label: None,
        });
    };
    match kind {
        "user" => {
            if let Some(text) = user_text(val) {
                push(turns, TranscriptTurnType::User, Some(text), None, vec![]);
            }
        }
        "reasoning" => {
            let text: Vec<&str> = val
                .get("summary")
                .and_then(|s| s.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.get("text").and_then(|t| t.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            let text = strip_ansi_codes(text.join("\n"));
            if !text.trim().is_empty() {
                push(
                    turns,
                    TranscriptTurnType::Assistant,
                    None,
                    Some(text),
                    vec![],
                );
            }
        }
        "assistant" => {
            let content = val
                .get("content")
                .and_then(|c| c.as_str())
                .map(|c| strip_ansi_codes(c.to_string()))
                .filter(|c| !c.trim().is_empty());
            let calls: Vec<TranscriptToolCall> = val
                .get("tool_calls")
                .and_then(|c| c.as_array())
                .map(|a| {
                    a.iter()
                        .map(|c| {
                            let raw = c.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                            let args = call_arguments(raw);
                            TranscriptToolCall {
                                call_id: c
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                tool_name: tool_name(
                                    c.get("name").and_then(|v| v.as_str()).unwrap_or("tool"),
                                ),
                                arguments_summary: summarize_arguments(&args),
                                full_arguments_json: serde_json::to_string_pretty(&args).ok(),
                                result: None,
                                is_error: false,
                                status: TranscriptStatus::Running,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            if content.is_some() || !calls.is_empty() {
                push(turns, TranscriptTurnType::Assistant, content, None, calls);
            }
        }
        "tool_result" => {
            let id = val
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let out = val
                .get("content")
                .and_then(|c| c.as_str())
                .map(|c| clip(&strip_ansi_codes(c.to_string()), 20_000));
            if let Some(call) = turns
                .iter_mut()
                .rev()
                .flat_map(|t| t.tool_calls.iter_mut())
                .find(|c| c.call_id == id)
            {
                // a terminal command's result starts with `exit: <code>`
                let failed = out
                    .as_deref()
                    .and_then(|o| o.strip_prefix("exit: "))
                    .and_then(|r| r.split_whitespace().next())
                    .is_some_and(|code| code != "0");
                call.result = out;
                call.is_error = failed;
                call.status = if failed {
                    TranscriptStatus::Error
                } else {
                    TranscriptStatus::Done
                };
            }
        }
        _ => {}
    }
}
