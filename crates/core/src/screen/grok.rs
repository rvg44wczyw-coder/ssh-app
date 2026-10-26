use super::*;

/// The composer when nothing is typed.
const PLACEHOLDER: &str = "Build anything";

/// A pane line without the scrollbar Grok draws down the right edge.
fn clean(l: &str) -> &str {
    l.trim_end_matches(['█', ' '])
}

/// `   ❯ text      10:40 AM`: the clock Grok puts at the end of a message line.
fn strip_clock(l: &str) -> &str {
    let t = l.trim_end();
    for suffix in [" AM", " PM"] {
        if let Some(rest) = t.strip_suffix(suffix) {
            let digits = rest.rsplit(' ').next().unwrap_or("");
            let ok = digits.len() >= 4
                && digits.contains(':')
                && digits.chars().all(|c| c.is_ascii_digit() || c == ':');
            if ok {
                return rest[..rest.len() - digits.len()].trim_end();
            }
        }
    }
    t
}

fn is_header(l: &str) -> bool {
    l.contains("[Dashboard]") && l.contains(" / ")
}

fn is_notice_title(l: &str) -> bool {
    l.trim_start().starts_with("Help improve Grok")
}

/// The boxed composer at the bottom: `╭──╮`, `│ ❯ draft │`, `╰── Grok 4.7 (high) ─╯`, then a hint line.
/// Returns where the footer starts (above the data-retention notice when it is shown) and the draft.
pub(super) fn split_footer(lines: &[String]) -> Option<(usize, Option<String>)> {
    let last = lines.iter().rposition(|l| !l.trim().is_empty())?;
    let bottom = (0..=last)
        .rev()
        .take(3)
        .find(|&i| lines[i].trim_start().starts_with('╰') && lines[i].contains("Grok"))?;
    let top = (0..bottom)
        .rev()
        .take(8)
        .find(|&i| lines[i].trim_start().starts_with('╭'))?;
    let draft: Vec<String> = lines[top + 1..bottom]
        .iter()
        .map(|l| {
            let t = l.trim().trim_matches('│').trim();
            t.strip_prefix('❯').unwrap_or(t).trim().to_string()
        })
        .filter(|l| !l.is_empty())
        .collect();
    let draft = draft.join("\n");
    let draft = (!draft.is_empty() && draft != PLACEHOLDER).then_some(draft);
    Some((above_blank(lines, notice_start(lines, top)), draft))
}

/// Moves a cut up over the blank lines and the scrollbar-only lines above it.
fn above_blank(lines: &[String], mut at: usize) -> usize {
    while at > 0 && clean(&lines[at - 1]).trim().is_empty() {
        at -= 1;
    }
    at
}

/// The data-retention notice sits right above the composer: its first line, or `at` when there is none.
fn notice_start(lines: &[String], at: usize) -> usize {
    (at.saturating_sub(8)..at)
        .find(|&i| is_notice_title(&lines[i]))
        .unwrap_or(at)
}

/// A permission dialog replaces the composer: `┃  › Allow Edit to …?`, `┃  2 (●) Yes, …`, a hint line with
/// `1/4:select`.
pub(super) fn prompt(lines: &[String]) -> Option<(usize, ScreenPrompt)> {
    picker(lines).or_else(|| permission(lines))
}

/// `/effort`, `/model`: a list drawn between two rules right above the composer, the cursor (`❯`) on one entry,
/// the composer holding the command with its placeholder (`/effort <level>`). The plain slash menu (the
/// composer holds `/` or a word) is not a question.
fn picker(lines: &[String]) -> Option<(usize, ScreenPrompt)> {
    let bottom = lines
        .iter()
        .rposition(|l| l.trim_start().starts_with('╰') && l.contains("Grok"))?;
    let top = (0..bottom)
        .rev()
        .take(8)
        .find(|&i| lines[i].trim_start().starts_with('╭'))?;
    let command = lines[top + 1..bottom]
        .iter()
        .map(|l| {
            l.trim()
                .trim_matches('│')
                .trim()
                .trim_start_matches('❯')
                .trim()
                .to_string()
        })
        .find(|l| l.starts_with('/'))?;
    if !command.contains(' ') {
        return None;
    }
    let is_rule = |l: &str| {
        let t = clean(l).trim();
        t.chars().count() >= 20
            && t.chars().all(|c| c == '─' || c.is_ascii_digit())
            && t.starts_with('─')
    };
    let rule2 = (0..top).rev().take(3).find(|&i| is_rule(&lines[i]))?;
    let rule1 = (0..rule2).rev().take(40).find(|&i| is_rule(&lines[i]))?;
    let mut options = Vec::new();
    let mut selected = 0;
    for l in &lines[rule1 + 1..rule2] {
        let t = clean(l).trim_start();
        if t.is_empty() {
            continue;
        }
        let cursor = t.starts_with('❯');
        let body = t.trim_start_matches('❯').trim();
        let (label, description) = split_entry(body);
        if cursor {
            selected = options.len() as u32;
        }
        options.push(PromptOption {
            number: options.len() as u32 + 1,
            label,
            description,
            checked: false,
        });
    }
    if options.is_empty() {
        return None;
    }
    let name = command.split(' ').next().unwrap_or("/").to_string();
    Some((
        above_blank(lines, notice_start(lines, rule1)),
        ScreenPrompt {
            title: None,
            detail: String::new(),
            question: format!("Choose ({name})"),
            options,
            selected,
            multi: false,
        },
    ))
}

