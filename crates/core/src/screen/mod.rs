//! The agent feed built from the tmux screen (`tmux capture-pane -p -J -S -`) instead of the
//! agent's session file. Pure text in, typed turns out; the capture itself is `host_api`.
//!
//! Two agents are understood, each by the markers its terminal UI prints:
//! - Claude Code: `⏺` reply / tool call, `❯` your input, `⎿` result, `✻` turn summary, folded
//!   tool groups ("Searched for 1 pattern (ctrl+o to expand)");
//! - Antigravity CLI (agy): `>` your input, `○ Thought for …`, `● Ran 5 commands (…)`, indented
//!   reply text, `──── Conversation compacted ────`.
//!
//! Every element of the screen becomes its own turn, in screen order, so the order of the story
//! (text, tool, thinking) is never rearranged. What is not recognised becomes a folded
//! "Terminal output" turn, never dropped. The input box, the status bar and the spinner at the
//! bottom are not turns: they are reported as `draft` and `status`.

use crate::transcript::{TranscriptStatus, TranscriptToolCall, TranscriptTurn, TranscriptTurnType};

/// One choice of a menu the agent is waiting on.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PromptOption {
    pub number: u32,
    pub label: String,
    /// Grey explanation under the label (Claude's question form); empty when there is none.
    pub description: String,
    /// Ticked box of a multi-choice menu.
    pub checked: bool,
}

/// A question the agent asks on screen and waits for: a permission ("Run this command?" with
/// Yes / Always / No) or a choice (single or with boxes). The app shows it as a card with buttons
/// and answers with arrow keys, Space and Enter.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScreenPrompt {
    pub title: Option<String>,
    /// What is being asked about (the command, the file), without the lead-in line.
    pub detail: String,
    pub question: String,
    pub options: Vec<PromptOption>,
    /// Index in `options` of the entry the cursor is on.
    pub selected: u32,
    /// Boxes to tick (Space toggles, Enter confirms) instead of one answer.
    pub multi: bool,
}

/// The screen read as a feed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScreenFeed {
    /// `Unknown`: nothing recognised, one folded "Terminal output" turn.
    pub dialect: ScreenDialect,
    pub turns: Vec<TranscriptTurn>,
    /// The live line of the agent ("✶ Thinking… (12s)"), if one is on screen.
    pub status: Option<String>,
    /// What is typed in the input box and not sent yet.
    pub draft: Option<String>,
    /// A question on screen that waits for an answer.
    pub prompt: Option<ScreenPrompt>,
}

/// Which agent's terminal UI the screen was recognised as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ScreenDialect {
    Claude,
    Agy,
    Unknown,
}

/// Parses a capture of a pane. Cheap enough to run on every poll.
#[uniffi::export]
pub fn parse_screen_feed(text: String) -> ScreenFeed {
    // The pane is untrusted display data: no escape sequences or control characters get through.
    let text = crate::transcript::strip_ansi_codes(text);
    let mut lines: Vec<String> = text.lines().map(|l| l.trim_end().to_string()).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let dialect = detect(&lines);

    // A menu replaces the input box: cut it off before anything else reads the lines.
    let prompt = match parse_prompt(&lines) {
        Some((start, p)) => {
            lines.truncate(start);
            Some(p)
        }
        None => None,
    };
    let (footer, draft) = if prompt.is_some() {
        (lines.len(), None)
    } else {
        split_footer(&lines)
    };
    lines.truncate(footer);
    let status = take_status(&mut lines);
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    let mut b = Builder::default();
    match dialect {
        ScreenDialect::Claude => claude(&lines, &mut b),
        ScreenDialect::Agy => agy(&join_thought_wraps(&lines), &mut b),
        ScreenDialect::Unknown => {
            let tail: Vec<&str> = lines.iter().map(String::as_str).collect();
            let keep = tail.len().saturating_sub(300);
            b.system("Terminal output", &tail[keep..].join("\n"));
        }
    }
    ScreenFeed {
        dialect,
        turns: b.turns,
        status,
        draft,
        prompt,
    }
}

fn detect(lines: &[String]) -> ScreenDialect {
    let mut claude = 0;
    let mut agy = 0;
    for l in lines {
        // The footer is on screen even when the part of the pane that scrolled by has no marker.
        if l.contains("⏵⏵") || l.contains("shift+tab to cycle") {
            claude += 1;
        } else if l.starts_with("? for shortcuts")
            || (l.contains(" · Gemini ") && l.contains("accept-edits"))
        {
            agy += 1;
        } else if l.starts_with("⏺ ") || l.starts_with("❯ ") || l.starts_with("✻ ") {
            claude += 1;
        } else if l.starts_with("○ ")
            || l.starts_with("● ")
            || l.contains("Antigravity CLI")
            || l.contains("Conversation compacted")
        {
            agy += 1;
        }
    }
    if claude == 0 && agy == 0 {
        ScreenDialect::Unknown
    } else if claude >= agy {
        ScreenDialect::Claude
    } else {
        ScreenDialect::Agy
    }
}

