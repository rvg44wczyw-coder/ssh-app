//! Background tasks of the agent (feature 59): the count and commands read from the footer, and the
//! task screens of Claude Code (`Shell details`) and Antigravity (`Task Details`). Pure functions over the
//! cleaned lines of a capture. Everything here is untrusted display text: lengths are capped, control
//! characters are gone before the lines arrive, and nothing parsed is ever executed or sent back as input.

use super::dialog::{DialogKind, DialogRow, RowState, ScreenDialog};
use super::{is_rule, ScreenDialect};

const MAX_COMMANDS: usize = 20;
const MAX_COMMAND: usize = 200;
const MAX_OUTPUT_ROWS: usize = 200;
const MAX_COUNT: u32 = 999;
/// How many of the last non-empty lines are looked at for a count: the footer, not old output.
const FOOTER_LINES: usize = 12;

/// How many background tasks the agent says it runs, and (where the screen names them) their commands.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BackgroundSummary {
    pub count: u32,
    pub commands: Vec<String>,
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn tail(lines: &[String], n: usize) -> Vec<&str> {
    let mut v: Vec<&str> = lines
        .iter()
        .rev()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .take(n)
        .collect();
    v.reverse();
    v
}

/// `3 shells`, `1 shell still running`, `2 task(s)`, `1 command still running`: the number in front
/// of `unit`, when the whole segment is exactly that.
fn counted(segment: &str, units: &[&str]) -> Option<u32> {
    let seg = segment.trim().trim_start_matches(['◎', '◉']).trim();
    let (num, rest) = seg.split_once(' ')?;
    let n: u32 = num.parse().ok()?;
    let rest = rest.trim();
    let rest = rest
        .strip_suffix("still running")
        .or_else(|| rest.strip_suffix("running"))
        .map_or(rest, str::trim_end);
    units.contains(&rest).then_some(n.min(MAX_COUNT))
}

fn claude_count(lines: &[&str]) -> Option<u32> {
    // The mode line of the footer (`⏵⏵ auto mode on · 1 shell`) is the live count; a mode line that
    // names no shell means none. Only when there is no mode line does a turn summary count.
    if let Some(mode) = lines.iter().rev().find(|l| l.contains("⏵⏵")) {
        return mode
            .split(" · ")
            .find_map(|s| counted(s, &["shell", "shells"]));
    }
    lines.iter().rev().find_map(|l| {
        l.starts_with('✻')
            .then(|| {
                l.split(" · ")
                    .find_map(|s| counted(s, &["shell", "shells"]))
            })
            .flatten()
    })
}

fn agy_count(lines: &[&str]) -> Option<u32> {
    let footer = lines.last()?;
    footer
        .split(" · ")
        .find_map(|s| counted(s, &["task(s)", "task", "tasks"]))
}

/// `● [11:40:03] sleep 120 running` -> `sleep 120`.
pub(super) fn agy_task_line(l: &str) -> Option<String> {
    let rest = l.strip_prefix('●')?.trim_start().strip_prefix('[')?;
    let (time, cmd) = rest.split_once("] ")?;
    if time.len() != 8 || !time.bytes().all(|b| b.is_ascii_digit() || b == b':') {
        return None;
    }
    let cmd = cmd.trim_end();
    let cmd = cmd.strip_suffix("running").map_or(cmd, str::trim_end);
    (!cmd.is_empty()).then(|| clip(cmd, MAX_COMMAND))
}

fn grok_count(lines: &[&str]) -> Option<u32> {
    lines
        .iter()
        .rev()
        .find(|l| l.starts_with(['◎', '◉']) && l.contains("still running"))
        .and_then(|l| counted(l, &["command", "commands"]))
}

/// `• Working (9s • esc to interrupt) · 1 background terminal running · /ps to view · /stop to close`:
/// the count, only while the line is Codex's working line. `/stop` ends all of them and `/ps` prints into
/// the feed, so Codex has a counter and the commands of the last `/ps` list, but no screen to drive.
fn codex_count(lines: &[&str]) -> Option<u32> {
    lines
        .iter()
        .rev()
        .find(|l| super::codex::is_working_line(l))
        .and_then(|l| {
            l.split(" · ")
                .find_map(|s| counted(s, &["background terminal", "background terminals"]))
        })
}

/// The commands under the last `Background terminals` heading (`• sleep 30`), up to the first blank line
/// after them.
fn codex_ps_commands(lines: &[String]) -> Vec<String> {
    let Some(at) = lines
        .iter()
        .rposition(|l| l.trim() == "Background terminals")
    else {
        return Vec::new();
    };
    lines[at + 1..]
        .iter()
        .map(|l| l.trim())
        .skip_while(|l| l.is_empty())
        .take_while(|l| l.starts_with("• "))
        .filter_map(|l| l.strip_prefix("• "))
        .map(|c| clip(c.trim(), MAX_COMMAND))
        .filter(|c| !c.is_empty())
        .take(MAX_COMMANDS)
        .collect()
}

/// The summary for the dialect of the screen; `None` when the agent shows no running task.
pub(super) fn summary(dialect: ScreenDialect, lines: &[String]) -> Option<BackgroundSummary> {
    let last = tail(lines, FOOTER_LINES);
    let (count, commands) = match dialect {
        ScreenDialect::Claude => (claude_count(&last)?, Vec::new()),
        ScreenDialect::Agy => (
            agy_count(&last)?,
            last.iter()
                .filter_map(|l| agy_task_line(l))
                .take(MAX_COMMANDS)
                .collect(),
        ),
        ScreenDialect::Grok => (grok_count(&last)?, Vec::new()),
        ScreenDialect::Codex => (codex_count(&last)?, codex_ps_commands(lines)),
        ScreenDialect::Unknown => return None,
    };
    (count > 0).then_some(BackgroundSummary { count, commands })
}

