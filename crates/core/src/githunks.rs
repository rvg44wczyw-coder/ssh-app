//! Staging single hunks of a file (feature 28, G5b), like `git add -p` / `git restore -p --staged`
//! without a terminal. The app reads the file's diff, shows the hunks and sends back the ones the
//! user picked; the core rebuilds a patch from ITS OWN header for the validated path plus those hunks
//! and the host applies it to the index only (`git apply --cached`). The working tree is never
//! touched, a stale or wrong patch is refused by git, and hunks with control or bidi characters can
//! only be staged as a whole file (their text cannot be shown faithfully).

use crate::error::SshCoreError;
use crate::gitdiff::GitDiffStatus;
use crate::gitops::{op_command_secs, parse_header, parse_op_output, GitOpResult, PRELUDE};
use crate::tmux::{shell_quote, validate_session_name};
use regex::Regex;
use std::sync::OnceLock;

const MAX_PATH: usize = 4096;
const MAX_DIFF: usize = 300_000;
const MAX_HUNKS: usize = 200;
const MAX_PATCH: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitHunk {
    /// `@@ -a,b +c,d @@ context`
    pub header: String,
    /// Body lines with their `+`, `-`, ` ` or `\` prefix.
    pub lines: Vec<String>,
    pub added: u32,
    pub removed: u32,
    /// False when a line has control or bidi characters (stage the whole file instead).
    pub stageable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitFileHunks {
    pub status: GitDiffStatus,
    pub hunks: Vec<GitHunk>,
    pub binary: bool,
    pub truncated: bool,
}

fn header_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^@@ -\d+(,\d+)? \+\d+(,\d+)? @@").expect("valid regex"))
}

