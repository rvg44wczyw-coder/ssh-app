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
    /// `claude`, `agy` or `unknown` (nothing recognised: one folded "Terminal output" turn).
    pub dialect: String,
    pub turns: Vec<TranscriptTurn>,
    /// The live line of the agent ("✶ Thinking… (12s)"), if one is on screen.
    pub status: Option<String>,
    /// What is typed in the input box and not sent yet.
    pub draft: Option<String>,
    /// A question on screen that waits for an answer.
    pub prompt: Option<ScreenPrompt>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialect {
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
        Dialect::Claude => claude(&lines, &mut b),
        Dialect::Agy => agy(&lines, &mut b),
        Dialect::Unknown => {
            let tail: Vec<&str> = lines.iter().map(String::as_str).collect();
            let keep = tail.len().saturating_sub(300);
            b.system("Terminal output", &tail[keep..].join("\n"));
        }
    }
    ScreenFeed {
        dialect: match dialect {
            Dialect::Claude => "claude",
            Dialect::Agy => "agy",
            Dialect::Unknown => "unknown",
        }
        .to_string(),
        turns: b.turns,
        status,
        draft,
        prompt,
    }
}

fn detect(lines: &[String]) -> Dialect {
    let mut claude = 0;
    let mut agy = 0;
    for l in lines {
        if l.starts_with("⏺ ") || l.starts_with("❯ ") || l.starts_with("✻ ") {
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
        Dialect::Unknown
    } else if claude >= agy {
        Dialect::Claude
    } else {
        Dialect::Agy
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
                .filter(|l| !l.is_empty())
                .collect();
            let draft = (!draft.is_empty()).then(|| draft.join("\n"));
            return (s1, draft);
        }
    }
    (lines.len(), None)
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
            || rest.contains("…)"))
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

static OPTION_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

fn option_re() -> &'static regex::Regex {
    OPTION_RE.get_or_init(|| {
        regex::Regex::new(
            r"^\s*(?:│\s*)?(?P<cur>[>❯])?\s*(?P<box>\[[ xX✓✔]\]|[☐☑☒◯◉])?\s*(?P<n>\d{1,2})[.)]\s+(?P<label>\S.*?)\s*(?:│)?\s*$",
        )
        .expect("valid option pattern")
    })
}

fn is_hint(l: &str) -> bool {
    let t = l.to_lowercase();
    t.contains("navigate")
        || t.contains("enter to select")
        || t.contains("enter to confirm")
        || t.contains("esc to cancel")
}

fn unbox(l: &str) -> String {
    l.trim_matches(|c: char| "│╭╰╮╯─".contains(c) || c.is_whitespace())
        .to_string()
}

fn box_glyph(label: &str) -> Option<(&str, &str)> {
    for g in ["[ ]", "[x]", "[X]", "[✓]", "[✔]", "☐", "☑", "☒", "◯", "◉"] {
        if let Some(rest) = label.strip_prefix(g) {
            return Some((g, rest.trim_start()));
        }
    }
    None
}

