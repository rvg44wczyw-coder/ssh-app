use super::*;

/// Whether a session file is the one of the conversation on the screen: one of the last five things
/// you said in the file is on the screen. Two agents of the same kind can work in the same folder
/// (one on the laptop), and the newest file may be the other one's; its thoughts must not be shown
/// here. A file without a user turn (a session just started) cannot be told apart and is accepted.
#[uniffi::export]
pub fn transcript_matches_screen(file: Vec<TranscriptTurn>, screen: String) -> bool {
    let screen = squash(&screen);
    let said: Vec<String> = file
        .iter()
        .filter(|t| t.turn_type == TranscriptTurnType::User)
        .filter_map(|t| t.content.as_deref())
        .map(|c| squash(c).chars().take(40).collect::<String>())
        .filter(|c| c.chars().count() >= 3)
        .collect();
    if said.is_empty() {
        return true;
    }
    said.iter()
        .rev()
        .take(5)
        .any(|c| screen.contains(c.as_str()))
}

/// An expanded thought block (`▾ Thought for 25s`) prints the thinking as indented text, and the
/// reply follows with no blank line, so the screen alone cannot tell where one ends. The session
/// file has both apart: the thought text is dropped (merge puts the file's thinking back where it
/// belongs) and, when the step also has a reply, the screen text is cut where that reply starts.
pub(super) fn drop_expanded_thoughts(
    screen: Vec<TranscriptTurn>,
    file: &[TranscriptTurn],
) -> Vec<TranscriptTurn> {
    let steps: Vec<(String, Option<String>)> = file
        .iter()
        .filter(|t| t.turn_type == TranscriptTurnType::Assistant)
        .filter_map(|t| {
            let k = squash(t.thinking.as_deref()?);
            (k.chars().count() >= 20).then(|| {
                let reply = t.content.as_deref().map(squash).filter(|c| !c.is_empty());
                (k.chars().take(40).collect::<String>(), reply)
            })
        })
        .collect();
    // A result note that swallowed the thought printed under it (the header line is missing):
    // keep the first line, the rest is the file's thinking.
    let screen: Vec<TranscriptTurn> = screen
        .into_iter()
        .map(|mut t| {
            if t.turn_type == TranscriptTurnType::System && t.label.as_deref() == Some("Note") {
                if let Some((head, rest)) = t.content.as_deref().and_then(|c| c.split_once('\n')) {
                    let rest = squash(rest);
                    if steps.iter().any(|(k, _)| rest.starts_with(k.as_str())) {
                        t.content = Some(head.to_string());
                    }
                }
            }
            t
        })
        .collect();
    let mut out: Vec<TranscriptTurn> = Vec::with_capacity(screen.len());
    let mut i = 0;
    while i < screen.len() {
        let header = &screen[i];
        let is_header = header.turn_type == TranscriptTurnType::System
            && header
                .label
                .as_deref()
                .is_some_and(|l| l.starts_with("Thought for"));
        let next = screen.get(i + 1).filter(|n| {
            n.turn_type == TranscriptTurnType::Assistant
                && n.thinking.is_none()
                && n.tool_calls.is_empty()
        });
        if let (true, Some(next)) = (is_header, next) {
            let text = next.content.clone().unwrap_or_default();
            let flat = squash(&text);
            if let Some((_, reply)) = steps.iter().find(|(k, _)| flat.starts_with(k.as_str())) {
                match reply {
                    None => {}
                    Some(reply) => {
                        let want: String = reply.chars().take(40).collect();
                        let cut = text
                            .char_indices()
                            .filter(|(at, _)| *at > 0 && text[..*at].ends_with(char::is_whitespace))
                            .map(|(at, _)| at)
                            .find(|at| squash(&text[*at..]).starts_with(want.as_str()));
                        let mut t = next.clone();
                        if let Some(at) = cut {
                            t.content = Some(text[at..].trim().to_string());
                        }
                        out.push(t);
                    }
                }
                i += 2;
                continue;
            }
        }
        out.push(screen[i].clone());
        i += 1;
    }
    out
}

/// A message of several paragraphs is drawn by the agent as its first paragraph at the prompt and
/// the rest as indented text after a blank line, which reads like a reply. The session file has the
/// message whole: paragraphs on the screen that continue a user turn exactly as the file's message
/// goes on are taken back into that user turn.
pub(super) fn absorb_user_paragraphs(
    screen: Vec<TranscriptTurn>,
    file: &[TranscriptTurn],
) -> Vec<TranscriptTurn> {
    let said: Vec<String> = file
        .iter()
        .filter(|t| t.turn_type == TranscriptTurnType::User)
        .filter_map(|t| t.content.as_deref())
        .map(squash)
        .collect();
    let plain_text = |t: &TranscriptTurn| {
        t.turn_type == TranscriptTurnType::Assistant
            && t.thinking.is_none()
            && t.tool_calls.is_empty()
            && t.content.as_deref().is_some_and(|c| !c.trim().is_empty())
    };
    let mut out: Vec<TranscriptTurn> = Vec::with_capacity(screen.len());
    let mut i = 0;
    while i < screen.len() {
        let mut turn = screen[i].clone();
        i += 1;
        if turn.turn_type == TranscriptTurnType::User {
            let head = squash(turn.content.as_deref().unwrap_or(""));
            if !head.is_empty() {
                if let Some(full) = said
                    .iter()
                    .rev()
                    .find(|m| m.len() > head.len() && m.starts_with(&head))
                {
                    let mut rest = full[head.len()..].trim_start().to_string();
                    let mut text = turn.content.clone().unwrap_or_default();
                    while i < screen.len() && plain_text(&screen[i]) {
                        let para = screen[i].content.clone().unwrap_or_default();
                        let sq = squash(&para);
                        if sq.is_empty() || !rest.starts_with(&sq) {
                            break;
                        }
                        rest = rest[sq.len()..].trim_start().to_string();
                        text.push_str("\n\n");
                        text.push_str(para.trim());
                        i += 1;
                    }
                    turn.content = Some(text);
                }
            }
        }
        out.push(turn);
    }
    out
}

