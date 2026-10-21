use super::*;

/// Joins lines the terminal wrapped (the agent breaks lines itself, so `-J` cannot): a line is
/// joined to the next one when it reaches the paragraph's widest line and the next one is a plain
/// continuation, not a list item, table row, heading or code.
pub(super) fn reflow(lines: &[String]) -> String {
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let converted = box_tables_to_markdown(lines);
    let lines = &converted[..];
    let mut out: Vec<String> = Vec::new();
    let mut prev_wrapped = false;
    let mut prev_hard = false;
    for l in lines {
        let t = l.trim_start();
        let block_start = t.is_empty()
            || t.starts_with("• ")
            || t.starts_with("- ")
            || t.starts_with("* ")
            || t.starts_with('|')
            // a row of a table the agent drew with pipes: its cells wrap, the rows must not merge
            || t.matches('|').count() >= 2
            || t.starts_with('#')
            || t.starts_with("```")
            || t == "---"
            || t.starts_with('>')
            || t.chars().next().is_some_and(|c| c.is_ascii_digit())
                && t.trim_start_matches(|c: char| c.is_ascii_digit())
                    .starts_with(". ");
        if prev_wrapped && !block_start {
            if let Some(last) = out.last_mut() {
                if !prev_hard {
                    last.push(' ');
                }
                last.push_str(t);
                prev_wrapped = l.chars().count() + 12 >= width && width > 24;
                prev_hard = hard_wrapped(l, width);
                continue;
            }
        }
        out.push(l.to_string());
        prev_wrapped = !t.is_empty()
            && t.matches('|').count() < 2
            && l.chars().count() + 12 >= width
            && width > 24;
        prev_hard = hard_wrapped(l, width);
    }
    out.join("\n")
}

/// A line the terminal cut in the middle of a long word (a path, a URL): it fills the pane to the
/// last column and ends in a long unbroken run. Word-wrapped lines stop short of the edge.
pub(super) fn hard_wrapped(line: &str, width: usize) -> bool {
    let n = line.chars().count();
    width > 24
        && n >= width
        && line
            .rsplit(' ')
            .next()
            .is_some_and(|w| w.chars().count() >= 12)
}

pub(super) fn strip_indent(l: &str, n: usize) -> String {
    let k = indent(l).min(n);
    l.chars().skip(k).collect()
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

/// A tool group the UI folded itself: "Searched for 1 pattern (ctrl+o to expand)".
pub(super) fn folded_summary(l: &str) -> Option<&str> {
    let t = l.trim();
    t.strip_suffix("(ctrl+o to expand)").map(str::trim_end)
}

/// Lines after `⎿`: the first on the marker's line, the rest indented deeper.
pub(super) fn take_result(lines: &[String], i: &mut usize) -> Option<String> {
    let l = lines.get(*i)?;
    let t = l.trim_start();
    let first = t.strip_prefix('⎿')?.trim().to_string();
    let base = indent(l);
    let mut out = vec![first];
    *i += 1;
    while *i < lines.len() {
        let n = &lines[*i];
        if n.is_empty() {
            // a result printed whole keeps its blank lines; what follows is indented well past
            // the result marker, unlike a reply or a thought block
            let next = lines.iter().skip(*i + 1).find(|x| !x.is_empty());
            if next.is_some_and(|x| indent(x) >= base + 4 && !is_thought_header(x)) {
                out.push(String::new());
                *i += 1;
                continue;
            }
            break;
        }
        if indent(n) <= base || is_thought_header(n) {
            break;
        }
        out.push(n.trim().to_string());
        *i += 1;
    }
    Some(out.join("\n"))
}

/// `▾ Thought for 25s, 1.5k tokens`: the header of a thought block the agent has expanded.
pub(super) fn is_thought_header(l: &str) -> bool {
    let t = l.trim_start();
    t.starts_with("▾ Thought for") || t.starts_with("▸ Thought for")
}

/// Unrecognised text between markers, kept folded.
pub(super) fn flush_other(b: &mut Builder, other: &mut Vec<String>) {
    if !other.is_empty() {
        let body = other.join("\n");
        if !body.trim().is_empty() {
            b.system("Terminal output", &body);
        }
        other.clear();
    }
}

// MARK: Claude Code

/// Claude Code draws a Markdown table with box characters (`┌─┬─┐`, `│ a │ b │`, `├─┼─┤`, `└─┴─┘`)
/// and wraps long cells over several `│` lines. Turns each such table back into Markdown rows
/// (`| a | b |` with a `---` rule under the header), so the app renders it as a table and
/// `reflow` never merges its lines.
pub(super) fn box_tables_to_markdown(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if !(t.starts_with('┌') && t.ends_with('┐')) {
            out.push(lines[i].clone());
            i += 1;
            continue;
        }
        let lead: String = lines[i].chars().take_while(|c| c.is_whitespace()).collect();
        // rows are the `│` lines between two separators; their wrapped cell lines are joined
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut current: Option<Vec<String>> = None;
        let mut j = i + 1;
        let mut closed = false;
        while j < lines.len() {
            let r = lines[j].trim();
            if r.starts_with('│') {
                let cells: Vec<String> = r
                    .trim_matches('│')
                    .split('│')
                    .map(|c| c.trim().to_string())
                    .collect();
                match current.as_mut() {
                    Some(row) if row.len() == cells.len() => {
                        for (cell, more) in row.iter_mut().zip(cells) {
                            if !more.is_empty() {
                                if !cell.is_empty() {
                                    cell.push(' ');
                                }
                                cell.push_str(&more);
                            }
                        }
                    }
                    _ => current = Some(cells),
                }
            } else if r.starts_with('├') || r.starts_with('└') {
                if let Some(row) = current.take() {
                    rows.push(row);
                }
                if r.starts_with('└') {
                    closed = true;
                    break;
                }
            } else {
                break;
            }
            j += 1;
        }
        if !closed || rows.is_empty() {
            // not a table after all: keep the lines as they were
            out.push(lines[i].clone());
            i += 1;
            continue;
        }
        let md = |cells: &[String]| format!("{lead}| {} |", cells.join(" | "));
        out.push(md(&rows[0]));
        out.push(format!("{lead}|{}", " --- |".repeat(rows[0].len())));
        for row in &rows[1..] {
            out.push(md(row));
        }
        i = j + 1;
    }
    out
}
