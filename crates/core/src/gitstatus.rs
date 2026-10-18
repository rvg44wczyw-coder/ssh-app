//! Git state of the files in a folder, for the file browser: which files are new, changed or
//! deleted. One `git status --porcelain=v1 -z` of the whole repository; the paths come back
//! absolute so the app can match them against its folder listing without any path arithmetic.

use crate::error::SshCoreError;
use crate::file_browser::validate_remote_dir_path;
use crate::tmux::shell_quote;

const MAX_OUTPUT: usize = 200_000;
const MAX_CHANGES: usize = 2000;
const MAX_PATH: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GitFileState {
    /// Staged as new (`git add` on a file git did not know).
    Added,
    /// A file git does not track yet.
    Untracked,
    Modified,
    Deleted,
    Renamed,
    /// Unmerged (conflict).
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitFileChange {
    /// Absolute path on the host.
    pub path: String,
    pub state: GitFileState,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitStatusResult {
    /// False when the folder is not inside a git repository (or git is missing).
    pub is_repo: bool,
    pub repo_root: Option<String>,
    pub branch: Option<String>,
    pub changes: Vec<GitFileChange>,
    /// More changes than the app shows, or the host output was cut.
    pub truncated: bool,
}

const GIT_STATUS_SCRIPT: &str = r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH
r=$(git -C "$1" rev-parse --show-toplevel 2>/dev/null)
case "$r" in ""|*[![:print:]]*) printf 'GS_STATUS=not_a_repo\n'; exit 0;; esac
printf 'GS_STATUS=ok\nGS_ROOT=%s\n' "$r"
b=$(git -C "$r" rev-parse --abbrev-ref HEAD 2>/dev/null)
case "$b" in *[![:print:]]*) b="";; esac
printf 'GS_BRANCH=%s\n' "$b"
printf 'GS_BEGIN\n'
git -C "$r" -c core.fsmonitor=false status --porcelain=v1 -z --untracked-files=normal 2>/dev/null | head -c 200001
exit 0"#;

/// Host command for the status of the repository that contains `dir` (an absolute folder). The
/// path is validated and passed as `$1`, never spliced into the script.
pub fn build_git_status_command(dir: &str) -> Result<String, SshCoreError> {
    let dir = validate_remote_dir_path(dir)?;
    Ok(crate::hostprobe::login_sh_script(
        GIT_STATUS_SCRIPT,
        &[shell_quote(&dir)],
        20,
    ))
}

fn state_of(xy: &[u8]) -> GitFileState {
    let (x, y) = (xy[0], xy[1]);
    if (x, y) == (b'?', b'?') {
        return GitFileState::Untracked;
    }
    if x == b'U' || y == b'U' || (x, y) == (b'A', b'A') || (x, y) == (b'D', b'D') {
        return GitFileState::Conflicted;
    }
    if x == b'D' || y == b'D' {
        return GitFileState::Deleted;
    }
    if x == b'R' || x == b'C' {
        return GitFileState::Renamed;
    }
    if x == b'A' || y == b'A' {
        return GitFileState::Added;
    }
    GitFileState::Modified
}

fn clean_path(root: &str, rel: &str) -> Option<String> {
    let rel = rel.trim_end_matches('/');
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.split('/').any(|c| c == ".." || c == ".")
        || rel.chars().any(char::is_control)
    {
        return None;
    }
    let full = format!("{}/{rel}", root.trim_end_matches('/'));
    (full.len() <= MAX_PATH).then_some(full)
}