/// The menu at the bottom of the screen: numbered option lines (`1. Yes`, `> 3. …` with the cursor
/// on one) right above a key-hint line ("↑/↓ Navigate · … esc to cancel"). Returns where it starts.
fn parse_prompt(lines: &[String]) -> Option<(usize, ScreenPrompt)> {
    let n = lines.len();
    let hint = (0..n)
        .rev()
        .take(8)
        .find(|&i| lines[i].to_lowercase().contains("navigate"))
        // "esc to cancel" alone is also agy's footer while it works: not a menu
        .or_else(|| {
            (0..n).rev().take(6).find(|&i| {
                let t = lines[i].to_lowercase();
                t.contains("enter to select") || t.contains("enter to confirm")
            })
        });
    let mut end = match hint {
        Some(h) => h,
        // The form's review tab ("Submit answers / Cancel") has no hint line: the tab strip
        // within the last lines marks it, and the menu runs to the last non-empty line.
        None => {
            let strip = (0..n).rev().take(24).any(|i| {
                let t = lines[i].trim();
                t.starts_with('←') && t.contains("Submit")
            });
            if !strip {
                return None;
            }
            (0..n).rev().find(|&i| !lines[i].trim().is_empty())? + 1
        }
    };
    // the hint may wrap onto the line before: back up over hint-ish lines
    while end > 0 && end <= n && hint.is_some() && is_hint(&lines[end - 1]) {
        end -= 1;
    }
    // Option starts: numbered lines counting down from the last one to 1. Wrapped labels in
    // between are not numbered, so they are skipped here and joined below.
    let lo = end.saturating_sub(40);
    let numbered: Vec<(usize, u32)> = (lo..end)
        .filter_map(|i| {
            option_re()
                .captures(&lines[i])
                .and_then(|c| c["n"].parse::<u32>().ok())
                .map(|k| (i, k))
        })
        .collect();
    let &(_, last_n) = numbered.last()?;
    let mut starts: Vec<usize> = Vec::new();
    let mut want = last_n;
    for &(i, k) in numbered.iter().rev() {
        if k == want {
            starts.push(i);
            if want == 1 {
                break;
            }
            want -= 1;
        }
    }
    if starts.len() < 2 {
        return None;
    }
    starts.reverse();
    // The menu ends at the hint: a numbered list in an ordinary reply, further up, is not one.
    let last_start = *starts.last()?;
    let tail = lines[last_start + 1..end]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .count();
    if tail > 5 {
        return None;
    }
    let first = starts[0];
    // the question: nearest non-empty line above the first option
    let question_idx = (0..first).rev().find(|&q| !lines[q].trim().is_empty())?;
    let question = unbox(&lines[question_idx]);

    let mut options: Vec<PromptOption> = Vec::new();
    let mut selected = 0u32;
    let mut multi = false;
    for (k, &st) in starts.iter().enumerate() {
        let stop = starts.get(k + 1).copied().unwrap_or(end);
        let caps = option_re().captures(&lines[st])?;
        let mut label = caps["label"].to_string();
        let mut description = String::new();
        let label_col = lines[st][..caps.name("label").map_or(0, |m| m.start())]
            .chars()
            .count();
        for cont in &lines[st + 1..stop] {
            let c = unbox(cont);
            if c.is_empty() {
                continue;
            }
            // indented under the label: an explanation; flush left: the label wrapped
            let indent = cont.chars().take_while(|ch| *ch == ' ').count();
            let target = if indent >= label_col && label_col > 0 {
                &mut description
            } else {
                &mut label
            };
            if !target.is_empty() {
                target.push(' ');
            }
            target.push_str(&c);
        }
        let mut glyph = caps.name("box").map(|m| m.as_str().to_string());
        if glyph.is_none() {
            if let Some((g, rest)) = box_glyph(&label) {
                glyph = Some(g.to_string());
                label = rest.to_string();
            }
        }
        if caps.name("cur").is_some() {
            selected = k as u32;
        }
        let checked = glyph
            .as_deref()
            .is_some_and(|b| b.contains(['x', 'X', '✓', '✔', '☑', '☒', '◉']));
        multi |= glyph.is_some();
        options.push(PromptOption {
            number: caps["n"].parse().unwrap_or(k as u32 + 1),
            label,
            description,
            checked,
        });
    }

    // Above the question: the detail (the command, the file) down to a rule, and the title above it.
    let mut start = question_idx;
    let mut title: Option<String> = None;
    let mut above: Vec<String> = Vec::new();
    let floor = question_idx.saturating_sub(14);
    let mut j = question_idx;
    while j > floor {
        j -= 1;
        let l = &lines[j];
        let t = l.trim();
        // Claude's multi-question form: a tab strip (`←  ☒ A  ☐ B  ✔ Submit  →`) above the question
        if t.starts_with('←') && t.contains("Submit") {
            title = Some(
                t.trim_matches(|c| c == '←' || c == '→' || c == ' ')
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            start = j;
            break;
        }
        if t.chars().count() >= 10 && t.chars().all(|c| c == '─' || c == '-') {
            start = j;
            if j > 0 && !lines[j - 1].trim().is_empty() {
                title = Some(unbox(&lines[j - 1]));
                start = j - 1;
            }
            break;
        }
        if !t.is_empty() {
            start = j;
        }
        above.push(unbox(l));
    }
    above.reverse();
    let detail: Vec<String> = above
        .into_iter()
        .filter(|d| !d.to_lowercase().starts_with("requesting permission"))
        .collect();
    Some((
        start,
        ScreenPrompt {
            title,
            detail: detail.join("\n").trim().to_string(),
            question,
            options,
            selected,
            multi,
        },
    ))
}

// MARK: shared helpers

/// Joins lines the terminal wrapped (the agent breaks lines itself, so `-J` cannot): a line is
/// joined to the next one when it reaches the paragraph's widest line and the next one is a plain
/// continuation, not a list item, table row, heading or code.
fn reflow(lines: &[String]) -> String {
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let mut out: Vec<String> = Vec::new();
    let mut prev_wrapped = false;
    for l in lines {
        let t = l.trim_start();
        let block_start = t.is_empty()
            || t.starts_with("• ")
            || t.starts_with("- ")
            || t.starts_with("* ")
            || t.starts_with('|')
            || t.starts_with('#')
            || t.starts_with("```")
            || t == "---"
            || t.starts_with('>')
            || t.chars().next().is_some_and(|c| c.is_ascii_digit())
                && t.trim_start_matches(|c: char| c.is_ascii_digit())
                    .starts_with(". ");
        if prev_wrapped && !block_start {
            if let Some(last) = out.last_mut() {
                last.push(' ');
                last.push_str(t);
                prev_wrapped = l.chars().count() + 12 >= width && width > 24;
                continue;
            }
        }
        out.push(l.to_string());
        prev_wrapped = !t.is_empty() && l.chars().count() + 12 >= width && width > 24;
    }
    out.join("\n")
}

fn strip_indent(l: &str, n: usize) -> String {
    let k = indent(l).min(n);
    l.chars().skip(k).collect()
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

/// A tool group the UI folded itself: "Searched for 1 pattern (ctrl+o to expand)".
fn folded_summary(l: &str) -> Option<&str> {
    let t = l.trim();
    t.strip_suffix("(ctrl+o to expand)").map(str::trim_end)
}

/// Lines after `⎿`: the first on the marker's line, the rest indented deeper.
fn take_result(lines: &[String], i: &mut usize) -> Option<String> {
    let l = lines.get(*i)?;
    let t = l.trim_start();
    let first = t.strip_prefix('⎿')?.trim().to_string();
    let base = indent(l);
    let mut out = vec![first];
    *i += 1;
    while *i < lines.len() {
        let n = &lines[*i];
        if n.is_empty() || indent(n) <= base {
            break;
        }
        out.push(n.trim().to_string());
        *i += 1;
    }
    Some(out.join("\n"))
}

/// Unrecognised text between markers, kept folded.
fn flush_other(b: &mut Builder, other: &mut Vec<String>) {
    if !other.is_empty() {
        let body = other.join("\n");
        if !body.trim().is_empty() {
            b.system("Terminal output", &body);
        }
        other.clear();
    }
}

// MARK: Claude Code

fn claude(lines: &[String], b: &mut Builder) {
    let mut other: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        let t = l.trim_start();
        if l.is_empty() {
            i += 1;
            continue;
        }
        if t.contains("▐▛")
            || t.contains("▝▜")
            || t.starts_with("▝▝")
            || t.contains("Claude Code v")
        {
            // the start-up banner: three lines
            flush_other(b, &mut other);
            let mut banner = Vec::new();
            while i < lines.len() && !lines[i].is_empty() {
                banner.push(lines[i].trim().to_string());
                i += 1;
            }
            b.system("Claude Code", &banner.join("\n"));
            continue;
        }
        if let Some(rest) = l.strip_prefix("❯ ") {
            flush_other(b, &mut other);
            let mut para = vec![rest.to_string()];
            i += 1;
            while i < lines.len() && !lines[i].is_empty() && indent(&lines[i]) >= 2 {
                para.push(strip_indent(&lines[i], 2));
                i += 1;
            }
            b.user(reflow(&para));
            continue;
        }
        if let Some(rest) = l.strip_prefix("! ") {
            flush_other(b, &mut other);
            b.user(format!("! {rest}"));
            i += 1;
            if let Some(out) = take_result(lines, &mut i) {
                b.system("Command output", &out);
            }
            continue;
        }
        if let Some(rest) = l.strip_prefix("✻ ") {
            flush_other(b, &mut other);
            // "Churned for 2m 32s · done 6:56 PM": the verb changes every time, the duration is the news
            let head = rest.split(" · ").next().unwrap_or(rest).trim();
            let label = match head.split_once(" for ") {
                Some((_, d)) => format!("Worked for {d}"),
                None => head.to_string(),
            };
            b.system(&label, "");
            i += 1;
            continue;
        }
        if let Some(rest) = l.strip_prefix("⏺ ") {
            flush_other(b, &mut other);
            if let Some((name, args_start)) = tool_head(rest) {
                let mut args = args_start.to_string();
                i += 1;
                while i < lines.len() {
                    if lines[i].is_empty() {
                        // Ctrl+O prints the call whole, blank lines of a script included: they
                        // belong to it when the text after them is still indented as arguments.
                        let next = lines.iter().skip(i + 1).find(|x| !x.is_empty());
                        if next.is_some_and(|x| indent(x) >= 4 && !x.trim_start().starts_with('⎿'))
                        {
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    if indent(&lines[i]) < 2 || lines[i].trim_start().starts_with('⎿') {
                        break;
                    }
                    args.push(' ');
                    args.push_str(lines[i].trim());
                    i += 1;
                }
                let args = args.trim_end_matches('…').trim_end();
                let args = args.strip_suffix(')').unwrap_or(args);
                let result = take_result(lines, &mut i);
                b.tool(
                    name.to_string(),
                    clip(args.trim_end_matches('…'), 4000),
                    result,
                );
                continue;
            }
            let mut para = vec![rest.to_string()];
            i += 1;
            let mut blocks: Vec<String> = Vec::new();
            while i < lines.len() {
                let n = &lines[i];
                if n.is_empty() {
                    // a blank line inside the reply, unless what follows is not indented
                    let next = lines.iter().skip(i + 1).find(|x| !x.is_empty());
                    if next.is_some_and(|x| {
                        indent(x) >= 2
                            && folded_summary(x).is_none()
                            && !x.trim_start().starts_with('⎿')
                    }) {
                        blocks.push(reflow(&para));
                        para = Vec::new();
                        i += 1;
                        continue;
                    }
                    break;
                }
                if indent(n) < 2 || folded_summary(n).is_some() || n.trim_start().starts_with('⎿')
                {
                    break;
                }
                para.push(strip_indent(n, 2));
                i += 1;
            }
            if !para.is_empty() {
                blocks.push(reflow(&para));
            }
            b.text(blocks.join("\n\n"));
            continue;
        }
        if let Some(sum) = folded_summary(l) {
            flush_other(b, &mut other);
            b.tool(sum.to_string(), String::new(), None);
            i += 1;
            continue;
        }
        if t.starts_with('⎿') {
            flush_other(b, &mut other);
            if let Some(out) = take_result(lines, &mut i) {
                b.system("Note", &out);
            }
            continue;
        }
        other.push(l.clone());
        i += 1;
    }
    flush_other(b, &mut other);
}

/// `Bash(ls -la …` -> ("Bash", "ls -la …"). Prose with a bracket in it ("Это (тест)") is not a call:
/// the name must be a bare identifier.
fn tool_head(rest: &str) -> Option<(&str, &str)> {
    let open = rest.find('(')?;
    let name = &rest[..open];
    let ok = !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == ':' || c == '-');
    ok.then(|| (name, &rest[open + 1..]))
}

// MARK: Antigravity CLI

fn agy(lines: &[String], b: &mut Builder) {
    let mut other: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        let t = l.trim_start();
        if l.is_empty() {
            i += 1;
            continue;
        }
        if !t.starts_with('⎿')
            && (l.contains("Antigravity CLI 1")
                || (indent(l) >= 2 && (t.starts_with('▄') || t.starts_with('▀'))))
        {
            flush_other(b, &mut other);
            let mut banner = Vec::new();
            while i < lines.len() && !lines[i].is_empty() {
                banner.push(lines[i].trim().to_string());
                i += 1;
            }
            b.system("Antigravity CLI", &banner.join("\n"));
            continue;
        }
        if is_rule(l) {
            i += 1;
            continue;
        }
        if l.starts_with('─') && l.contains("Conversation compacted") {
            flush_other(b, &mut other);
            b.system("Conversation compacted", "");
            i += 1;
            continue;
        }
        if let Some(rest) = l.strip_prefix("> ") {
            flush_other(b, &mut other);
            let mut para = vec![rest.to_string()];
            i += 1;
            while i < lines.len() && !lines[i].is_empty() && indent(&lines[i]) >= 2 {
                para.push(strip_indent(&lines[i], 2));
                i += 1;
            }
            b.user(reflow(&para));
            continue;
        }
        if let Some(rest) = l.strip_prefix("○ ") {
            flush_other(b, &mut other);
            b.thinking(rest.trim().to_string());
            i += 1;
            continue;
        }
        if let Some(rest) = l.strip_prefix("● ").or_else(|| l.strip_prefix("… ")) {
            flush_other(b, &mut other);
            let mut full = rest.trim().to_string();
            i += 1;
            // a summary the terminal wrapped: the next line starts at column 0 and is no marker
            while i < lines.len()
                && !lines[i].is_empty()
                && !lines[i].starts_with(|c: char| " ●○…>⚠─┃⎿".contains(c))
            {
                full.push(' ');
                full.push_str(lines[i].trim());
                i += 1;
            }
            let full = full
                .strip_suffix("(ctrl+o to expand)")
                .map(str::trim_end)
                .unwrap_or(&full)
                .to_string();
            let (name, args) = match full.find(" (") {
                Some(p) => (full[..p].to_string(), full[p + 1..].to_string()),
                None => (full, String::new()),
            };
            b.tool(name, args, None);
            continue;
        }
        if let Some(rest) = l.strip_prefix("⚠ ") {
            flush_other(b, &mut other);
            let label = rest.trim().to_string();
            i += 1;
            let body = take_result(lines, &mut i).unwrap_or_default();
            b.system(&label, &body);
            continue;
        }
        if t.starts_with('┃') {
            flush_other(b, &mut other);
            let mut notice = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('┃') {
                notice.push(
                    lines[i]
                        .trim_start()
                        .trim_start_matches('┃')
                        .trim()
                        .to_string(),
                );
                i += 1;
            }
            b.system("Notice", &notice.join("\n"));
            continue;
        }
        if t.starts_with('⎿') {
            flush_other(b, &mut other);
            if let Some(out) = take_result(lines, &mut i) {
                b.system("Note", &out);
            }
            continue;
        }
        if indent(l) >= 2 {
            flush_other(b, &mut other);
            let mut para: Vec<String> = Vec::new();
            let mut blocks: Vec<String> = Vec::new();
            while i < lines.len() {
                let n = &lines[i];
                if n.is_empty() {
                    let next = lines.iter().skip(i + 1).find(|x| !x.is_empty());
                    if next.is_some_and(|x| indent(x) >= 2 && !x.trim_start().starts_with('⎿')) {
                        blocks.push(reflow(&para));
                        para = Vec::new();
                        i += 1;
                        continue;
                    }
                    break;
                }
                if indent(n) < 2 || n.trim_start().starts_with('⎿') {
                    break;
                }
                let text = strip_indent(n, 2);
                para.push(
                    if is_rule(text.trim()) || text.trim().chars().all(|c| c == '─') {
                        "---".to_string()
                    } else {
                        text
                    },
                );
                i += 1;
            }
            if !para.is_empty() {
                blocks.push(reflow(&para));
            }
            b.text(blocks.join("\n\n"));
            continue;
        }
        other.push(l.clone());
        i += 1;
    }
    flush_other(b, &mut other);
}

// MARK: thinking from the session file

fn norm(s: &str, n: usize) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .chars()
        .take(n)
        .collect()
}