/// Thinking whose turn is not found goes before the next one that is; thinking after the last
/// match is dropped rather than put in a wrong place.
#[uniffi::export]
pub fn merge_thinking(
    screen: Vec<TranscriptTurn>,
    file: Vec<TranscriptTurn>,
) -> Vec<TranscriptTurn> {
    let screen = absorb_user_paragraphs(screen, &file);
    let screen = drop_expanded_thoughts(screen, &file);
    let mut before: Vec<Vec<String>> = vec![Vec::new(); screen.len() + 1];
    let mut pending: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    // Calls the screen shows only as a folded group ("Searched for 1 pattern (ctrl+o to expand)")
    // come from the file, to the group before the next matched turn.
    let mut group_calls: Vec<TranscriptToolCall> = Vec::new();
    let mut groups: std::collections::HashMap<usize, Vec<TranscriptToolCall>> =
        std::collections::HashMap::new();
    // The screen cuts a call's arguments and result ("… +2 lines"); the file has them whole, so the
    // app can show the diff of an Edit and the full result.
    let mut enrich: std::collections::HashMap<usize, TranscriptToolCall> =
        std::collections::HashMap::new();
    let is_folded = |t: &TranscriptTurn| {
        t.turn_type == TranscriptTurnType::Assistant
            && t.tool_calls.first().is_some_and(|c| {
                c.result.is_none()
                    && (c.arguments_summary.is_empty()
                        || c.tool_name.starts_with("Ran ")
                        || c.tool_name.contains(" more action"))
            })
    };
    for f in file
        .iter()
        .filter(|f| f.turn_type == TranscriptTurnType::Assistant)
    {
        if let Some(th) = f.thinking.as_ref().filter(|t| !t.trim().is_empty()) {
            pending.push(th.clone());
        }
        let text = f
            .content
            .as_deref()
            .map(|c| norm(c, 40))
            .filter(|c| c.chars().count() >= 8);
        let tool = f
            .tool_calls
            .first()
            .map(|t| (t.tool_name.clone(), norm(&t.arguments_summary, 24)));
        if text.is_none() && tool.is_none() {
            continue;
        }
        let found = (cursor..screen.len()).find(|&j| {
            let s = &screen[j];
            if s.turn_type != TranscriptTurnType::Assistant {
                return false;
            }
            if let (Some(t), Some(c)) = (&text, s.content.as_deref()) {
                let sc = norm(c, 40);
                return sc.starts_with(t.as_str()) || t.starts_with(sc.as_str()) && sc.len() >= 8;
            }
            if text.is_none() {
                if let (Some((name, args)), Some(sc)) = (&tool, s.tool_calls.first()) {
                    let sa = norm(&sc.arguments_summary, 24);
                    return tool_class(&sc.tool_name) == tool_class(name)
                        && (sa.starts_with(args.as_str()) || args.starts_with(sa.as_str()));
                }
            }
            false
        });
        if let Some(j) = found {
            before[j].append(&mut pending);
            if text.is_none() {
                if let Some(c) = f.tool_calls.first() {
                    enrich.insert(j, c.clone());
                }
            }
            if !group_calls.is_empty() {
                if let Some(g) = (cursor..j).rev().find(|&g| is_folded(&screen[g])) {
                    groups.entry(g).or_default().append(&mut group_calls);
                }
                group_calls.clear();
            }
            cursor = j + 1;
        } else if text.is_none() {
            group_calls.extend(
                f.tool_calls
                    .iter()
                    .filter(|c| FOLDABLE_TOOLS.contains(&c.tool_name.as_str()))
                    .cloned(),
            );
        }
    }
    if !group_calls.is_empty() {
        if let Some(g) = (cursor..screen.len())
            .rev()
            .find(|&g| is_folded(&screen[g]))
        {
            groups.entry(g).or_default().append(&mut group_calls);
        }
    }
    let mut out = Vec::with_capacity(screen.len());
    for (j, mut t) in screen.into_iter().enumerate() {
        if let (Some(calls), Some(c)) = (groups.remove(&j), t.tool_calls.first_mut()) {
            describe_group(c, &calls);
        }
        if let (Some(f), Some(c)) = (enrich.remove(&j), t.tool_calls.first_mut()) {
            if c.full_arguments_json.is_none() {
                c.full_arguments_json = f.full_arguments_json;
            }
            let cut = c
                .result
                .as_deref()
                .is_some_and(|r| r.contains("ctrl+o to expand") || r.contains("… +"));
            if cut && f.result.is_some() {
                c.result = f.result;
            }
        }
        for (k, th) in before[j].drain(..).enumerate() {
            out.push(TranscriptTurn {
                id: format!("think_{}_{}", t.id, k),
                step_index: t.step_index,
                turn_type: TranscriptTurnType::Assistant,
                timestamp: None,
                content: None,
                thinking: Some(th),
                tool_calls: vec![],
                status: TranscriptStatus::Done,
                label: None,
            });
        }
        out.push(t);
    }
    out
}
