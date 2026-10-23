//! Reading a session file by cursor: the host script returns only the complete lines after
//! `offset` (or, when the file was replaced, shrank or there is no cursor yet, the usual window),
//! with the new cursor. The script runs under plain `sh`; it never takes a path that was not
//! checked by `validate_transcript_path`, and the resolved real path is checked again.

use crate::error::SshCoreError;
use crate::tmux::shell_quote;

/// Where the app has read to: the file's inode and the byte offset after the last complete line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FeedCursor {
    pub inode: u64,
    pub offset: u64,
}

/// The answer of a read by cursor.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FeedRead {
    /// The data replaces what the app has (first read, file replaced or shorter): call
    /// `FeedState::reset` before applying it.
    pub reset: bool,
    pub cursor: FeedCursor,
    /// New complete lines (may be empty).
    pub text: String,
    /// The resolved real path, to be checked by the caller.
    pub path: String,
}

const FILTER: &str = r#"LC_ALL=C grep -aEv '^\{"type":"(api-request-blob|api-request|mode|atis-latch|ai-title|last-prompt|queue-operation|file-history-snapshot)"'"#;

/// The folders a session file must never be read from, as `case` patterns for the resolved real
/// path (the same list as `validate_transcript_path`): a link in an allowed folder cannot lead
/// the host to read `~/.ssh` and send it over the wire before the app can refuse it.
const BLOCKED_PATTERNS: &str = "/etc/*|/var/*|/dev/*|/proc/*|/sys/*|/private/etc/*|/private/var/*|*/.ssh/*|*/.gnupg/*|*/.aws/*|*/Library/Keychains/*";

/// `$1` path, `$2` inode of the cursor or `-`, `$3` offset of the cursor or `-`.
pub fn delta_script() -> String {
    delta_script_with(true)
}

