use super::*;

pub(super) static OPTION_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

pub(super) fn option_re() -> &'static regex::Regex {
    OPTION_RE.get_or_init(|| {
        regex::Regex::new(
            r"^\s*(?:│\s*)?(?P<cur>[>❯])?\s*(?P<box>\[[ xX✓✔]\]|[☐☑☒◯◉])?\s*(?P<n>\d{1,2})[.)]\s+(?P<label>\S.*?)\s*(?:│)?\s*$",
        )
        .expect("valid option pattern")
    })
}

pub(super) fn is_hint(l: &str) -> bool {
    let t = l.to_lowercase();
    t.contains("navigate")
        || t.contains("enter to select")
        || t.contains("enter to confirm")
        || t.contains("esc to cancel")
}

pub(super) fn unbox(l: &str) -> String {
    l.trim_matches(|c: char| "│╭╰╮╯─".contains(c) || c.is_whitespace())
        .to_string()
}

pub(super) fn box_glyph(label: &str) -> Option<(&str, &str)> {
    for g in ["[ ]", "[x]", "[X]", "[✓]", "[✔]", "☐", "☑", "☒", "◯", "◉"] {
        if let Some(rest) = label.strip_prefix(g) {
            return Some((g, rest.trim_start()));
        }
    }
    None
}

/// The menu at the bottom of the screen: numbered option lines (`1. Yes`, `> 3. …` with the cursor
/// on one) right above a key-hint line ("↑/↓ Navigate · … esc to cancel"). Returns where it starts.
pub(super) fn parse_prompt(lines: &[String]) -> Option<(usize, ScreenPrompt)> {
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