/// Tools the agent's screen folds into "Searched for N patterns" / "Read N files" groups.
const FOLDABLE_TOOLS: [&str; 15] = [
    // Antigravity
    "run_command",
    "view_file",
    "grep_search",
    "find_by_name",
    "list_dir",
    "search_web",
    "read_url_content",
    // Claude Code
    "Grep",
    "Glob",
    "Read",
    "LS",
    "WebSearch",
    "WebFetch",
    "ToolSearch",
    "NotebookRead",
];

/// Fills a folded group with the calls the session file knows: what was searched / read, and a
/// few lines of each result.
fn describe_group(group: &mut TranscriptToolCall, calls: &[TranscriptToolCall]) {
    let mut lines: Vec<String> = Vec::new();
    for c in calls {
        lines.push(format!("{}({})", c.tool_name, c.arguments_summary));
        if let Some(r) = c.result.as_deref() {
            let shown: Vec<&str> = r.lines().filter(|l| !l.trim().is_empty()).take(3).collect();
            for l in &shown {
                lines.push(format!("  {}", l.chars().take(200).collect::<String>()));
            }
            let total = r.lines().filter(|l| !l.trim().is_empty()).count();
            if total > shown.len() {
                lines.push(format!("  … +{} lines", total - shown.len()));
            }
        }
    }
    group.arguments_summary = calls
        .iter()
        .map(|c| c.arguments_summary.chars().take(40).collect::<String>())
        .collect::<Vec<_>>()
        .join(", ");
    group.result = Some(lines.join("\n"));
}

