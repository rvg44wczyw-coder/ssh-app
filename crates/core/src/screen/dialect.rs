//! The screen parsers of the agents behind one interface (docs/features/60-universal-agent-feed.md). A
//! dialect says how to tell its agent's terminal UI from others (a score per line), which overlay or dialog
//! it draws over the pane, how its conversation lines become turns and how it draws inline code. The
//! footer and the spinner line are shared (`split_footer`, `take_status`): they know the markers of every
//! dialect at once. A new agent adds one `DialectParser` here and is listed in `DIALECTS`; `parse_screen_feed`
//! and the styling code do not change.

use super::*;

/// What a dialect found drawn over the bottom of the pane, and where the lines above it end.
pub(super) enum Overlay {
    None,
    /// The `/btw` side answer: the conversation ends at `usize`.
    Aside(usize, ScreenAside),
    /// A full-screen dialog of a slash command.
    Dialog(usize, ScreenDialog),
}

pub(super) trait DialectParser: Sync {
    fn dialect(&self) -> ScreenDialect;
    /// How much one screen line speaks for this agent (0: not at all). The dialect with the highest
    /// total wins; a tie goes to the dialect listed first in `DIALECTS`; all zero is `Unknown`.
    fn line_score(&self, line: &str) -> u32;
    /// A dialog or overlay drawn over the pane. The lines are cut at the returned index before anything
    /// else reads them, so a dialog's numbered list is never mistaken for a question of the agent.
    fn overlay(&self, _lines: &[String]) -> Overlay {
        Overlay::None
    }
    /// Where the footer (input box and status lines) starts and the draft typed in it, for an agent whose
    /// footer is not the pair of rules the shared `split_footer` looks for.
    fn footer(&self, _lines: &[String]) -> Option<(usize, Option<String>)> {
        None
    }
    /// A question the agent waits on, when its dialog is not the numbered list the shared `parse_prompt` reads.
    fn prompt(&self, _lines: &[String]) -> Option<(usize, ScreenPrompt)> {
        None
    }
    /// The conversation above the footer as turns, in screen order.
    fn turns(&self, lines: &[String], b: &mut Builder);
    /// The foreground colour parameters this agent draws inline code with.
    fn is_code_colour(&self, _params: &[u32]) -> bool {
        false
    }
    /// The agent draws a heading as a bold line of its own (the styling turns it into `###`).
    fn bold_line_is_heading(&self) -> bool {
        false
    }
}

struct ClaudeDialect;
struct AgyDialect;
struct CodexDialect;
struct GrokDialect;
struct UnknownDialect;

impl DialectParser for ClaudeDialect {
    fn dialect(&self) -> ScreenDialect {
        ScreenDialect::Claude
    }

    fn line_score(&self, l: &str) -> u32 {
        // The footer is on screen even when the part of the pane that scrolled by has no marker.
        (l.contains("⏵⏵")
            || l.contains("shift+tab to cycle")
            || (!is_agy_line(l)
                && (l.starts_with("⏺ ") || l.starts_with("❯ ") || l.starts_with("✻ "))))
            as u32
    }

    fn overlay(&self, lines: &[String]) -> Overlay {
        if let Some((start, a)) = parse_aside(lines) {
            Overlay::Aside(start, a)
        } else if let Some((start, d)) = parse_dialog(lines) {
            Overlay::Dialog(start.saturating_sub(1), d)
        } else {
            Overlay::None
        }
    }

    fn turns(&self, lines: &[String], b: &mut Builder) {
        claude(lines, b)
    }

    fn is_code_colour(&self, params: &[u32]) -> bool {
        // 256-colour 153.
        params == [38, 5, 153]
    }

    fn bold_line_is_heading(&self) -> bool {
        true
    }
}

/// A line one of the stronger Antigravity rules claims; Claude's weaker line markers give way to it.
fn is_agy_line(l: &str) -> bool {
    l.starts_with("Keyboard: ↑/↓")
        || l.starts_with("? for shortcuts")
        || (l.contains(" · Gemini ") && l.contains("accept-edits"))
}

impl DialectParser for AgyDialect {
    fn dialect(&self) -> ScreenDialect {
        ScreenDialect::Agy
    }

    fn line_score(&self, l: &str) -> u32 {
        if l.contains("⏵⏵") || l.contains("shift+tab to cycle") {
            0
        } else if l.starts_with("Keyboard: ↑/↓") {
            // The footer of every dialog of Antigravity's CLI.
            3
        } else if l.starts_with("? for shortcuts")
            || (l.contains(" · Gemini ") && l.contains("accept-edits"))
        {
            1
        } else if l.starts_with("⏺ ") || l.starts_with("❯ ") || l.starts_with("✻ ") {
            0
        } else {
            (l.starts_with("○ ")
                || l.starts_with("● ")
                || l.contains("Antigravity CLI")
                || l.contains("Conversation compacted")) as u32
        }
    }

    fn overlay(&self, lines: &[String]) -> Overlay {
        match parse_agy_dialog(lines) {
            Some((start, d)) => Overlay::Dialog(start, d),
            None => Overlay::None,
        }
    }