/// `guard: false` only in tests, whose temporary folders are under the blocked `/var`.
pub(crate) fn delta_script_with(guard: bool) -> String {
    let check = if guard {
        format!("case \"$r\" in {BLOCKED_PATTERNS}) echo NOFILE; exit 0;; esac\n")
    } else {
        String::new()
    };
    format!(
        r#"r=$(realpath -- "$1" 2>/dev/null || readlink -f -- "$1" 2>/dev/null) && [ -f "$r" ] || {{ echo NOFILE; exit 0; }}
{check}st=$(stat -c '%i %s' "$r" 2>/dev/null || stat -f '%i %z' "$r" 2>/dev/null) || {{ echo NOFILE; exit 0; }}
ino=${{st%% *}}; size=${{st##* }}
mode=delta
if [ "$2" = - ] || [ "$3" = - ] || [ "$2" != "$ino" ] || [ "$3" -gt "$size" ] 2>/dev/null; then mode=full; off=0; else off=$3; fi
lastb=$(head -c "$size" "$r" | tail -c 1 | od -An -tx1 | tr -d ' \n')
if [ "$lastb" = 0a ] || [ "$size" -le "$off" ]; then part=0; else part=$(head -c "$size" "$r" | tail -c +$((off+1)) | tail -n 1 | wc -c | tr -d ' '); fi
used=$((size - part))
printf 'TP_PATH=%s\nMODE %s\nCUR %s %s\n' "$r" "$mode" "$ino" "$used"
if [ "$mode" = full ]; then
  head -c "$used" "$r" | tail -c 32000000 | {FILTER} | tail -n 3000 | tail -c 4000000
else
  head -c "$used" "$r" | tail -c +$((off+1)) | {FILTER}
fi
exit 0"#
    )
}

/// The command to run on the host (plain `sh`, arguments quoted).
pub fn build_delta_command(path: &str, cursor: Option<FeedCursor>) -> Result<String, SshCoreError> {
    crate::transcript::validate_transcript_path(path).map_err(|e| {
        SshCoreError::TranscriptError {
            reason: e.to_string(),
        }
    })?;
    let (ino, off) = match cursor {
        Some(c) => (c.inode.to_string(), c.offset.to_string()),
        None => ("-".to_string(), "-".to_string()),
    };
    Ok(format!(
        "exec sh -c {} sh {} {} {}",
        shell_quote(&delta_script()),
        shell_quote(path),
        ino,
        off
    ))
}

/// Reads the script's answer. `Ok(None)`: the file does not exist.
pub fn parse_delta_output(out: &str) -> Result<Option<FeedRead>, SshCoreError> {
    parse_inner(out, true)
}

/// The virtual path of an Antigravity conversation read from the agent's database.
pub(crate) fn is_agy_path(path: &str) -> bool {
    path.strip_prefix("agy:")
        .is_some_and(|id| id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
}

/// `check_path: false` only in tests, whose temporary folders are under the blocked `/var`.
pub(crate) fn parse_inner(out: &str, check_path: bool) -> Result<Option<FeedRead>, SshCoreError> {
    if out.trim_start().starts_with("NOFILE") {
        return Ok(None);
    }
    let bad = || SshCoreError::TranscriptError {
        reason: "Unexpected answer of the host".to_string(),
    };
    let mut it = out.splitn(4, '\n');
    let path = it
        .next()
        .and_then(|l| l.strip_prefix("TP_PATH="))
        .ok_or_else(bad)?;
    let mode = it
        .next()
        .and_then(|l| l.strip_prefix("MODE "))
        .ok_or_else(bad)?;
    let cur = it
        .next()
        .and_then(|l| l.strip_prefix("CUR "))
        .ok_or_else(bad)?;
    let (ino, off) = cur.split_once(' ').ok_or_else(bad)?;
    if check_path && !is_agy_path(path) {
        crate::transcript::validate_transcript_path(path).map_err(|e| {
            SshCoreError::TranscriptError {
                reason: e.to_string(),
            }
        })?;
    }
    Ok(Some(FeedRead {
        reset: mode == "full",
        cursor: FeedCursor {
            inode: ino.trim().parse().map_err(|_| bad())?,
            offset: off.trim().parse().map_err(|_| bad())?,
        },
        text: it.next().unwrap_or("").to_string(),
        path: path.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::Command;

    fn run(path: &std::path::Path, cursor: Option<FeedCursor>) -> Option<FeedRead> {
        let (ino, off) = match cursor {
            Some(c) => (c.inode.to_string(), c.offset.to_string()),
            None => ("-".into(), "-".into()),
        };
        let out = Command::new("sh")
            .arg("-c")
            .arg(delta_script_with(false))
            .arg("sh")
            .arg(path)
            .arg(ino)
            .arg(off)
            .output()
            .expect("sh");
        parse_inner(&String::from_utf8_lossy(&out.stdout), false).expect("parse")
    }

    fn user(n: usize) -> String {
        format!(
            "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"m{n} Привет\"}}}}\n"
        )
    }

    #[test]
    fn first_read_is_full_and_later_reads_return_only_new_complete_lines() {
        let dir = tempfile::tempdir().expect("dir");
        let f = dir.path().join("s.jsonl");
        std::fs::write(
            &f,
            format!(
                "{{\"type\":\"mode\",\"mode\":\"x\"}}\n{}{}",
                user(1),
                user(2)
            ),
        )
        .expect("w");
        let a = run(&f, None).expect("file");
        assert!(a.reset);
        assert_eq!(
            a.text.lines().count(),
            2,
            "the filtered records are not sent: {:?}",
            a.text
        );
        assert_eq!(a.cursor.offset, std::fs::metadata(&f).expect("m").len());

        // Nothing new: an empty delta, same cursor.
        let b = run(&f, Some(a.cursor)).expect("file");
        assert!(!b.reset && b.text.is_empty());
        assert_eq!(b.cursor, a.cursor);

        // One full line and half of another: only the full one is returned, the half waits.
        let mut h = std::fs::OpenOptions::new()
            .append(true)
            .open(&f)
            .expect("open");
        let half = user(4);
        h.write_all(format!("{}{}", user(3), &half[..20]).as_bytes())
            .expect("w");
        let c = run(&f, Some(b.cursor)).expect("file");
        assert!(!c.reset);
        assert_eq!(c.text, user(3));
        h.write_all(&half.as_bytes()[20..]).expect("w");
        let d = run(&f, Some(c.cursor)).expect("file");
        assert_eq!(
            d.text, half,
            "the half line arrives whole once it is finished"
        );
        assert_eq!(d.cursor.offset, std::fs::metadata(&f).expect("m").len());
    }

    #[test]
    fn a_replaced_or_shorter_file_gives_a_reset_and_a_missing_one_gives_none() {
        let dir = tempfile::tempdir().expect("dir");
        let f = dir.path().join("s.jsonl");
        std::fs::write(&f, format!("{}{}{}", user(1), user(2), user(3))).expect("w");
        let a = run(&f, None).expect("file");
        // Shorter (rewritten in place).
        std::fs::write(&f, user(9)).expect("w");
        let b = run(&f, Some(a.cursor)).expect("file");
        assert!(b.reset);
        assert_eq!(b.text, user(9));
        // Replaced by another file (new inode) of the same length.
        let g = dir.path().join("other.jsonl");
        std::fs::write(&g, user(8)).expect("w");
        std::fs::rename(&g, &f).expect("rename");
        let c = run(&f, Some(b.cursor)).expect("file");
        assert!(c.reset, "another inode is a new file");
        assert!(run(&dir.path().join("nope.jsonl"), None).is_none());
    }

    #[test]
    fn the_first_read_keeps_the_window_of_the_newest_records() {
        let dir = tempfile::tempdir().expect("dir");
        let f = dir.path().join("big.jsonl");
        let mut s = String::new();
        for i in 0..3100 {
            s.push_str(&user(i));
        }
        std::fs::write(&f, s).expect("w");
        let a = run(&f, None).expect("file");
        assert_eq!(a.text.lines().count(), 3000);
        assert!(a.text.lines().last().expect("last").contains("m3099"));
    }

    #[test]
    fn replaying_the_deltas_equals_parsing_the_file() {
        let dir = tempfile::tempdir().expect("dir");
        let f = dir.path().join("s.jsonl");
        std::fs::write(&f, "").expect("w");
        let state = crate::feed::FeedState::new();
        let mut cursor = None;
        let mut all = String::new();
        for i in 0..40 {
            let line = if i % 5 == 4 {
                format!("{{\"type\":\"assistant\",\"message\":{{\"id\":\"m{i}\",\"role\":\"assistant\",\"content\":[{{\"type\":\"tool_use\",\"id\":\"c{i}\",\"name\":\"Bash\",\"input\":{{\"command\":\"ls\"}}}}]}}}}\n")
            } else if i % 5 == 0 && i > 0 {
                format!("{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"c{}\",\"content\":\"ok\"}}]}}}}\n", i - 1)
            } else {
                user(i)
            };
            all.push_str(&line);
            // Append in two writes, sometimes cut inside a multi-byte character.
            let cut = if i % 3 == 0 {
                line.len() / 2
            } else {
                line.len()
            };
            let cut = (0..=cut)
                .rev()
                .find(|c| line.is_char_boundary(*c))
                .unwrap_or(0);
            let mut h = std::fs::OpenOptions::new()
                .append(true)
                .open(&f)
                .expect("open");
            h.write_all(&line.as_bytes()[..cut]).expect("w");
            let r = run(&f, cursor).expect("file");
            if r.reset {
                state.reset();
            }
            state.apply_chunk(r.text);
            cursor = Some(r.cursor);
            h.write_all(&line.as_bytes()[cut..]).expect("w");
        }
        let r = run(&f, cursor).expect("file");
        state.apply_chunk(r.text);
        let want = crate::transcript::parse_transcript_jsonl(all).expect("parse");
        assert_eq!(state.turns(), want);
    }

    #[test]
    fn the_command_quotes_the_path_and_refuses_bad_ones() {
        assert!(build_delta_command("../etc/passwd", None).is_err());
        assert!(build_delta_command("/home/u/a'b.jsonl", None).is_err());
        let c = build_delta_command(
            "/home/u/a b.jsonl",
            Some(FeedCursor {
                inode: 7,
                offset: 9,
            }),
        )
        .expect("ok");
        assert!(c.contains("'/home/u/a b.jsonl' 7 9"), "{c}");
    }

    #[test]
    fn the_script_itself_refuses_a_link_into_a_blocked_folder() {
        // Under the user's cache (not under /var) so that only the `.ssh` rule decides.
        let home = std::env::var("HOME").expect("HOME");
        let base = std::path::PathBuf::from(home)
            .join(format!(".cache/ssh-app-itest-guard-{}", std::process::id()));
        let secret_dir = base.join(".ssh");
        std::fs::create_dir_all(&secret_dir).expect("dir");
        std::fs::write(secret_dir.join("id"), "SECRET\n").expect("w");
        std::fs::write(base.join("ok.jsonl"), user(1)).expect("w");
        std::os::unix::fs::symlink(secret_dir.join("id"), base.join("link.jsonl")).expect("link");
        let go = |p: &std::path::Path| {
            let out = Command::new("sh")
                .args(["-c", &delta_script(), "sh"])
                .arg(p)
                .args(["-", "-"])
                .output()
                .expect("sh");
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        let linked = go(&base.join("link.jsonl"));
        assert!(linked.starts_with("NOFILE"), "{linked}");
        assert!(!linked.contains("SECRET"));
        assert!(go(&base.join("ok.jsonl")).contains("MODE full"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_hostile_answer_is_refused() {
        assert!(parse_delta_output("hello\nworld\n").is_err());
        assert!(parse_delta_output("TP_PATH=/etc/passwd\nMODE full\nCUR 1 2\n").is_err());
        assert!(parse_delta_output("TP_PATH=/home/u/a.jsonl\nMODE full\nCUR x y\n").is_err());
        assert!(parse_delta_output("NOFILE\n").expect("ok").is_none());
    }
}