fn is_rule(l: &str) -> bool {
    l.chars().count() >= 10 && l.chars().all(|c| c == '─')
}

fn indent(l: &str) -> usize {
    l.chars().take_while(|c| *c == ' ').count()
}

/// The input box and status bar at the bottom: two full-width rules close together. Returns
/// where the footer starts and what is typed in the box.
fn split_footer(lines: &[String]) -> (usize, Option<String>) {
    // Ctrl+O's detailed transcript has no input box: a rule and one status line close it.
    if let Some(last) = lines.iter().rposition(|l| !l.trim().is_empty()) {
        if lines[last].contains("Showing detailed transcript") {
            let cut = if last > 0 && is_rule(&lines[last - 1]) {
                last - 1
            } else {
                last
            };
            return (cut, None);
        }
    }
    let rules: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_rule(l))
        .map(|(i, _)| i)
        .collect();
    if let [.., s1, s2] = rules[..] {
        if s2 > s1 && s2 - s1 <= 8 {
            let draft: Vec<String> = lines[s1 + 1..s2]
                .iter()
                .map(|l| {
                    let t = l.trim_start();
                    t.strip_prefix('❯')
                        .or_else(|| t.strip_prefix('>'))
                        .unwrap_or(t)
                        .trim()
                        .to_string()
                })
                .filter(|l| !l.is_empty() && !is_box_hint(l))
                .collect();
            let draft = (!draft.is_empty()).then(|| draft.join("\n"));
            return (s1, draft);
        }
    }
    (lines.len(), None)
}

/// The agent's own hint inside the input box ("Press up to edit queued messages"), not a draft.
fn is_box_hint(l: &str) -> bool {
    let l = l.to_lowercase();
    l.contains("to edit queued messages") || l.starts_with("press up") || l.starts_with("press ↑")
}

/// The spinner line just above the input box ("✶ Thinking… (12s · esc to interrupt)").
fn take_status(lines: &mut Vec<String>) -> Option<String> {
    let idx = lines.iter().rposition(|l| !l.is_empty())?;
    let l = lines[idx].clone();
    let mut chars = l.chars();
    let first = chars.next()?;
    let rest: String = chars.collect();
    let spinner = "✶✳✢✽·✻*⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";
    if spinner.contains(first)
        && rest.starts_with(' ')
        && rest.contains('…')
        && (rest.contains("esc to interrupt")
            || rest.trim_end().ends_with('…')
            || rest.contains("…)")
            // newer Claude: "✶ Creating… (4m 30s · ↓ 4.9k tokens)", no interrupt hint
            || rest.contains("… ("))
    {
        lines.truncate(idx);
        return Some(l.trim().to_string());
    }
    None
}

#[derive(Default)]
struct Builder {
    turns: Vec<TranscriptTurn>,
}

impl Builder {
    fn push(
        &mut self,
        turn_type: TranscriptTurnType,
        content: Option<String>,
        thinking: Option<String>,
        tool_calls: Vec<TranscriptToolCall>,
        label: Option<String>,
    ) {
        let n = self.turns.len() as u64;
        self.turns.push(TranscriptTurn {
            id: format!("scr{n}"),
            step_index: n,
            turn_type,
            timestamp: None,
            content,
            thinking,
            tool_calls,
            status: TranscriptStatus::Done,
            label,
        });
    }

    fn user(&mut self, text: String) {
        if !text.trim().is_empty() {
            self.push(TranscriptTurnType::User, Some(text), None, vec![], None);
        }
    }

    fn text(&mut self, text: String) {
        if !text.trim().is_empty() {
            self.push(
                TranscriptTurnType::Assistant,
                Some(text),
                None,
                vec![],
                None,
            );
        }
    }

    fn thinking(&mut self, text: String) {
        self.push(
            TranscriptTurnType::Assistant,
            None,
            Some(text),
            vec![],
            None,
        );
    }

    fn system(&mut self, label: &str, body: &str) {
        let body = body.trim();
        self.push(
            TranscriptTurnType::System,
            (!body.is_empty()).then(|| body.to_string()),
            None,
            vec![],
            Some(label.to_string()),
        );
    }

    fn tool(&mut self, name: String, args: String, result: Option<String>) {
        let id = format!("scr{}", self.turns.len());
        let running = result
            .as_deref()
            .is_some_and(|r| r.trim_start().starts_with("Running…"));
        let call = TranscriptToolCall {
            call_id: id,
            tool_name: name,
            arguments_summary: args,
            full_arguments_json: None,
            result,
            is_error: false,
            status: if running {
                TranscriptStatus::Running
            } else {
                TranscriptStatus::Done
            },
        };
        self.push(TranscriptTurnType::Assistant, None, None, vec![call], None);
    }
}

// MARK: questions the agent waits on

mod prompt;
use prompt::*;
mod text;
use text::*;
mod claude;
use claude::*;
mod agy;
use agy::*;
mod tools;
use tools::*;
mod merge;
pub use merge::{merge_thinking, transcript_matches_screen};
#[cfg(test)]
mod tests;