fn output_rows(lines: &[&str], none_text: &str) -> Vec<DialogRow> {
    lines
        .iter()
        .map(|l| l.trim_end())
        .filter(|l| !l.trim().is_empty() && l.trim() != none_text)
        .take(MAX_OUTPUT_ROWS)
        .map(|l| DialogRow {
            number: None,
            label: clip(l.trim_start(), MAX_COMMAND),
            value: None,
            detail: String::new(),
            state: RowState::Plain,
        })
        .collect()
}

fn dialog(title: &str, note: Vec<String>, rows: Vec<DialogRow>, hints: &str) -> ScreenDialog {
    ScreenDialog {
        kind: DialogKind::Tasks,
        title: title.to_string(),
        tabs: Vec::new(),
        search: None,
        note: note.join("\n"),
        rows,
        selected: 0,
        hints: clip(hints, MAX_COMMAND),
        more_above: false,
        more_below: false,
    }
}

/// Claude Code's `Shell details` screen. Returns the line the pane is cut at (the rule above the
/// title) and the dialog: `note` holds Status / Runtime / Command, `rows` the output lines.
pub(super) fn claude_shell_details(lines: &[String]) -> Option<(usize, ScreenDialog)> {
    let last = lines.iter().rposition(|l| !l.trim().is_empty())?;
    let hints = lines[last].trim();
    if !(hints.contains("to close") && hints.contains("x to stop")) {
        return None;
    }
    let from = last.saturating_sub(60);
    let title = (from..last)
        .rev()
        .find(|&i| lines[i].trim() == "Shell details")?;
    let cut = if title > 0 && is_rule(&lines[title - 1]) {
        title - 1
    } else {
        title
    };
    let field = |name: &str| {
        lines[title + 1..last]
            .iter()
            .map(|l| l.trim())
            .find_map(|l| l.strip_prefix(name).map(|v| clip(v.trim(), MAX_COMMAND)))
    };
    let mut note = Vec::new();
    for (name, label) in [
        ("Status:", "Status"),
        ("Runtime:", "Runtime"),
        ("Command:", "Command"),
    ] {
        if let Some(v) = field(name) {
            note.push(format!("{label}: {v}"));
        }
    }
    let out_at = (title + 1..last).find(|&i| lines[i].trim() == "Output:");
    let body: Vec<&str> = out_at
        .map(|i| lines[i + 1..last].iter().map(String::as_str).collect())
        .unwrap_or_default();
    Some((
        cut,
        dialog(
            "Shell details",
            note,
            output_rows(&body, "No output available"),
            hints,
        ),
    ))
}

/// Antigravity's `Task Details` screen, drawn under the input box. Returns the line the pane is cut at
/// (as `parse_agy_dialog` does) and the dialog.
pub(super) fn agy_task_detail(lines: &[String]) -> Option<(usize, ScreenDialog)> {
    let close = (2..lines.len()).rev().find(|&i| {
        is_rule(&lines[i]) && lines[i - 1].trim_start().starts_with('>') && is_rule(&lines[i - 2])
    })?;
    let end = lines.iter().rposition(|l| !l.trim().is_empty())? + 1;
    // The model line at the very bottom is not part of the dialog.
    let mut end = end;
    if end > close + 1 && lines[end - 1].contains(" · ") && super::indent(&lines[end - 1]) > 20 {
        end -= 1;
    }
    let region = &lines[close + 1..end];
    let title_at = region.iter().position(|l| l.trim() == "Task Details")?;
    let hints_at = region.iter().rposition(|l| l.contains("esc back"))?;
    if hints_at <= title_at {
        return None;
    }
    let field = |name: &str| {
        region[title_at + 1..hints_at]
            .iter()
            .map(|l| l.trim())
            .find_map(|l| {
                l.strip_prefix(name)
                    .map(|v| clip(v.trim().trim_start_matches('●').trim(), MAX_COMMAND))
            })
    };
    let mut note = Vec::new();
    for (name, label) in [
        ("Command:", "Command"),
        ("Status:", "Status"),
        ("Type:", "Type"),
        ("Started:", "Started"),
    ] {
        if let Some(v) = field(name) {
            note.push(format!("{label}: {v}"));
        }
    }
    let out_at = (title_at + 1..hints_at).find(|&i| region[i].trim() == "Output");
    let body: Vec<&str> = out_at
        .map(|i| region[i + 1..hints_at].iter().map(String::as_str).collect())
        .unwrap_or_default();
    Some((
        close.saturating_sub(1),
        dialog(
            "Task Details",
            note,
            output_rows(&body, "(no output)"),
            region[hints_at].trim(),
        ),
    ))
}

/// Whether the Stop key may be sent now: only while a task's detail is on screen (the list has no
/// note, and any other dialog or question is not ours). Pure, so the rule can be tested.
#[uniffi::export]
pub fn background_stop_allowed(feed: super::ScreenFeed) -> bool {
    feed.prompt.is_none()
        && feed
            .dialog
            .is_some_and(|d| d.kind == DialogKind::Tasks && d.note.contains("Command:"))
}

/// Whether the open keys may be sent now: the screen was recognised as `expected`, it shows running
/// tasks, no dialog or question is up and the input box is empty (the first Claude key is an arrow
/// that would move the cursor in a draft).
#[uniffi::export]
pub fn background_open_allowed(feed: super::ScreenFeed, expected: ScreenDialect) -> bool {
    feed.dialect == expected
        && feed.background.is_some()
        && feed.dialog.is_none()
        && feed.aside.is_none()
        && feed.prompt.is_none()
        && feed.draft.as_deref().is_none_or(str::is_empty)
}