pub fn parse_git_status_output(output: &str) -> GitStatusResult {
    let none = GitStatusResult {
        is_repo: false,
        repo_root: None,
        branch: None,
        changes: Vec::new(),
        truncated: false,
    };
    let Some((head, body)) = output.split_once("GS_BEGIN\n") else {
        return none;
    };
    let mut root = None;
    let mut branch = None;
    let mut ok = false;
    for line in head.lines() {
        if line == "GS_STATUS=ok" {
            ok = true;
        } else if let Some(v) = line.strip_prefix("GS_ROOT=") {
            root = (!v.is_empty() && v.starts_with('/') && !v.chars().any(char::is_control))
                .then(|| v.to_string());
        } else if let Some(v) = line.strip_prefix("GS_BRANCH=") {
            branch = (!v.is_empty() && v.len() <= 200 && !v.chars().any(char::is_control))
                .then(|| v.to_string());
        }
    }
    let Some(root) = root.filter(|_| ok) else {
        return none;
    };
    let mut truncated = body.len() > MAX_OUTPUT;
    let mut changes = Vec::new();
    let mut fields = body.split('\0');
    while let Some(rec) = fields.next() {
        // "XY path"; a rename or copy is followed by its source path as a separate field.
        let bytes = rec.as_bytes();
        if bytes.len() < 4 || bytes[2] != b' ' {
            continue;
        }
        let state = state_of(&bytes[..2]);
        if matches!(bytes[0], b'R' | b'C') {
            fields.next();
        }
        if changes.len() >= MAX_CHANGES {
            truncated = true;
            break;
        }
        if let Some(path) = rec.get(3..).and_then(|p| clean_path(&root, p)) {
            changes.push(GitFileChange { path, state });
        }
    }
    GitStatusResult {
        is_repo: true,
        repo_root: Some(root),
        branch,
        changes,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(records: &[&str]) -> String {
        format!(
            "GS_STATUS=ok\nGS_ROOT=/r\nGS_BRANCH=main\nGS_BEGIN\n{}",
            records.iter().map(|r| format!("{r}\0")).collect::<String>()
        )
    }

    fn state(r: &GitStatusResult, p: &str) -> Option<GitFileState> {
        r.changes.iter().find(|c| c.path == p).map(|c| c.state)
    }

    #[test]
    fn classifies_the_porcelain_codes() {
        let r = parse_git_status_output(&out(&[
            "?? new.txt",
            "A  staged.rs",
            " M edited.rs",
            "M  both.rs",
            " D gone.rs",
            "D  gone2.rs",
            "UU clash.rs",
            "AA clash2.rs",
            "R  moved.rs",
            "old.rs",
            "?? dir/",
        ]));
        assert!(r.is_repo);
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert_eq!(state(&r, "/r/new.txt"), Some(GitFileState::Untracked));
        assert_eq!(state(&r, "/r/staged.rs"), Some(GitFileState::Added));
        assert_eq!(state(&r, "/r/edited.rs"), Some(GitFileState::Modified));
        assert_eq!(state(&r, "/r/both.rs"), Some(GitFileState::Modified));
        assert_eq!(state(&r, "/r/gone.rs"), Some(GitFileState::Deleted));
        assert_eq!(state(&r, "/r/gone2.rs"), Some(GitFileState::Deleted));
        assert_eq!(state(&r, "/r/clash.rs"), Some(GitFileState::Conflicted));
        assert_eq!(state(&r, "/r/clash2.rs"), Some(GitFileState::Conflicted));
        assert_eq!(state(&r, "/r/moved.rs"), Some(GitFileState::Renamed));
        assert_eq!(
            state(&r, "/r/old.rs"),
            None,
            "a rename source is not its own change"
        );
        assert_eq!(state(&r, "/r/dir"), Some(GitFileState::Untracked));
    }

    #[test]
    fn refuses_what_is_not_a_clean_relative_path() {
        let r = parse_git_status_output(&out(&[
            " M ../escape",
            " M /abs",
            " M a/./b",
            " M with\nnewline",
            " M fine.txt",
        ]));
        assert_eq!(r.changes.len(), 1);
        assert_eq!(r.changes[0].path, "/r/fine.txt");
    }

    #[test]
    fn not_a_repo_and_garbage_give_an_empty_result() {
        assert!(!parse_git_status_output("GS_STATUS=not_a_repo\n").is_repo);
        assert!(!parse_git_status_output("anything").is_repo);
        let bad_root = "GS_STATUS=ok\nGS_ROOT=relative\nGS_BEGIN\n M a\0";
        assert!(!parse_git_status_output(bad_root).is_repo);
    }

    #[test]
    fn the_number_of_changes_is_capped() {
        let many: Vec<String> = (0..MAX_CHANGES + 5).map(|i| format!(" M f{i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let r = parse_git_status_output(&out(&refs));
        assert_eq!(r.changes.len(), MAX_CHANGES);
        assert!(r.truncated);
    }

    #[test]
    fn the_folder_is_validated_and_quoted() {
        assert!(build_git_status_command("").is_err());
        assert!(build_git_status_command("/home/u/.ssh").is_err());
        assert!(build_git_status_command("/ok/dir")
            .unwrap()
            .contains("/ok/dir"));
        let hostile = build_git_status_command("/a'; touch /tmp/x; '").unwrap();
        assert!(!hostile.contains("touch /tmp/x; '\n"));
    }
}
