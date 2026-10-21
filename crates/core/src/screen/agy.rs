use super::*;

pub(super) fn agy(lines: &[String], b: &mut Builder) {
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
                && !is_thought_header(&lines[i])
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
            // `Bash(ls -la)` (a call shown whole) or `Ran (ls -la)` / `Explored 2 files (a, b)`
            let (name, args) = if let Some((name, args)) = tool_head(&full) {
                let args = args.trim_end().strip_suffix(')').unwrap_or(args);
                (name.to_string(), args.trim().to_string())
            } else {
                match full.find(" (") {
                    Some(p) => (full[..p].to_string(), full[p + 1..].to_string()),
                    None => (full, String::new()),
                }
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
        if is_thought_header(l) {
            flush_other(b, &mut other);
            b.system(t.trim_start_matches(['▾', '▸']).trim(), "");
            i += 1;
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
                    if next.is_some_and(|x| {
                        indent(x) >= 2 && !x.trim_start().starts_with('⎿') && !is_thought_header(x)
                    }) {
                        blocks.push(reflow(&para));
                        para = Vec::new();
                        i += 1;
                        continue;
                    }
                    break;
                }
                if indent(n) < 2 || n.trim_start().starts_with('⎿') || is_thought_header(n) {
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
