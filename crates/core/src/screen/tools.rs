use super::*;

pub(super) fn norm(s: &str, n: usize) -> String {
    s.split(['*', '`'])
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .chars()
        .take(n)
        .collect()
}

/// The screen of Antigravity names its calls like Claude's (`Bash`, `Read`, `Edit`), its session
/// file by the tool (`run_command`, `view_file`, `replace_file_content`): same call, same class.
pub(super) fn tool_class(name: &str) -> &str {
    match name {
        "Bash" | "run_command" => "bash",
        "Read" | "read_file" | "view_file" => "read",
        "Edit"
        | "Write"
        | "edit_file"
        | "write_file"
        | "write_to_file"
        | "replace_file_content"
        | "multi_replace_file_content" => "edit",
        "Grep" | "grep_search" => "grep",
        "Glob" | "find_by_name" => "glob",
        "LS" | "list_dir" => "ls",
        other => other,
    }
}

/// Tools the agent's screen folds into "Searched for N patterns" / "Read N files" groups.
pub(super) const FOLDABLE_TOOLS: [&str; 15] = [
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

/// Lines of a tool result worth showing, in order: blanks, the UI's own "… +N lines (ctrl+o to
/// expand)" hints and runs of build progress ("Compiling x", "Downloaded y") are filtered out, not
/// cut off; a run of progress lines becomes one line saying how many there were.
pub(super) fn readable_result_lines(result: &str) -> Vec<String> {
    let progress = |l: &str| {
        let t = l.trim_start();
        [
            "Compiling ",
            "Downloaded ",
            "Downloading ",
            "Fresh ",
            "Checking ",
            "Blocking ",
        ]
        .iter()
        .any(|p| t.starts_with(p))
    };
    let mut out: Vec<String> = Vec::new();
    let mut run: Option<(String, usize)> = None;
    let flush = |out: &mut Vec<String>, run: &mut Option<(String, usize)>| {
        if let Some((first, n)) = run.take() {
            out.push(if n > 1 {
                format!("{} (+{} similar lines)", first.trim(), n - 1)
            } else {
                first
            });
        }
    };
    for l in result.lines() {
        let t = l.trim();
        if t.is_empty() || t.ends_with("(ctrl+o to expand)") || is_more_marker(t) {
            continue;
        }
        if progress(l) {
            match run.as_mut() {
                Some((_, n)) => *n += 1,
                None => run = Some((l.to_string(), 1)),
            }
            continue;
        }
        flush(&mut out, &mut run);
        out.push(l.to_string());
    }
    flush(&mut out, &mut run);
    out
}

/// "… +13 lines" (what the screen prints where it cut a result).
pub(super) fn is_more_marker(t: &str) -> bool {
    // Claude prints "… +13 lines", Antigravity prints "<output +73 lines>".
    let count = t
        .strip_prefix("… +")
        .and_then(|r| r.strip_suffix(" lines"))
        .or_else(|| {
            t.strip_prefix("<output +")
                .and_then(|r| r.strip_suffix(" lines>"))
        });
    count.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Lines of one result kept in a folded group, and characters of one line.
pub(super) const GROUP_RESULT_LINES: usize = 40;
pub(super) const GROUP_LINE_CHARS: usize = 400;

/// Fills a folded group with the calls the session file knows: what was searched / read, and the
/// result of each, filtered for noise and shown at length (not just the first lines).
pub(super) fn describe_group(group: &mut TranscriptToolCall, calls: &[TranscriptToolCall]) {
    let mut lines: Vec<String> = Vec::new();
    for c in calls {
        lines.push(format!("{}({})", c.tool_name, c.arguments_summary));
        if let Some(r) = c.result.as_deref() {
            let all = readable_result_lines(r);
            for l in all.iter().take(GROUP_RESULT_LINES) {
                lines.push(format!("  {}", clip(l, GROUP_LINE_CHARS)));
            }
            if all.len() > GROUP_RESULT_LINES {
                lines.push(format!("  … +{} lines", all.len() - GROUP_RESULT_LINES));
            }
        }
    }
    group.arguments_summary = calls
        .iter()
        .map(|c| clip(&c.arguments_summary, 80))
        .collect::<Vec<_>>()
        .join(", ");
    group.result = Some(lines.join("\n"));
}

/// Puts the agent's thinking (which the screen does not show) into the screen's feed, each block
/// right before the reply text or tool call it preceded, so the order of the story is kept.
/// `file` is the session file's turns; a turn there is matched to the screen turn with the same
/// reply text or the same tool call (name and the start of its arguments), moving forward only.
/// Whitespace-collapsed text without Markdown emphasis marks, so a reply compares equal whether
/// or not its styles were turned into `**` and backticks on either side.
pub(super) fn squash(s: &str) -> String {
    let plain: String = s.chars().filter(|c| !matches!(c, '*' | '`')).collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}
