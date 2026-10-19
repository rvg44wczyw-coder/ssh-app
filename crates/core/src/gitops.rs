//! Shared parts of the git operations that change a repository (feature 28, phases G1 and up):
//! name validation, the host-side prelude that finds the repository of a tmux pane, and the
//! result type. Every operation passes its arguments as `$2..` of a fixed script; nothing typed by
//! the user is ever part of the script text.

use crate::error::SshCoreError;
use crate::gitdiff::GitDiffStatus;
use crate::tmux::{shell_quote, validate_session_name};

const MAX_OUTPUT: usize = 20_000;
const MAX_NAME: usize = 200;

/// Result of one git operation.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitOpResult {
    pub status: GitDiffStatus,
    /// Git finished with exit code 0.
    pub ok: bool,
    pub exit_code: i32,
    /// What git printed (display-safe, capped), or the reason the app refused to run it.
    pub output: String,
}

/// A branch / tag / remote-branch name that is safe to hand to git as one argument. Stricter than
/// `git check-ref-format` on purpose (the host checks again with git itself).
pub fn validate_ref_name(name: &str) -> Result<&str, SshCoreError> {
    let n = name.trim();
    let bad_char = |c: char| {
        c.is_control()
            || c.is_whitespace()
            || matches!(
                c,
                '~' | '^'
                    | ':'
                    | '?'
                    | '*'
                    | '['
                    | '\\'
                    | '"'
                    | '\''
                    | '`'
                    | '$'
                    | ';'
                    | '&'
                    | '|'
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '{'
                    | '}'
                    | '!'
                    | '#'
            )
    };
    if n.is_empty()
        || n.len() > MAX_NAME
        || n.starts_with('-')
        || n.starts_with('/')
        || n.ends_with('/')
        || n.ends_with('.')
        || n.ends_with(".lock")
        || n.contains("..")
        || n.contains("//")
        || n.contains("@{")
        || n == "@"
        || n.chars().any(bad_char)
        || n.split('/')
            .any(|part| part.starts_with('.') || part.ends_with(".lock"))
    {
        return Err(crate::agents::invalid("Not a valid branch name"));
    }
    Ok(n)
}

/// A start point: a ref name or a commit id (7..40 hex).
pub fn validate_start_point(s: &str) -> Result<&str, SshCoreError> {
    let t = s.trim();
    if crate::gitlog::validate_commit_hash(t).is_ok() {
        return Ok(t);
    }
    validate_ref_name(t)
}

