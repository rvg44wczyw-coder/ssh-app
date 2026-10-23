use super::*;

/// Antigravity wraps the user's request in `<USER_REQUEST>…</USER_REQUEST>`.
pub(super) fn strip_agy_user_wrapper(c: &str) -> String {
    let t = c.trim();
    t.strip_prefix("<USER_REQUEST>")
        .map(|r| r.strip_suffix("</USER_REQUEST>").unwrap_or(r))
        .unwrap_or(t)
        .trim()
        .to_string()
}

/// Antigravity tool results start with "Created At: …\nCompleted At: …\n\n"; the header is noise.
pub(super) fn strip_agy_result_header(c: &str) -> String {
    if c.starts_with("Created At:") {
        if let Some((_, rest)) = c.split_once("\n\n") {
            return rest.trim_start_matches('\n').to_string();
        }
    }
    c.to_string()
}

/// Antigravity stores argument values as JSON-encoded strings (`"\"ls -la\""`); unwrap them.
pub(super) fn unquote_values(v: &serde_json::Value) -> serde_json::Value {
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
pub(super) fn is_claude_record(val: &serde_json::Value) -> bool {
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

pub(super) fn text_of_blocks(content: &serde_json::Value) -> Option<String> {
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

pub(super) fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('…');
        t
    }
}

pub(super) fn join_opt(prev: Option<String>, add: Option<String>) -> Option<String> {
    match (prev, add) {
        (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
        (a, None) => a,
        (None, b) => b,
    }
}

pub(super) fn duration_label(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 60 {
        format!("Worked for {}m {}s", s / 60, s % 60)
    } else {
        format!("Worked for {s}s")
    }
}

/// Names for Claude Code's `attachment` records (what it adds to the model's context) and the
/// one line worth showing for each. `None`: nothing to show.
pub(super) fn describe_attachment(att: &serde_json::Value) -> Option<(String, Option<String>)> {
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
pub(super) fn parse_claude_record(
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
        // "expanded") the same when the window of the live tail slides.
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
        // The permission mode the session runs in (default, plan, acceptEdits, auto, bypassPermissions
        // ...): kept as a hidden service turn labelled `permission-mode`, the app shows the last one
        // in its status line and leaves these out of the feed.
        "permission-mode" => {
            let mode = val
                .get("permissionMode")
                .and_then(|v| v.as_str())
                .filter(|m| {
                    !m.is_empty()
                        && m.len() <= 32
                        && m.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                });
            let repeated = turns.last().is_some_and(|t| {
                t.label.as_deref() == Some("permission-mode") && t.content.as_deref() == mode
            });
            if let (Some(mode), false) = (mode, repeated) {
                push(
                    turns,
                    TranscriptTurnType::System,
                    Some(mode.to_string()),
                    None,
                    vec![],
                    Some("permission-mode".to_string()),
                );
            }
        }
        // api-request*, last-prompt, ai-title, queue-operation, file-history-* ...
        _ => {}
    }
}
