use super::*;

/// The composer line when nothing is typed.
const PLACEHOLDER: &str = "Ask Codex to do anything";

/// The bottom of Codex's screen: the queue it keeps for follow-ups, the composer (`› draft`) and the
/// two lines under it (model and folder, then the key hints). Returns where it starts and the draft.
/// Codex draws no rules around its input, so the composer is found from the hint line up.
pub(super) fn split_footer(lines: &[String]) -> Option<(usize, Option<String>)> {
    let last = lines.iter().rposition(|l| !l.trim().is_empty())?;
    // `← for agents · ? for shortcuts`, or `tab to queue message` while a turn runs
    if !is_hint_line(&lines[last]) {
        return None;
    }
    let composer = (0..last)
        .rev()
        .take(4)
        .find(|&i| lines[i].starts_with("› "))?;
    let draft = lines[composer].trim_start_matches("› ").trim().to_string();
    let mut cut = composer;
    // the queue sits right above the composer: a title, `↳` entries, the edit hint
    while cut > 0 {
        let prev = &lines[cut - 1];
        let t = prev.trim_start();
        if prev.trim().is_empty() || t.starts_with("↳ ") || t.starts_with("⇧← ") {
            cut -= 1;
        } else if t.starts_with("• Queued follow-up") {
            cut -= 1;
            break;
        } else {
            break;
        }
    }
    let wrapped: Vec<&str> = lines[composer + 1..last]
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.contains(" · ~"))
        .collect();
    let mut draft = draft;
    for w in wrapped {
        draft.push('\n');
        draft.push_str(w);
    }
    let draft = (!draft.is_empty() && draft != PLACEHOLDER).then_some(draft);
    Some((cut, draft))
}

fn is_hint_line(l: &str) -> bool {
    let t = l.trim();
    t.contains("for shortcuts") || t.contains("tab to queue message") || t.starts_with("esc to ")
}

/// `• Working (9s • esc to interrupt) · 1 background terminal running · /ps to view · /stop to close`
pub(super) fn is_working_line(l: &str) -> bool {
    l.starts_with("• Working (") || l.starts_with("• Thinking (")
}

pub(super) fn codex(lines: &[String], b: &mut Builder) {
    let mut other: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        let t = l.trim_start();
        if l.trim().is_empty() || is_art(t) {
            i += 1;
            continue;
        }
        if t.starts_with(">_ OpenAI Codex") {
            flush_other(b, &mut other);
            let mut banner = vec![t.trim_end().to_string()];
            i += 1;
            while i < lines.len() && !lines[i].trim().is_empty() && indent(&lines[i]) >= 2 {
                banner.push(lines[i].trim().to_string());
                i += 1;
            }
            b.system("OpenAI Codex", &banner.join("\n"));
            continue;
        }
        if let Some(rest) = l.strip_prefix("› ") {
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
        if let Some(rest) = l.strip_prefix("• ") {
            flush_other(b, &mut other);
            i += 1;
            if let Some((name, args)) = command_head(rest) {
                let mut out: Vec<String> = Vec::new();
                while i < lines.len() && !lines[i].trim().is_empty() && indent(&lines[i]) >= 2 {
                    let c = lines[i].trim();
                    if !c.starts_with("└ Tip:") {
                        out.push(c.trim_start_matches("└ ").to_string());
                    }
                    i += 1;
                }
                let result = match out.join("\n").trim() {
                    "" | "(no output)" => None,
                    o => Some(o.to_string()),
                };
                b.tool(name, args, result);
                continue;
            }
            let mut para = vec![rest.to_string()];
            while i < lines.len()
                && !lines[i].trim().is_empty()
                && indent(&lines[i]) >= 2
                && !is_worked_line(&lines[i])
                && !lines[i].trim_start().starts_with("└ Tip:")
            {
                para.push(strip_indent(&lines[i], 2));
                i += 1;
            }
            b.text(reflow(&para));
            continue;
        }
        if is_worked_line(l) {
            flush_other(b, &mut other);
            let label = t.split(" • ").next().unwrap_or(t).trim();
            b.system(label, "");
            i += 1;
            continue;
        }
        if let Some(rest) = l.strip_prefix("✗ ").or_else(|| l.strip_prefix("■ ")) {
            flush_other(b, &mut other);
            b.system(rest.trim(), "");
            i += 1;
            continue;
        }
        if t.starts_with("└ Tip:") {
            i += 1;
            continue;
        }
        other.push(l.clone());
        i += 1;
    }
    flush_other(b, &mut other);
}

/// The braille picture of the start screen.
fn is_art(t: &str) -> bool {
    t.chars()
        .next()
        .is_some_and(|c| ('\u{2800}'..='\u{28ff}').contains(&c))
}

/// `  Worked for 22s • 09:36`
fn is_worked_line(l: &str) -> bool {
    l.starts_with("  ") && l.trim_start().starts_with("Worked for ")
}

/// `Ran sleep 20`, `Running ls`, `Exited (status 6) curl -sI …` : the tool call and its command.
fn command_head(rest: &str) -> Option<(String, String)> {
    for p in ["Ran ", "Running "] {
        if let Some(cmd) = rest.strip_prefix(p) {
            return Some(("Bash".into(), cmd.trim().to_string()));
        }
    }
    let after = rest.strip_prefix("Exited (status ")?;
    let (_, cmd) = after.split_once(") ")?;
    Some(("Bash".into(), cmd.trim().to_string()))
}