/// `Grok 4.7 (current) SpaceXAI's latest…` or `Extra High    Maximum reasoning…`: the name and the note.
fn split_entry(body: &str) -> (String, String) {
    for mark in ["(current)", "(active)"] {
        if let Some(p) = body.find(mark) {
            let end = p + mark.len();
            return (body[..end].to_string(), body[end..].trim().to_string());
        }
    }
    match body.split_once("  ") {
        Some((a, b)) => (a.trim().to_string(), b.trim().to_string()),
        None => (body.to_string(), String::new()),
    }
}

fn permission(lines: &[String]) -> Option<(usize, ScreenPrompt)> {
    let last = lines.iter().rposition(|l| !l.trim().is_empty())?;
    if !(lines[last].contains(":select") && lines[last].contains("Tab:next option")) {
        return None;
    }
    let q = (0..last)
        .rev()
        .take(14)
        .find(|&i| lines[i].trim_start().starts_with("┃") && lines[i].contains(" ›  "))?;
    let question = lines[q].split(" ›  ").nth(1)?.trim().to_string();
    let mut options = Vec::new();
    let mut selected = 0;
    for l in &lines[q + 1..last] {
        let t = l.trim_start().trim_start_matches('┃').trim();
        let Some((n, rest)) = t.split_once(' ') else {
            continue;
        };
        let Ok(number) = n.parse::<u32>() else {
            continue;
        };
        let rest = rest.trim_start();
        let (on, label) = if let Some(r) = rest.strip_prefix("(●)") {
            (true, r)
        } else if let Some(r) = rest.strip_prefix("(○)") {
            (false, r)
        } else {
            continue;
        };
        if on {
            selected = options.len() as u32;
        }
        options.push(PromptOption {
            number,
            label: label.trim().to_string(),
            description: String::new(),
            checked: false,
        });
    }
    if options.len() < 2 {
        return None;
    }
    // the blank `┃` line above the question starts the dialog, and the data-retention notice above it
    let mut start = q;
    if q > 0 && lines[q - 1].trim() == "┃" {
        start = q - 1;
    }
    Some((
        above_blank(lines, notice_start(lines, start)),
        ScreenPrompt {
            title: None,
            detail: String::new(),
            question,
            options,
            selected,
            multi: false,
        },
    ))
}

/// `⠴ Waiting for response… 0.3s   3.0s ⇣20.1k [stop]` is shared with Antigravity's braille spinner; the
/// queue Grok lists under it is `#1 text`.
pub(super) fn is_queue_line(l: &str) -> bool {
    let t = l.trim_start();
    t.strip_prefix('#')
        .is_some_and(|r| r.chars().next().is_some_and(|c| c.is_ascii_digit()))
}