fn odd_char(c: char) -> bool {
    (c.is_control() && c != '\t')
        || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// `git apply` reads a removed line `-- x` directly followed by an added line `++ y` as the start of
/// another file's patch. Such a hunk (a file whose own text looks like a diff header) is only
/// stageable as a whole file.
fn looks_like_file_header(lines: &[String]) -> bool {
    lines
        .windows(2)
        .any(|w| w[0].starts_with("--- ") && w[1].starts_with("+++ "))
}

fn validate_path(path: &str) -> Result<&str, SshCoreError> {
    let t = path;
    if t.is_empty()
        || t.len() > MAX_PATH
        || t.starts_with('-')
        || t.starts_with('/')
        || t.split('/').any(|c| c == "..")
        || t.chars().any(|c| c.is_control() || matches!(c, '"' | '\\'))
    {
        return Err(crate::agents::invalid("Not a valid repository path"));
    }
    Ok(t)
}

// $2 = staged|worktree, $3 = path.
const HUNKS_BODY: &str = r#"GIT_LITERAL_PATHSPECS=1; export GIT_LITERAL_PATHSPECS
if [ "$2" = staged ]; then out=$(g diff --cached --no-color --no-ext-diff --no-textconv -U3 -- "$3" 2>&1 | head -c 300000)
else out=$(g diff --no-color --no-ext-diff --no-textconv -U3 -- "$3" 2>&1 | head -c 300000); fi
rc=0"#;

/// The diff is long, so this does not use the shared epilogue that cuts output at 20 000 bytes.
const BIG_EPILOGUE: &str = r#"
printf 'GO_RC=%s\nGO_BEGIN\n' "$rc"
printf '%s\n' "$out"
exit 0"#;

pub fn build_git_hunks_command(
    session: &str,
    path: &str,
    staged: bool,
) -> Result<String, SshCoreError> {
    validate_session_name(session.trim())?;
    let path = validate_path(path)?;
    let script = format!("{PRELUDE}{HUNKS_BODY}{BIG_EPILOGUE}");
    let args = [
        shell_quote(session.trim()),
        shell_quote(if staged { "staged" } else { "worktree" }),
        shell_quote(path),
    ];
    Ok(crate::hostprobe::login_sh_script(&script, &args, 25))
}

pub fn parse_git_hunks_output(output: &str) -> GitFileHunks {
    let (status, _) = parse_header(output);
    let empty = |status| GitFileHunks {
        status,
        hunks: Vec::new(),
        binary: false,
        truncated: false,
    };
    if status != GitDiffStatus::Ok {
        return empty(status);
    }
    let text = output
        .split_once("GO_BEGIN\n")
        .map(|(_, t)| t)
        .unwrap_or("");
    let binary = text.lines().any(|l| l.starts_with("Binary files "));
    let mut hunks: Vec<GitHunk> = Vec::new();
    let mut truncated = text.len() >= MAX_DIFF;
    for line in text.lines() {
        if header_re().is_match(line) {
            if hunks.len() >= MAX_HUNKS {
                truncated = true;
                break;
            }
            hunks.push(GitHunk {
                header: crate::approval_queue::display_safe(line),
                lines: Vec::new(),
                added: 0,
                removed: 0,
                stageable: !line.chars().any(odd_char),
            });
            continue;
        }
        let Some(h) = hunks.last_mut() else { continue };
        match line.chars().next() {
            Some('+') => h.added += 1,
            Some('-') => h.removed += 1,
            Some(' ') | Some('\\') => {}
            // Anything else after a hunk (a new file header) ends it.
            _ => continue,
        }
        if line.chars().any(odd_char) {
            h.stageable = false;
        }
        h.lines.push(crate::approval_queue::display_safe(line));
    }
    for h in hunks.iter_mut() {
        if looks_like_file_header(&h.lines) {
            h.stageable = false;
        }
    }
    // A cut diff can end in the middle of a hunk: that hunk is not complete.
    if truncated {
        hunks.pop();
    }
    GitFileHunks {
        status,
        hunks,
        binary,
        truncated,
    }
}

// $2 = stage|unstage, $3 = the patch.
const APPLY_BODY: &str = r#"GIT_LITERAL_PATHSPECS=1; export GIT_LITERAL_PATHSPECS
if [ "$2" = unstage ]; then out=$(printf '%s' "$3" | g apply --cached --recount --whitespace=nowarn --reverse - 2>&1); rc=$?
else out=$(printf '%s' "$3" | g apply --cached --recount --whitespace=nowarn - 2>&1); rc=$?; fi"#;

fn build_patch(path: &str, hunks: &[GitHunk]) -> Result<String, SshCoreError> {
    let bad = |why: &str| crate::agents::invalid(why);
    if hunks.is_empty() || hunks.len() > MAX_HUNKS {
        return Err(bad("Pick between 1 and 200 hunks"));
    }
    let mut patch = format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
    for h in hunks {
        if !h.stageable
            || looks_like_file_header(&h.lines)
            || !header_re().is_match(&h.header)
            || h.header.chars().any(odd_char)
            || h.lines.is_empty()
        {
            return Err(bad("This hunk cannot be staged on its own"));
        }
        patch.push_str(&h.header);
        patch.push('\n');
        for l in &h.lines {
            if !matches!(l.chars().next(), Some('+' | '-' | ' ' | '\\')) || l.chars().any(odd_char)
            {
                return Err(bad("This hunk cannot be staged on its own"));
            }
            patch.push_str(l);
            patch.push('\n');
        }
    }
    if patch.len() > MAX_PATCH {
        return Err(bad("The selected hunks are too large"));
    }
    Ok(patch)
}

pub fn build_git_apply_hunks_command(
    session: &str,
    path: &str,
    hunks: &[GitHunk],
    unstage: bool,
) -> Result<String, SshCoreError> {
    let path = validate_path(path)?;
    let patch = build_patch(path, hunks)?;
    op_command_secs(
        session,
        APPLY_BODY,
        &[if unstage { "unstage" } else { "stage" }, &patch],
        30,
    )
}

pub fn parse_git_apply_output(output: &str) -> GitOpResult {
    parse_op_output(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "GO_STATUS=ok\nGO_ROOT=/r\nGO_RC=0\nGO_BEGIN\ndiff --git a/a.txt b/a.txt\nindex 1..2 100644\n--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n@@ -10,2 +10,3 @@ fn x()\n ten\n+eleven\n twelve\n\\ No newline at end of file\n";

    #[test]
    fn hunks_are_split_and_counted() {
        let r = parse_git_hunks_output(DIFF);
        assert_eq!(r.status, GitDiffStatus::Ok);
        assert_eq!(r.hunks.len(), 2);
        assert_eq!((r.hunks[0].added, r.hunks[0].removed), (1, 1));
        assert_eq!((r.hunks[1].added, r.hunks[1].removed), (1, 0));
        assert_eq!(r.hunks[1].lines.len(), 4);
        assert!(r.hunks.iter().all(|h| h.stageable));
        assert!(!r.binary && !r.truncated);
        let bin = parse_git_hunks_output(
            "GO_STATUS=ok\nGO_ROOT=/r\nGO_RC=0\nGO_BEGIN\nBinary files a/x and b/x differ\n",
        );
        assert!(bin.binary && bin.hunks.is_empty());
        assert_eq!(
            parse_git_hunks_output("GO_STATUS=not_a_repo\n").status,
            GitDiffStatus::NotARepo
        );
    }

    #[test]
    fn odd_characters_make_a_hunk_unstageable() {
        let d = DIFF.replace("+TWO", "+T\u{202e}WO");
        let r = parse_git_hunks_output(&d);
        assert!(!r.hunks[0].stageable);
        assert!(r.hunks[1].stageable);
        assert!(build_patch("a.txt", &r.hunks[..1]).is_err());
    }

    #[test]
    fn a_hunk_that_imitates_a_file_header_is_not_stageable() {
        let d = "GO_STATUS=ok\nGO_ROOT=/r\nGO_RC=0\nGO_BEGIN\n@@ -1,2 +1,2 @@\n--- a/other\n+++ b/other\n ctx\n";
        let r = parse_git_hunks_output(d);
        assert!(!r.hunks[0].stageable);
        let mut h = r.hunks[0].clone();
        h.stageable = true;
        assert!(build_patch("a.txt", &[h]).is_err());
        // A lone removed "-- comment" line is fine.
        let d = "GO_STATUS=ok\nGO_ROOT=/r\nGO_RC=0\nGO_BEGIN\n@@ -1,2 +1,2 @@\n--- comment\n+-- other\n ctx\n";
        assert!(parse_git_hunks_output(d).hunks[0].stageable);
    }

    #[test]
    fn patch_uses_our_header_and_refuses_hostile_input() {
        let r = parse_git_hunks_output(DIFF);
        let p = build_patch("src/a b.txt", &r.hunks[1..]).unwrap();
        assert!(p.starts_with("diff --git a/src/a b.txt b/src/a b.txt\n--- a/src/a b.txt\n+++ b/src/a b.txt\n@@ -10,2 +10,3 @@"));
        assert!(p.ends_with("\\ No newline at end of file\n"));
        let mut h = r.hunks[0].clone();
        h.lines.push("diff --git a/.ssh/x b/.ssh/x".into());
        assert!(
            build_patch("a.txt", &[h]).is_err(),
            "a line without a diff prefix"
        );
        let mut h = r.hunks[0].clone();
        h.header = "@@ -1 +1 @@\n--- a/../evil".into();
        assert!(build_patch("a.txt", &[h]).is_err(), "newline in the header");
        let mut h = r.hunks[0].clone();
        h.header = "not a header".into();
        assert!(build_patch("a.txt", &[h]).is_err());
        assert!(build_patch("a.txt", &[]).is_err());
        for bad in ["", "-x", "/etc/x", "../x", "a/../b", "a\"b", "a\\b", "a\nb"] {
            assert!(build_git_hunks_command("s", bad, false).is_err(), "{bad:?}");
            assert!(
                build_git_apply_hunks_command("s", bad, &r.hunks, false).is_err(),
                "{bad:?}"
            );
        }
        let cmd = build_git_apply_hunks_command("s1", "a.txt", &r.hunks, true).unwrap();
        assert!(cmd.contains("unstage"));
        assert!(!cmd.contains("--- a/../"));
    }
}