/// Common head of every operation script: finds the pane's repository (`r`), defines `g` (git in
/// that repository without hooks that start programs from config) and prints the header.
pub(crate) const PRELUDE: &str = r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH
GIT_EDITOR=true; GIT_TERMINAL_PROMPT=0; GIT_PAGER=cat; export GIT_EDITOR GIT_TERMINAL_PROMPT GIT_PAGER
p=$(tmux display-message -p -t "=$1:" '#{pane_current_path}' 2>/dev/null)
if [ -z "$p" ]; then printf 'GO_STATUS=no_session\n'; exit 0; fi
case "$p" in *[![:print:]]*) printf 'GO_STATUS=no_session\n'; exit 0;; esac
r=$(git -C "$p" rev-parse --show-toplevel 2>/dev/null)
case "$r" in ""|*[![:print:]]*) printf 'GO_STATUS=not_a_repo\n'; exit 0;; esac
printf 'GO_STATUS=ok\nGO_ROOT=%s\n' "$r"
rc=""; out=""
gp_exists() { gpp=$(g rev-parse --git-path "$1" 2>/dev/null); case "$gpp" in /*) ;; *) gpp="$r/$gpp";; esac; [ -e "$gpp" ]; }
g() { git -C "$r" -c core.fsmonitor=false -c log.showSignature=false -c gpg.program=false "$@"; }
"#;

/// Tail of a mutating script: `out` and `rc` were set by the body.
pub(crate) const EPILOGUE: &str = r#"
printf 'GO_RC=%s\nGO_BEGIN\n' "$rc"
printf '%s\n' "$out" | head -c 20000
exit 0"#;

/// The full host command: prelude + body + epilogue, run through the login shell with a watchdog.
pub(crate) fn op_command(session: &str, body: &str, args: &[&str]) -> Result<String, SshCoreError> {
    op_command_secs(session, body, args, 25)
}

/// Like `op_command` with its own watchdog (network operations and commit hooks take longer).
pub(crate) fn op_command_secs(
    session: &str,
    body: &str,
    args: &[&str],
    secs: u32,
) -> Result<String, SshCoreError> {
    validate_session_name(session)?;
    let script = format!("{PRELUDE}{body}{EPILOGUE}");
    let mut quoted = vec![shell_quote(session.trim())];
    quoted.extend(args.iter().map(|a| shell_quote(a)));
    Ok(crate::hostprobe::login_sh_script(&script, &quoted, secs))
}

fn status_of(output: &str) -> GitDiffStatus {
    match output
        .lines()
        .find_map(|l| l.strip_prefix("GO_STATUS="))
        .map(str::trim)
    {
        Some("ok") => GitDiffStatus::Ok,
        Some("not_a_repo") => GitDiffStatus::NotARepo,
        Some("no_session") => GitDiffStatus::NoSession,
        _ => GitDiffStatus::Unknown,
    }
}

pub fn parse_op_output(output: &str) -> GitOpResult {
    let status = status_of(output);
    if status != GitDiffStatus::Ok {
        return GitOpResult {
            status,
            ok: false,
            exit_code: -1,
            output: String::new(),
        };
    }
    let exit_code = output
        .lines()
        .find_map(|l| l.strip_prefix("GO_RC="))
        .and_then(|v| v.trim().parse::<i32>().ok())
        .unwrap_or(-1);
    let text = output
        .split_once("GO_BEGIN\n")
        .map(|(_, t)| t)
        .unwrap_or("");
    let mut end = text.len().min(MAX_OUTPUT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    GitOpResult {
        status,
        ok: exit_code == 0,
        exit_code,
        output: crate::approval_queue::display_safe(text[..end].trim_end()),
    }
}

pub(crate) fn parse_header(output: &str) -> (GitDiffStatus, Option<String>) {
    let root = output
        .lines()
        .find_map(|l| l.strip_prefix("GO_ROOT="))
        .map(str::to_string);
    (status_of(output), root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_names_that_could_hurt_are_refused() {
        for ok in [
            "main",
            "feature/x-1",
            "release_2.0",
            "v1.2.3",
            "origin/main",
        ] {
            assert!(validate_ref_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "-x",
            "--force",
            "a b",
            "a..b",
            "a//b",
            "/a",
            "a/",
            "a.",
            "x.lock",
            "a/.hidden",
            "a@{1}",
            "@",
            "a;b",
            "a$(x)",
            "a`b`",
            "a\nb",
            "a~1",
            "a^",
            "a:b",
            "a*",
            "a?",
            "a[b",
            "a\\b",
            "a'b",
            "a\"b",
        ] {
            assert!(validate_ref_name(bad).is_err(), "{bad:?}");
        }
        assert!(validate_ref_name(&"a".repeat(201)).is_err());
    }

    #[test]
    fn start_points_are_refs_or_hashes() {
        assert!(validate_start_point("0123abc").is_ok());
        assert!(validate_start_point("origin/main").is_ok());
        assert!(validate_start_point("--orphan").is_err());
    }

    #[test]
    fn op_output_is_parsed() {
        let ok = parse_op_output("GO_STATUS=ok\nGO_ROOT=/r\nGO_RC=0\nGO_BEGIN\nSwitched\n");
        assert!(ok.ok);
        assert_eq!(ok.output, "Switched");
        let fail = parse_op_output("GO_STATUS=ok\nGO_RC=1\nGO_BEGIN\nerror: x\n");
        assert!(!fail.ok);
        assert_eq!(fail.exit_code, 1);
        assert_eq!(
            parse_op_output("GO_STATUS=not_a_repo\n").status,
            GitDiffStatus::NotARepo
        );
        assert_eq!(parse_op_output("garbage").status, GitDiffStatus::Unknown);
    }

    #[test]
    fn a_bad_session_name_builds_nothing() {
        assert!(op_command("bad name;", "x", &[]).is_err());
        assert!(op_command("s1", "out=x; rc=0", &["any text"]).is_ok());
    }
}