/// Puts the agent's thinking (which the screen does not show) into the screen's feed, each block
/// right before the reply text or tool call it preceded, so the order of the story is kept.
/// `file` is the session file's turns; a turn there is matched to the screen turn with the same
/// reply text or the same tool call (name and the start of its arguments), moving forward only.
fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

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

/// Thinking whose turn is not found goes before the next one that is; thinking after the last
/// match is dropped rather than put in a wrong place.
#[uniffi::export]
pub fn merge_thinking(
    screen: Vec<TranscriptTurn>,
    file: Vec<TranscriptTurn>,
) -> Vec<TranscriptTurn> {
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
                    return &sc.tool_name == name
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

#[cfg(test)]
mod tests {
    use super::*;

    fn turns(text: &str) -> ScreenFeed {
        parse_screen_feed(text.to_string())
    }

    const CLAUDE: &str = "\
 ▐▛███▛█   Claude Code v2.1.303
▝▜██████▀  Sonnet 5.5 · Claude Pro
 ▝▝   ▝▝   ~/rust/ssh-app

❯ в ленте чата только hi, хотя мы с
  тобой уже наговорили

  Searched for 1 pattern (ctrl+o to expand)

⏺ Bash(ls -la ~/.claude/projects 2>&1 | head; env |
      grep -i CLAUDE_CONFIG)
  ⎿  total 0
     drwxr-xr-x@ 19 ivs  staff  608
     … +13 lines (ctrl+o to expand)

⏺ Нашёл причину: лента читала не тот файл. Проверяю
  правку и собираю.

  Причина. Ваш Claude запущен с другим каталогом.
  - первый пункт
  - второй пункт

✻ Churned for 1m 7s · done 6:11 PM

! tmux display -p
  ⎿  Running…

✶ Thinking… (5s · esc to interrupt)
──────────────────────────────────────────────────
❯ черновик
──────────────────────────────────────────────────
  Sonnet 5.5 · 5h 9% · 7d 10%
";

    #[test]
    fn claude_screen() {
        let f = turns(CLAUDE);
        assert_eq!(f.dialect, "claude");
        assert_eq!(f.draft.as_deref(), Some("черновик"));
        assert_eq!(
            f.status.as_deref(),
            Some("✶ Thinking… (5s · esc to interrupt)")
        );
        let kinds: Vec<String> = f
            .turns
            .iter()
            .map(|t| match (&t.turn_type, &t.label) {
                (TranscriptTurnType::User, _) => "user".to_string(),
                (TranscriptTurnType::System, Some(l)) => format!("sys:{l}"),
                (_, _) if !t.tool_calls.is_empty() => format!("tool:{}", t.tool_calls[0].tool_name),
                _ => "text".to_string(),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "sys:Claude Code",
                "user",
                "tool:Searched for 1 pattern",
                "tool:Bash",
                "text",
                "sys:Worked for 1m 7s",
                "user",
                "sys:Command output",
            ]
        );
        assert_eq!(
            f.turns[1].content.as_deref(),
            Some("в ленте чата только hi, хотя мы с тобой уже наговорили")
        );
        let bash = &f.turns[3].tool_calls[0];
        assert_eq!(
            bash.arguments_summary,
            "ls -la ~/.claude/projects 2>&1 | head; env | grep -i CLAUDE_CONFIG"
        );
        assert!(bash.result.as_deref().unwrap().contains("… +13 lines"));
        let text = f.turns[4].content.as_deref().unwrap();
        assert!(text.starts_with("Нашёл причину"));
        assert!(text.contains("\n\nПричина."));
        assert!(text.contains("- первый пункт\n- второй пункт"));
    }

    const AGY: &str = "\
      ▄▀▀▄        Antigravity CLI 1.4.2
     ▀▀▀▀▀▀       Gemini 3.8 Flash (Medium)

  Подготовил подробное техническое исследование по
  разработке собственного терминала:

  ### Ключевые выводы

  1. Реализуемость:
      • Задача выполнима.

○ Thought for 6m 29s (Integration tests for `ivs-term...)
● Explored 1 file (lib.rs)
… 95 more actions (23 reads, 25 edits, 47 commands: 7
fai...
● Ran 5 commands (+1 · cargo clipp...) (ctrl+o to expand)

  Проверяю сборку и тесты.

──────────────── Conversation compacted ─────────────────

> давай периодические статусы

⚠ Conversation already open
  ⎿  When you opened this conversation it was already
     open in another CLI instance.

 ┃ Verbosity now defaults to medium
 ┃ esc to dismiss
  ⎿  Interrupted · What should Antigravity CLI do instead?
──────────────────────────────────────────────────────────
>
──────────────────────────────────────────────────────────
? for shortcuts   accept-edits · Gemini 3.8 Flash · medium
";

    #[test]
    fn agy_screen() {
        let f = turns(AGY);
        assert_eq!(f.dialect, "agy");
        assert_eq!(f.draft, None);
        let kinds: Vec<String> = f
            .turns
            .iter()
            .map(|t| match (&t.turn_type, &t.label) {
                (TranscriptTurnType::User, _) => "user".to_string(),
                (TranscriptTurnType::System, Some(l)) => format!("sys:{l}"),
                (_, _) if t.thinking.is_some() => "thinking".to_string(),
                (_, _) if !t.tool_calls.is_empty() => format!("tool:{}", t.tool_calls[0].tool_name),
                _ => "text".to_string(),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "sys:Antigravity CLI",
                "text",
                "thinking",
                "tool:Explored 1 file",
                "tool:95 more actions",
                "tool:Ran 5 commands",
                "text",
                "sys:Conversation compacted",
                "user",
                "sys:Conversation already open",
                "sys:Notice",
                "sys:Note",
            ]
        );
        assert!(f.turns[1]
            .content
            .as_deref()
            .unwrap()
            .starts_with("Подготовил подробное"));
        assert!(f.turns[4].tool_calls[0]
            .arguments_summary
            .contains("fai..."));
    }

    #[test]
    fn agy_permission_prompt_becomes_a_card() {
        let text = "\
> Как дела? Чем занят?
○ Thought for 6.1s (`uniffi-bindgen` requires a UDL f...)
● Ran 4 commands (2 failed) (ctrl+o to expand)
  Слежу за процессом.
Command
──────────────────────────────────────────────────────────
Requesting permission for:
   file target/debug/libivs_term_bridge.dylib
Run this command?
  1. Yes, run command
  2. Yes, and always allow in this conversation for
commands that start with 'file'
> 3. Yes, and always allow for commands that start with
'file' (Persist to settings.json)
  4. No, cancel
  ↑/↓ Navigate · tab Amend · ctrl+g edit/expand command ·
esc to cancel     accept-edits · Gemini 3.8 Flash · medium
";
        let f = parse_screen_feed(text.to_string());
        let p = f.prompt.expect("a prompt");
        assert_eq!(p.title.as_deref(), Some("Command"));
        assert_eq!(p.detail, "file target/debug/libivs_term_bridge.dylib");
        assert_eq!(p.question, "Run this command?");
        assert_eq!(p.options.len(), 4);
        assert_eq!(p.options[0].label, "Yes, run command");
        assert_eq!(
            p.options[1].label,
            "Yes, and always allow in this conversation for commands that start with 'file'"
        );
        assert_eq!(p.selected, 2);
        assert!(!p.multi);
        // nothing of the menu leaks into the feed
        assert!(f
            .turns
            .iter()
            .all(|t| !t.content.as_deref().unwrap_or("").contains("Yes, run")));
        assert!(f
            .turns
            .iter()
            .any(|t| t.content.as_deref() == Some("Как дела? Чем занят?")));
    }

    #[test]
    fn claude_checkbox_menu_is_multi() {
        let text = "\
⏺ Что включить?

 Что включить?
 ❯ 1. [ ] Хуки
   2. [x] Пуши
   3. [ ] Лента
 Enter to select · ↑/↓ to navigate · Esc to cancel
";
        let f = parse_screen_feed(text.to_string());
        let p = f.prompt.expect("a prompt");
        assert!(p.multi);
        assert_eq!(p.options.len(), 3);
        assert!(p.options[1].checked && !p.options[0].checked);
        assert_eq!(p.selected, 0);
        assert_eq!(p.question, "Что включить?");
    }

    #[test]
    fn claude_question_form_with_tabs_and_descriptions() {
        let text = "\
⏺ Bash(make iphone-full)
  ⎿  App installed

⏺ Теперь задам мультивопрос.
───────────────────────────────────────────────────────────────
←  ☒ Дальше  ☐ Перезапуск  ✔ Submit  →

Как перезапускать агента после включения хуков?

❯ 1. Сам, с подтверждением (Рекомендуется)
     Приложение шлёт /exit и claude --resume
  2. Только подсказка
     Перезапускаете вы
  3. Type something.
───────────────────────────────────────────────────────────────
  4. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel
";
        let f = parse_screen_feed(text.to_string());
        let p = f.prompt.expect("a prompt");
        assert_eq!(
            p.question,
            "Как перезапускать агента после включения хуков?"
        );
        assert_eq!(p.title.as_deref(), Some("☒ Дальше ☐ Перезапуск ✔ Submit"));
        assert_eq!(p.options.len(), 4);
        assert!(p.options[0].label.starts_with("Сам, с подтверждением"));
        assert_eq!(p.options[0].label, "Сам, с подтверждением (Рекомендуется)");
        assert_eq!(
            p.options[0].description,
            "Приложение шлёт /exit и claude --resume"
        );
        assert_eq!(p.options[3].label, "Chat about this");
        assert_eq!(p.selected, 0);
        assert!(f.turns.iter().all(|t| !t
            .content
            .as_deref()
            .unwrap_or("")
            .contains("Перезапускаете")));
    }

    #[test]
    fn claude_form_review_tab_without_hint() {
        let text = "\
❯ Задай мне мультивопрос

──────────────────────────────────────────────────────────
←  ☒ Дальше  ☒ Лента  ✔ Submit  →

Review your answers

 ● Что делать следующим?
   → Хуки в работающей сессии

Ready to submit your answers?

❯ 1. Submit answers
  2. Cancel
";
        let f = parse_screen_feed(text.to_string());
        let p = f.prompt.expect("a prompt");
        assert_eq!(p.question, "Ready to submit your answers?");
        assert_eq!(p.options.len(), 2);
        assert!(p.detail.contains("Хуки в работающей сессии"));
    }

    #[test]
    fn the_screen_is_cleaned_of_escapes_and_control_characters() {
        let dirty = "\u{1b}]0;title\u{7}❯ hi\u{1b}[31m red\u{8}\u{202e}evil\u{7}\n";
        let feed = parse_screen_feed(dirty.to_string());
        let text = format!("{feed:?}");
        for bad in ['\u{1b}', '\u{7}', '\u{8}', '\u{202e}'] {
            assert!(!text.contains(bad), "{bad:?} survived: {text}");
        }
    }

    fn user(text: &str) -> TranscriptTurn {
        TranscriptTurn {
            id: text.into(),
            step_index: 0,
            turn_type: TranscriptTurnType::User,
            timestamp: None,
            content: Some(text.into()),
            thinking: None,
            tool_calls: vec![],
            status: TranscriptStatus::Done,
            label: None,
        }
    }

    #[test]
    fn a_file_matches_the_screen_when_something_you_said_is_on_it() {
        let screen = "❯ fix the login\n\n⏺ Done.\n❯ and   now the\n  tests please\n".to_string();
        assert!(transcript_matches_screen(
            vec![user("fix the login"), user("and now the tests please")],
            screen.clone()
        ));
        // another conversation in the same folder
        assert!(!transcript_matches_screen(
            vec![user("rewrite the parser")],
            screen.clone()
        ));
        // nothing said yet: cannot tell, accepted
        assert!(transcript_matches_screen(vec![], screen));
    }

    #[test]
    fn numbered_reply_above_a_working_footer_is_not_a_menu() {
        let text = "\
> Опиши варианты
● Вот варианты:
  1. Первый вариант
  2. Второй вариант
  Первый лучше, потому что проще.
  Второй дороже в поддержке.
  Итого рекомендую первый.
  Это описание, а не выбор.
  Спрашивайте дальше.
────────────────────────────────────────
>
────────────────────────────────────────
  esc to cancel     accept-edits · Gemini 3.8 Flash · medium
";
        let f = parse_screen_feed(text.to_string());
        assert!(f.prompt.is_none());
    }

    #[test]
    fn folded_group_gets_its_calls_from_the_file() {
        let mk = |content: Option<&str>, tool: Option<(&str, &str, Option<&str>)>| TranscriptTurn {
            id: "x".into(),
            step_index: 0,
            turn_type: TranscriptTurnType::Assistant,
            timestamp: None,
            content: content.map(String::from),
            thinking: None,
            tool_calls: tool
                .map(|(n, a, r)| TranscriptToolCall {
                    call_id: "c".into(),
                    tool_name: n.into(),
                    arguments_summary: a.into(),
                    full_arguments_json: None,
                    result: r.map(String::from),
                    is_error: false,
                    status: TranscriptStatus::Done,
                })
                .into_iter()
                .collect(),
            status: TranscriptStatus::Done,
            label: None,
        };
        let screen = vec![
            mk(None, Some(("Searched for 1 pattern", "", None))),
            mk(Some("Нашёл причину: лента читала не тот файл."), None),
        ];
        let file = vec![
            mk(
                None,
                Some((
                    "Grep",
                    "pattern: projects",
                    Some("a.rs:1\nb.rs:2\nc.rs:3\nd.rs:4"),
                )),
            ),
            mk(Some("Нашёл причину: лента читала не тот файл."), None),
        ];
        let out = merge_thinking(screen, file);
        let c = &out[0].tool_calls[0];
        assert_eq!(c.tool_name, "Searched for 1 pattern");
        assert_eq!(c.arguments_summary, "pattern: projects");
        let r = c.result.as_deref().unwrap();
        assert!(r.starts_with("Grep(pattern: projects)\n  a.rs:1"));
        assert!(r.ends_with("… +1 lines"));
    }

    #[test]
    fn matched_edit_gets_arguments_and_result_from_the_file() {
        let call = |args: Option<&str>, result: &str| TranscriptToolCall {
            call_id: "c".into(),
            tool_name: "Edit".into(),
            arguments_summary: "a.rs".into(),
            full_arguments_json: args.map(String::from),
            result: Some(result.into()),
            is_error: false,
            status: TranscriptStatus::Done,
        };
        let turn = |c| TranscriptTurn {
            id: "x".into(),
            step_index: 0,
            turn_type: TranscriptTurnType::Assistant,
            timestamp: None,
            content: None,
            thinking: None,
            tool_calls: vec![c],
            status: TranscriptStatus::Done,
            label: None,
        };
        let screen = vec![turn(call(
            None,
            "Added 2 lines\n… +2 lines (ctrl+o to expand)",
        ))];
        let file = vec![turn(call(Some("{\"old_string\":\"a\"}"), "full result"))];
        let out = merge_thinking(screen, file);
        let c = &out[0].tool_calls[0];
        assert_eq!(
            c.full_arguments_json.as_deref(),
            Some("{\"old_string\":\"a\"}")
        );
        assert_eq!(c.result.as_deref(), Some("full result"));
    }

    #[test]
    fn ctrl_o_transcript_view_keeps_full_calls_and_drops_its_footer() {
        let text = include_str!("../tests/data/claude_ctrl_o.txt");
        let f = parse_screen_feed(text.to_string());
        assert_eq!(f.dialect, "claude");
        let bash = f
            .turns
            .iter()
            .flat_map(|t| t.tool_calls.iter())
            .find(|c| c.tool_name == "Bash")
            .expect("a Bash call");
        assert!(bash.arguments_summary.contains("python3"));
        assert!(bash
            .result
            .as_deref()
            .unwrap_or("")
            .contains("test result: ok"));
        assert!(f.turns.iter().all(|t| !t
            .content
            .as_deref()
            .unwrap_or("")
            .contains("Showing detailed transcript")));
        assert!(f.prompt.is_none());
    }

    #[test]
    fn no_prompt_without_a_hint_line() {
        let f = parse_screen_feed("⏺ Вот план:\n\n  1. Первое\n  2. Второе\n".to_string());
        assert!(f.prompt.is_none());
        assert_eq!(f.turns.len(), 1);
    }

    #[test]
    fn unknown_screen_is_kept_folded() {
        let f = turns("user@host ~ % ls\nfoo bar\n");
        assert_eq!(f.dialect, "unknown");
        assert_eq!(f.turns.len(), 1);
        assert!(f.turns[0].content.as_deref().unwrap().contains("foo bar"));
    }

    #[test]
    fn thinking_is_put_before_its_reply_and_tool() {
        let screen = parse_screen_feed(CLAUDE.to_string()).turns;
        let mk = |thinking: Option<&str>, content: Option<&str>, tool: Option<(&str, &str)>| {
            TranscriptTurn {
                id: "f".into(),
                step_index: 0,
                turn_type: TranscriptTurnType::Assistant,
                timestamp: None,
                content: content.map(String::from),
                thinking: thinking.map(String::from),
                tool_calls: tool
                    .map(|(n, a)| {
                        vec![TranscriptToolCall {
                            call_id: "c".into(),
                            tool_name: n.into(),
                            arguments_summary: a.into(),
                            full_arguments_json: None,
                            result: None,
                            is_error: false,
                            status: TranscriptStatus::Done,
                        }]
                    })
                    .unwrap_or_default(),
                status: TranscriptStatus::Done,
                label: None,
            }
        };
        let file = vec![
            mk(
                Some("first thought"),
                None,
                Some(("Bash", "ls -la ~/.claude/projects 2>&1 | head")),
            ),
            mk(
                Some("second thought"),
                Some("Нашёл причину: лента читала не тот файл. Проверяю правку и собираю."),
                None,
            ),
        ];
        let merged = merge_thinking(screen.clone(), file);
        assert_eq!(merged.len(), screen.len() + 2);
        let pos = |pred: &dyn Fn(&TranscriptTurn) -> bool| merged.iter().position(pred).unwrap();
        let t1 = pos(&|t| t.thinking.as_deref() == Some("first thought"));
        let bash = pos(&|t| t.tool_calls.first().is_some_and(|c| c.tool_name == "Bash"));
        let t2 = pos(&|t| t.thinking.as_deref() == Some("second thought"));
        let reply = pos(&|t| {
            t.content
                .as_deref()
                .is_some_and(|c| c.starts_with("Нашёл причину"))
        });
        assert_eq!(t1 + 1, bash);
        assert_eq!(t2 + 1, reply);
        assert!(bash < t2);
    }
}
