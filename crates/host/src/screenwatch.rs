//! Questions on the screen of agents without hooks (Antigravity): the agent reads the tmux pane of
//! each such session and reports `waiting` while a numbered menu with a key-hint line is shown.

use crate::proc::run_with_input;
use std::process::Command;
use std::time::Duration;

fn numbered(line: &str) -> Option<u32> {
    let t = line.trim_start().trim_start_matches(['│', '>', '❯', ' ']);
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 2 {
        return None;
    }
    let rest = &t[digits.len()..];
    (rest.starts_with(". ") || rest.starts_with(") ")).then(|| digits.parse().ok())?
}

/// True when the bottom of the screen is a menu: a key-hint line ("↑/↓ Navigate", "enter to select")
/// right below numbered options counting 1..n. A numbered list in an ordinary reply, or the
/// working footer ("esc to cancel" alone), is not one.
pub fn asks(screen: &str) -> bool {
    let lines: Vec<&str> = screen.lines().collect();
    let n = lines.len();
    let hint = (0..n).rev().take(8).find(|&i| {
        let t = lines[i].to_lowercase();
        t.contains("navigate") || t.contains("enter to select") || t.contains("enter to confirm")
    });
    let Some(end) = hint else { return false };
    let mut want: Option<u32> = None;
    let mut count = 0;
    let mut reached_one = false;
    let mut last_start = None;
    for i in (end.saturating_sub(40)..end).rev() {
        if let Some(k) = numbered(lines[i]) {
            if want.is_none() {
                want = Some(k);
                last_start = Some(i);
            }
            if Some(k) == want {
                count += 1;
                if k == 1 {
                    reached_one = true;
                    break;
                }
                want = Some(k - 1);
            }
        }
    }
    let Some(last) = last_start else { return false };
    let tail = lines[last + 1..end]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .count();
    count >= 2 && reached_one && tail <= 5
}

/// The visible text of the active pane of a tmux session, when there is one.
pub fn capture(tmux: &str) -> Option<String> {
    if tmux.is_empty() || tmux.contains(['\n', '\0']) {
        return None;
    }
    let mut cmd = Command::new("tmux");
    cmd.args(["capture-pane", "-p", "-t", &format!("={tmux}:")]);
    cmd.env_remove("TMUX").env_remove("TMUX_PANE");
    match run_with_input(cmd, b"", Duration::from_secs(3)) {
        Some((Some(0), out)) => Some(String::from_utf8_lossy(&out).into_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::asks;

    #[test]
    fn menu_with_hint_asks() {
        let s = "Which one?\n\n  > 1. Alpha\n    2. Beta\n    3. Gamma\n\n ↑/↓ Navigate · enter to select · esc to cancel\n";
        assert!(asks(s));
    }

    #[test]
    fn working_footer_and_plain_lists_do_not_ask() {
        assert!(!asks("Working…\n\n esc to cancel\n"));
        assert!(!asks("Plan:\n1. one\n2. two\n3. three\n\n> \n"));
        assert!(!asks(""));
    }

    #[test]
    fn list_not_starting_at_one_does_not_ask() {
        assert!(!asks("x\n  2. Beta\n  3. Gamma\n\n ↑/↓ Navigate\n"));
    }
}