pub(super) fn grok(lines: &[String], b: &mut Builder) {
    let lines: Vec<String> = lines.iter().map(|l| clean(l).to_string()).collect();
    let mut other: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        let t = l.trim_start();
        if l.trim().is_empty()
            || is_header(l)
            || t.contains("[Send now]")
            || t.contains("[✗]")
            || is_queue_line(l)
            // the task strip under the header (`▾ Tasks 1`, `⁙ Task … [↗][✗]`)
            || t.starts_with("▾ Tasks")
            || t.starts_with("⁙ ")
        {
            i += 1;
            continue;
        }
        // the box Grok draws around a message it sends at once
        let t = t.trim_matches(['┌', '└', '┐', '┘', '│', '─']).trim();
        if t.is_empty() {
            i += 1;
            continue;
        }
        if t.starts_with('╭') && lines[i..].iter().take(4).any(|x| x.contains("Grok Build")) {
            flush_other(b, &mut other);
            let mut text: Vec<String> = Vec::new();
            while i < lines.len() && !lines[i].trim_start().starts_with('╰') {
                let inner = lines[i]
                    .trim()
                    .trim_matches(['│', '╭', '╮', '─'])
                    .trim()
                    .trim_start_matches(|c: char| ('\u{2800}'..='\u{28ff}').contains(&c))
                    .trim();
                if !inner.is_empty() {
                    text.push(inner.to_string());
                }
                i += 1;
            }
            i += 1;
            b.system("Grok Build", &text.join("\n"));
            continue;
        }
        if let Some(rest) = t.strip_prefix("❯ ") {
            flush_other(b, &mut other);
            let mut para = vec![strip_clock(rest).trim().to_string()];
            i += 1;
            while i < lines.len() && !lines[i].trim().is_empty() && indent(&lines[i]) >= 6 {
                para.push(strip_clock(lines[i].trim()).to_string());
                i += 1;
            }
            b.user(reflow(&para));
            continue;
        }
        if let Some(rest) = t.strip_prefix("◆ Thought for ") {
            flush_other(b, &mut other);
            b.system(&format!("Thought for {}", strip_clock(rest)), "");
            i += 1;
            continue;
        }
        if t.starts_with("◆ Thinking") || t.starts_with("┃") {
            flush_other(b, &mut other);
            let mut text: Vec<String> = Vec::new();
            if t.starts_with("┃") {
                text.push(t.trim_start_matches('┃').trim().to_string());
            }
            i += 1;
            while i < lines.len() && lines[i].trim_start().starts_with('┃') {
                text.push(
                    lines[i]
                        .trim_start()
                        .trim_start_matches('┃')
                        .trim()
                        .to_string(),
                );
                i += 1;
            }
            let text: Vec<String> = text
                .into_iter()
                .filter(|x| !x.is_empty() && !x.starts_with("◆ Thinking") && x != "…")
                .collect();
            if !text.is_empty() {
                b.thinking(reflow(&text));
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("◈ ").or_else(|| t.strip_prefix("◆ ")) {
            flush_other(b, &mut other);
            i += 1;
            let (name, args) = tool_head(rest);
            let mut out: Vec<String> = Vec::new();
            while i < lines.len() && !lines[i].trim().is_empty() && indent(&lines[i]) >= 7 {
                out.push(strip_clock(lines[i].trim()).to_string());
                i += 1;
            }
            let result = (!out.is_empty()).then(|| out.join("\n"));
            b.tool(name, args, result);
            continue;
        }
        if let Some(rest) = t.strip_prefix("Worked for ") {
            flush_other(b, &mut other);
            b.system(&format!("Worked for {}", rest.trim()), "");
            i += 1;
            continue;
        }
        if t.starts_with("Turn cancelled") {
            flush_other(b, &mut other);
            b.system(strip_clock(t), "");
            i += 1;
            continue;
        }
        if indent(l) >= 4 {
            flush_other(b, &mut other);
            let mut para = vec![strip_clock(t).to_string()];
            i += 1;
            while i < lines.len() {
                let n = &lines[i];
                let nt = n.trim_start();
                if n.trim().is_empty()
                    || indent(n) < 4
                    || nt.starts_with("❯ ")
                    || nt.starts_with("◆ ")
                    || nt.starts_with("◈ ")
                    || nt.starts_with("Worked for ")
                    || nt.starts_with("Turn cancelled")
                    || nt.starts_with("┃")
                {
                    break;
                }
                para.push(strip_clock(nt).to_string());
                i += 1;
            }
            b.text(reflow(&para));
            continue;
        }
        other.push(l.clone());
        i += 1;
    }
    flush_other(b, &mut other);
}

/// `Ran print hello-grok`, `Explored docs`, `List docs (175 entries)`, `Creating path`: the tool and what it was given.
fn tool_head(rest: &str) -> (String, String) {
    let rest = strip_clock(rest).trim();
    let table = [
        ("Ran ", "Bash"),
        ("Running ", "Bash"),
        ("Explored ", "LS"),
        ("Exploring ", "LS"),
        ("List ", "LS"),
        ("Read ", "Read"),
        ("Reading ", "Read"),
        ("Creating ", "Write"),
        ("Created ", "Write"),
        ("Edited ", "Edit"),
        ("Editing ", "Edit"),
        ("Searched ", "Grep"),
        ("Searching ", "Grep"),
    ];
    for (prefix, name) in table {
        if let Some(args) = rest.strip_prefix(prefix) {
            return (name.to_string(), args.trim().to_string());
        }
    }
    (rest.to_string(), String::new())
}
