//! Claude Code's answers that are not in its transcript but drawn over the pane: the `/btw`
//! overlay and the full-screen dialogs of `/model`, `/permissions`, `/config`, `/mcp` and
//! `/status` (feature 52). Pure functions over the cleaned lines of a capture; everything read
//! here is untrusted display text, so lengths are capped and nothing is ever executed.

const MAX_ROWS: usize = 200;
const MAX_TEXT: usize = 200;
const MAX_ANSWER: usize = 20_000;
/// A dialog is drawn inside the last screenful; looking further up would read old output.
const MAX_REGION: usize = 60;

/// The `/btw` side question and its answer (the overlay closes with `Esc`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScreenAside {
    pub question: String,
    pub answer: String,
    /// "· Answering…" is still on screen.
    pub pending: bool,
    /// The footer offers `↑/↓ to scroll`: more of the answer than the pane shows.
    pub scrollable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DialogKind {
    Model,
    Permissions,
    Config,
    Mcp,
    Status,
    /// A background task screen (feature 59): the list, or one task's details in `note` and `rows`.
    Tasks,
    /// Any other dialog of Antigravity: a title, blocks of text and one list with a `>` cursor.
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RowState {
    Plain,
    /// `✔`: the value in effect (the current model, a connected server).
    Current,
    /// `true` of a setting.
    On,
    /// `false` of a setting, or `◯` (not configured).
    Off,
    /// `⚠`.
    Warning,
    /// A group title that is not selectable.
    Header,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DialogRow {
    pub number: Option<u32>,
    pub label: String,
    pub value: Option<String>,
    pub detail: String,
    pub state: RowState,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DialogTab {
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScreenDialog {
    pub kind: DialogKind,
    pub title: String,
    pub tabs: Vec<DialogTab>,
    /// The text of the search box, empty while it only shows its placeholder; `None`: no box.
    pub search: Option<String>,
    /// Explanation lines under the title and the effort line of `/model`.
    pub note: String,
    pub rows: Vec<DialogRow>,
    /// Index into `rows` of the row under the `❯` cursor (0 when none is marked).
    pub selected: u32,
    /// The footer line of keys ("Enter to set as default · … · Esc to cancel").
    pub hints: String,
    pub more_above: bool,
    pub more_below: bool,
}

fn clip(s: &str) -> String {
    s.chars().take(MAX_TEXT).collect()
}

fn last_nonblank(lines: &[String]) -> Option<usize> {
    lines.iter().rposition(|l| !l.trim().is_empty())
}

/// Index of the line where the `/btw` overlay starts (its echo of the question), the question,
/// and the overlay's footer.
pub(super) fn parse_aside(lines: &[String]) -> Option<(usize, ScreenAside)> {
    let last = last_nonblank(lines)?;
    let footer = lines[last].trim();
    if !footer.ends_with("Esc to close") {
        return None;
    }
    let from = last.saturating_sub(MAX_REGION);
    // The overlay repeats the question without the prompt glyph, under the input line's echo.
    let echo = (from..last).rev().find(|&i| {
        let t = lines[i].trim_start();
        t.starts_with("/btw ") && !lines[i].trim_start().starts_with('❯')
    })?;
    let question = clip(lines[echo].trim_start().trim_start_matches("/btw").trim());
    let body: Vec<&str> = lines[echo + 1..last].iter().map(|l| l.trim()).collect();
    let pending = body.iter().any(|l| l.starts_with("· Answering"));
    let answer = if pending {
        String::new()
    } else {
        let text = body.join("\n").trim().to_string();
        text.chars().take(MAX_ANSWER).collect()
    };
    Some((
        echo,
        ScreenAside {
            question,
            answer,
            pending,
            scrollable: footer.contains("↑/↓"),
        },
    ))
}

fn kind_of(region: &[String]) -> Option<DialogKind> {
    let text = region.join("\n");
    if text.contains("Select model") {
        Some(DialogKind::Model)
    } else if text.contains("Manage MCP servers") {
        Some(DialogKind::Mcp)
    } else if region
        .iter()
        .any(|l| l.contains("Permissions") && l.contains("Allow") && l.contains("Deny"))
    {
        Some(DialogKind::Permissions)
    } else if region
        .iter()
        .any(|l| l.contains("Settings") && l.contains("Status") && l.contains("Usage"))
    {
        if text.contains("Session ID:") || text.contains("cwd:") {
            Some(DialogKind::Status)
        } else {
            Some(DialogKind::Config)
        }
    } else {
        None
    }
}

fn split_cols(s: &str) -> (String, String) {
    match s.find("  ") {
        Some(i) => (clip(s[..i].trim()), clip(s[i..].trim())),
        None => (clip(s.trim()), String::new()),
    }
}

fn numbered(line: &str) -> Option<(bool, bool, bool, u32, String)> {
    // `❯ 3.  Sonnet 5.5 ✔  …`, `↓ 10. …`, `    2.  Opus 5.5`
    let t = line.trim_start();
    let (cursor, up, down, t) = if let Some(r) = t.strip_prefix('❯') {
        (true, false, false, r)
    } else if let Some(r) = t.strip_prefix('↓') {
        (false, false, true, r)
    } else if let Some(r) = t.strip_prefix('↑') {
        (false, true, false, r)
    } else {
        (false, false, false, t)
    };
    let t = t.trim_start();
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 4 {
        return None;
    }
    let rest = t[digits.len()..].strip_prefix('.')?;
    Some((
        cursor,
        up,
        down,
        digits.parse().ok()?,
        rest.trim().to_string(),
    ))
}

/// The dialog on the last screenful, with the line it starts at. Known dialogs only; anything
/// else with an `Esc to cancel` footer stays what it was (a prompt card, or terminal output).
pub(super) fn parse_dialog(lines: &[String]) -> Option<(usize, ScreenDialog)> {
    let last = last_nonblank(lines)?;
    let hints = lines[last].trim().to_string();
    if !(hints.contains("Esc to cancel")
        || hints.contains("Esc to close")
        || hints.contains("Esc to clear"))
    {
        return None;
    }
    let from = last.saturating_sub(MAX_REGION);
    // The dialog sits under a full-width rule; the input box (rule, `❯`, rule) is not one.
    let rule = (from..last).rev().find(|&i| super::is_rule(&lines[i]))?;
    let start = rule + 1;
    let region = &lines[start..last];
    let first = region.iter().find(|l| !l.trim().is_empty())?;
    if first.trim_start().starts_with('❯')
        && region.iter().filter(|l| super::is_rule(l)).count() > 0
    {
        return None;
    }
    let kind = kind_of(region)?;

    let (mut tabs, mut search, mut note, mut rows) = (vec![], None, vec![], vec![]);
    let (mut selected, mut above, mut below) = (0u32, false, false);
    for line in region {
        let t = line.trim();
        if t.is_empty() || super::is_rule(line) {
            continue;
        }
        if t == "Manage MCP servers" || t.starts_with('╭') || t.starts_with('╰') {
            continue;
        }
        if let Some(inner) = t.strip_prefix('│') {
            let inner = inner.trim_end_matches('│').trim();
            if let Some(q) = inner.strip_prefix('⌕') {
                let q = q.trim();
                search = Some(if q.starts_with("Search") {
                    String::new()
                } else {
                    clip(q)
                });
            }
            continue;
        }
        if t.starts_with('…') || (t.starts_with('↓') && t.ends_with("more")) {
            below = true;
            continue;
        }
        if t.starts_with('↑') && t.ends_with("more") {
            above = true;
            continue;
        }
        if rows.len() >= MAX_ROWS {
            break;
        }
        match kind {
            // Antigravity's dialogs have their own reader (`parse_agy_dialog`).
            DialogKind::Generic | DialogKind::Tasks => continue,
            DialogKind::Model | DialogKind::Permissions => {
                if let Some((cursor, up, down, number, rest)) = numbered(line) {
                    above |= up;
                    below |= down;
                    if cursor {
                        selected = rows.len() as u32;
                    }
                    let current = rest.contains('✔');
                    let rest = rest.replace('✔', "");
                    let (label, detail) = split_cols(rest.trim());
                    rows.push(DialogRow {
                        number: Some(number),
                        label,
                        value: None,
                        detail,
                        state: if current {
                            RowState::Current
                        } else {
                            RowState::Plain
                        },
                    });
                } else if tabs.is_empty() && kind == DialogKind::Permissions && t.contains("Allow")
                {
                    tabs = tab_names(t, "Permissions");
                } else {
                    note.push(clip(t));
                }
            }
            DialogKind::Config | DialogKind::Status => {
                if tabs.is_empty() && t.contains("Settings") && t.contains("Usage") {
                    tabs = tab_names(
                        t,
                        if kind == DialogKind::Status {
                            "Status"
                        } else {
                            "Config"
                        },
                    );
                    continue;
                }
                let cursor = t.starts_with('❯');
                let body = t.trim_start_matches('❯').trim();
                if kind == DialogKind::Status {
                    if let Some((k, v)) = body.split_once(':') {
                        rows.push(DialogRow {
                            number: None,
                            label: clip(k.trim()),
                            value: Some(clip(v.trim())),
                            detail: String::new(),
                            state: RowState::Plain,
                        });
                    }
                    continue;
                }
                if body.starts_with("Type to filter") {
                    continue;
                }
                let (label, value) = split_cols(body);
                if value.is_empty() {
                    note.push(label);
                    continue;
                }
                if cursor {
                    selected = rows.len() as u32;
                }
                let state = match value.as_str() {
                    "true" => RowState::On,
                    "false" => RowState::Off,
                    _ => RowState::Plain,
                };
                rows.push(DialogRow {
                    number: None,
                    label,
                    value: Some(value),
                    detail: String::new(),
                    state,
                });
            }
            DialogKind::Mcp => {
                let cursor = t.starts_with('❯');
                let body = t.trim_start_matches('❯').trim();
                let glyph = body.chars().next();
                let state = match glyph {
                    Some('✔') => RowState::Current,
                    Some('◯') => RowState::Off,
                    Some('⚠') => RowState::Warning,
                    _ => RowState::Header,
                };
                if state == RowState::Header {
                    if body.contains("servers")
                        && body.chars().next().is_some_and(|c| c.is_ascii_digit())
                    {
                        note.push(clip(body));
                    } else if !body.starts_with("http") && !body.contains("for help") {
                        rows.push(DialogRow {
                            number: None,
                            label: clip(body),
                            value: None,
                            detail: String::new(),
                            state,
                        });
                    }
                    continue;
                }
                if cursor {
                    selected = rows.len() as u32;
                }
                let (label, detail) = split_cols(body[glyph.map_or(0, char::len_utf8)..].trim());
                rows.push(DialogRow {
                    number: None,
                    label,
                    value: None,
                    detail,
                    state,
                });
            }
        }
    }
    let title = match kind {
        DialogKind::Model => "Select model",
        DialogKind::Permissions => "Permissions",
        DialogKind::Config => "Settings",
        DialogKind::Mcp => "MCP servers",
        DialogKind::Status => "Status",
        DialogKind::Generic | DialogKind::Tasks => "",
    };
    Some((
        start,
        ScreenDialog {
            kind,
            title: title.to_string(),
            tabs,
            search,
            note: note.join("\n"),
            rows,
            selected,
            hints: clip(&hints),
            more_above: above,
            more_below: below,
        },
    ))
}

/// Tab names of a header line (two or more spaces between them); `active` is the one of the
/// dialog (the plain capture does not tell which tab is lit).
fn tab_names(line: &str, active: &str) -> Vec<DialogTab> {
    line.split("  ")
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .take(12)
        .map(|n| DialogTab {
            name: clip(n),
            active: n == active,
        })
        .collect()
}

/// A dialog of Antigravity's CLI. It is drawn under the input box (rule, `>` line, rule) and ends
/// with a `Keyboard:` footer (or a line of `↑/↓` keys). Blocks are separated by blank lines; the
/// list is the block that holds the `>` cursor. Returns the line after the input box's opening
/// rule (the caller cuts the pane there), and the dialog.
pub(super) fn parse_agy_dialog(lines: &[String]) -> Option<(usize, ScreenDialog)> {
    // The last input box: a rule, a `>` line, a rule.
    let close = (2..lines.len()).rev().find(|&i| {
        super::is_rule(&lines[i])
            && lines[i - 1].trim_start().starts_with('>')
            && super::is_rule(&lines[i - 2])
    })?;
    // The model line at the very bottom is not part of the dialog.
    let mut end = lines.len();
    while end > close + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    if end > close + 1 && lines[end - 1].contains(" · ") && super::indent(&lines[end - 1]) > 20 {
        end -= 1;
    }
    let region: Vec<&String> = lines[close + 1..end].iter().collect();
    let footer_start = region.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with("Keyboard:") || t.starts_with("↑/↓")
    })?;
    let hints = region[footer_start..]
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let hints = hints.trim_start_matches("Keyboard:").trim().to_string();
    let body: Vec<&String> = region[..footer_start].to_vec();
    let first = body.iter().find(|l| !l.trim().is_empty())?;
    let title = clip(first.trim().trim_start_matches('└').trim());
    if title.is_empty() {
        return None;
    }
    let kind = match title.as_str() {
        "Settings" => DialogKind::Config,
        "MCP Servers" => DialogKind::Mcp,
        "Permission Config Editor" => DialogKind::Permissions,
        "Tasks" => DialogKind::Tasks,
        _ => DialogKind::Generic,
    };

    // The cursor line and the block around it.
    let cursor = body
        .iter()
        .position(|l| l.trim_start().starts_with("> ") || l.trim() == ">")?;
    let col = super::indent(body[cursor]) + 2;
    let mut lo = cursor;
    while lo > 0 && !body[lo - 1].trim().is_empty() {
        lo -= 1;
    }
    let mut hi = cursor;
    while hi + 1 < body.len() && !body[hi + 1].trim().is_empty() {
        hi += 1;
    }
    let is_row = |i: usize| {
        let l = body[i];
        let t = l.trim();
        i == cursor
            || (super::indent(l) == col
                && !t.starts_with("Search:")
                && !t.contains('━')
                && !t.contains('◂')
                && !super::is_rule(t)
                && !t.starts_with('['))
    };

    let (mut counts, mut search) = (vec![], None);
    let mut rows = vec![];
    let mut selected = 0u32;
    let (mut above, mut below) = (false, false);
    for (i, line) in body.iter().enumerate() {
        let t = line.trim();
        if t.is_empty() || super::is_rule(t) || i == 0 || rows.len() >= MAX_ROWS {
            continue;
        }
        if let Some(q) = t.strip_prefix("Search:") {
            search = Some(clip(q.trim()));
            continue;
        }
        if t.starts_with("Type to search") {
            search = Some(String::new());
            continue;
        }
        if t.starts_with('[') && t.contains(" of ") && t.ends_with(" items]") {
            counts.push(clip(t));
            continue;
        }
        if t.contains('◂') && t.contains('▸') {
            // The effort slider: the word under it says where it is.
            continue;
        }
        if (lo..=hi).contains(&i) && is_row(i) {
            let text = t.strip_prefix('>').unwrap_or(t).trim();
            let (current, text) = match text.strip_suffix("(current)") {
                Some(r) => (true, r.trim_end().to_string()),
                None => (false, text.to_string()),
            };
            let (label, value) = split_cols(&text.replace('\t', "  "));
            if i == cursor {
                selected = rows.len() as u32;
            }
            let on = value == "on";
            let off = value == "off";
            let state = if current {
                RowState::Current
            } else if kind == DialogKind::Config && on {
                RowState::On
            } else if kind == DialogKind::Config && off {
                RowState::Off
            } else if t.starts_with("> ✗") || t.starts_with("✗") {
                RowState::Warning
            } else {
                RowState::Plain
            };
            rows.push(DialogRow {
                number: None,
                label,
                value: (!value.is_empty()).then_some(value),
                detail: String::new(),
                state,
            });
        } else {
            // Text around the list: a heading or an explanation, shown but not selectable.
            rows.push(DialogRow {
                number: None,
                label: clip(t),
                value: None,
                detail: String::new(),
                state: RowState::Header,
            });
        }
    }
    // `above`/`below` come from the "[1-10 of 21 items]" line.
    for n in &counts {
        if let Some(range) = n.strip_prefix('[') {
            let nums: Vec<u32> = range
                .split(|c: char| !c.is_ascii_digit())
                .filter_map(|p| p.parse().ok())
                .collect();
            if let [from, to, total] = nums[..] {
                above = from > 1;
                below = to < total;
            }
        }
    }
    Some((
        close.saturating_sub(1),
        ScreenDialog {
            kind,
            title,
            tabs: vec![],
            search,
            note: String::new(),
            rows,
            selected,
            hints: clip(&hints),
            more_above: above,
            more_below: below,
        },
    ))
}