    fn turns(&self, lines: &[String], b: &mut Builder) {
        agy(&join_thought_wraps(lines), b)
    }

    fn is_code_colour(&self, params: &[u32]) -> bool {
        // Bright blue.
        params == [94]
    }
}

impl DialectParser for CodexDialect {
    fn dialect(&self) -> ScreenDialect {
        ScreenDialect::Codex
    }

    fn line_score(&self, l: &str) -> u32 {
        if l.starts_with("← for agents") || l.trim_start().starts_with(">_ OpenAI Codex") {
            3
        } else if l.contains("enter resume   ⌃a archive") || l.trim() == "enter select · esc back"
        {
            // the footers of Codex's own pickers
            3
        } else if super::codex::is_working_line(l) || l.contains("/ps to view · /stop to close") {
            2
        } else {
            (l.starts_with("› Ask Codex") || l.trim_start().starts_with("Worked for ")) as u32
        }
    }

    fn footer(&self, lines: &[String]) -> Option<(usize, Option<String>)> {
        super::codex::split_footer(lines)
    }

    /// Codex prints an option's note on the same line after two spaces (`1. Ask for approval (current)  Read …`).
    fn prompt(&self, lines: &[String]) -> Option<(usize, ScreenPrompt)> {
        let (start, mut p) = parse_prompt(lines)?;
        for o in &mut p.options {
            if let Some((label, note)) = o.label.split_once("  ") {
                o.description = format!("{} {}", note.trim(), o.description)
                    .trim()
                    .to_string();
                o.label = label.trim().to_string();
            }
        }
        Some((start, p))
    }

    fn turns(&self, lines: &[String], b: &mut Builder) {
        super::codex::codex(lines, b)
    }
}

impl DialectParser for GrokDialect {
    fn dialect(&self) -> ScreenDialect {
        ScreenDialect::Grok
    }

    fn line_score(&self, l: &str) -> u32 {
        let t = l.trim_start();
        if l.contains("Ctrl+x:shortcuts") || t.starts_with("Help improve Grok") {
            3
        } else if t.starts_with('╰') && l.contains(" Grok ") {
            2
        } else {
            (t.starts_with("◈ ") || t.starts_with("◆ Thought for") || t.starts_with("◆ Thinking"))
                as u32
        }
    }

    fn footer(&self, lines: &[String]) -> Option<(usize, Option<String>)> {
        super::grok::split_footer(lines)
    }

    fn prompt(&self, lines: &[String]) -> Option<(usize, ScreenPrompt)> {
        super::grok::prompt(lines)
    }

    fn turns(&self, lines: &[String], b: &mut Builder) {
        super::grok::grok(lines, b)
    }
}

impl DialectParser for UnknownDialect {
    fn dialect(&self) -> ScreenDialect {
        ScreenDialect::Unknown
    }

    fn line_score(&self, _line: &str) -> u32 {
        0
    }

    /// Nothing recognised: one folded "Terminal output" turn with the last screenfuls, never dropped.
    fn turns(&self, lines: &[String], b: &mut Builder) {
        let tail: Vec<&str> = lines.iter().map(String::as_str).collect();
        let keep = tail.len().saturating_sub(300);
        b.system("Terminal output", &tail[keep..].join("\n"));
    }
}

/// The recognised dialects, in tie-break order. `Unknown` is not listed: it is what nothing scores for.
static DIALECTS: [&dyn DialectParser; 4] =
    [&ClaudeDialect, &AgyDialect, &CodexDialect, &GrokDialect];

/// The parser of a dialect (the fallback one for `Unknown`).
pub(super) fn parser_for(dialect: ScreenDialect) -> &'static dyn DialectParser {
    DIALECTS
        .iter()
        .copied()
        .find(|p| p.dialect() == dialect)
        .unwrap_or(&UnknownDialect)
}

/// The bottom of the pane (the input box and the footer) is where an agent's own UI is drawn; text the
/// conversation prints higher up is content, which can be anything (a `cat` of a hostile file).
const FOOTER_LINES: usize = 10;
/// What the lines above the footer area may add to one dialect, however many of them speak for it.
const BODY_CAP: u32 = 2;

/// Which agent's terminal UI the lines are. The footer area counts in full; the rest counts up to
/// `BODY_CAP`, so lines of one agent's style printed into the conversation cannot outvote the real footer of
/// the other (a tie still goes to the dialect listed first).
pub(super) fn detect(lines: &[String]) -> ScreenDialect {
    let split = lines.len().saturating_sub(FOOTER_LINES);
    let (body, footer) = lines.split_at(split);
    let mut best = (0u32, ScreenDialect::Unknown);
    for p in DIALECTS {
        let above: u32 = body.iter().map(|l| p.line_score(l)).sum();
        let total = above.min(BODY_CAP) + footer.iter().map(|l| p.line_score(l)).sum::<u32>();
        if total > best.0 {
            best = (total, p.dialect());
        }
    }
    best.1
}
