use super::*;

pub(super) fn claude(lines: &[String], b: &mut Builder) {
    let pane_width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
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
        if let Some(rest) = l.strip_prefix("⏺ ").or_else(|| running_tool(lines, i)) {
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
                    if !hard_wrapped(&lines[i - 1], pane_width) {
                        args.push(' ');
                    }
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
                // "Allowed by auto mode classifier" follows every auto-approved call: it is the
                // reason a call ran, not news, and it would split the run of calls the feed folds.
                if out.trim() != "Allowed by auto mode classifier" {
                    b.system("Note", &out);
                }
            }
            continue;
        }
        other.push(l.clone());
        i += 1;
    }
    flush_other(b, &mut other);
}

/// The `⏺` of a running call blinks, so a capture can catch the call without it: `  Bash(…` at the
/// indent of a reply, arguments indented deeper, and the `⎿` result line right below.
pub(super) fn running_tool(lines: &[String], i: usize) -> Option<&str> {
    let l = &lines[i];
    if indent(l) != 2 {
        return None;
    }
    let t = l.trim_start();
    tool_head(t)?;
    for n in lines.iter().skip(i + 1).take(12) {
        if n.trim_start().starts_with('⎿') {
            return Some(t);
        }
        if n.is_empty() || indent(n) < 4 {
            return None;
        }
    }
    None
}

/// `Bash(ls -la …` -> ("Bash", "ls -la …"). Prose with a bracket in it ("Это (тест)") is not a call:
/// the name must be a bare identifier.
pub(super) fn tool_head(rest: &str) -> Option<(&str, &str)> {
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

/// An expanded thought block is wrapped by the agent itself, and the wrapped lines come back at
/// column 0 (the first line of each paragraph is indented). Such a line belongs to the line above,
/// up to the next marker (a call, a result, a prompt, a rule, a spinner).
pub(super) fn join_thought_wraps(lines: &[String]) -> Vec<String> {
    let is_marker = |l: &str| {
        l.starts_with(|c: char| "●○⎿>─⚠┃└▾▸".contains(c) || ('\u{2800}'..='\u{28ff}').contains(&c))
    };
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut in_thought = false;
    for l in lines {
        if is_thought_header(l) {
            in_thought = true;
        } else if l.is_empty() {
            // blank lines stay: they separate paragraphs
        } else if is_marker(l.trim_start()) && indent(l) < 2 {
            in_thought = false;
        } else if in_thought && indent(l) == 0 {
            if let Some(prev) = out.last_mut().filter(|p| indent(p) >= 2) {
                prev.push(' ');
                prev.push_str(l.trim());
                continue;
            }
        }
        out.push(l.clone());
    }
    out
}
