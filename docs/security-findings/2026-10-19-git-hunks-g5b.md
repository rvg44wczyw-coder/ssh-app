# Security findings: feature 28 G5b, staging single hunks (2026-10-19)

## 1. Scope

`crates/core/src/githunks.rs` (`parse_git_hunks_output`, `build_patch`, the two host scripts),
`host_api.rs` (`fetch_git_file_hunks`, `apply_git_hunks`), Swift `GitHunksSheetView`.

Applies: text from a repository (untrusted) that becomes a patch applied by git, path handling,
quoting of a large argument. Ruled out: crypto, replay, timing, network input.

Live tests (temporary repository on the loopback sshd harness, private HOME, private tmux socket;
tmux pid 35350 and `HOME=/Users/ivs` unchanged afterwards): stage one of two hunks of a file with a
space in its name (index changes, working tree byte-identical), stage the other, unstage one, a stale
hunk (context edited) refused by git with the index unchanged, a hunk with an extra `diff --git` line,
a path `../x`. Unit tests: bidi in a hunk, header with a newline, wrong prefixes, hostile paths
(`-x`, `/etc/x`, `a/../b`, quote, backslash, newline), a hunk imitating a file header.

## 2. Findings

### 1. [InputValidation] Low, fixed: a file whose text looks like a diff header
`git apply` reads a removed line `-- x` followed by an added line `++ y` (diff lines `--- x` / `+++ y`)
as the start of another file's patch, so a hostile file could make a staged hunk touch the index
entry of another path inside the repository (index only; paths outside the repository are refused by
git; the diff is shown to the user first). Now such a hunk is `stageable: false` in the parser and
refused again in `build_patch`; a lone `-- comment` line (SQL, Lua) is fine.

### 2. [Informational] A blank line at the end of the patch
First version fed the patch with `printf '%s\n'` and git counted the extra empty line as context
(`patch does not apply`). Fail-closed, found by the live test, fixed (`printf '%s'`).

### 3. [Informational] Scope of the write
`git apply --cached` changes the index only. The patch header (`diff --git a/P b/P`) is made by the
core from the validated path, never taken from the host's diff, so a hunk cannot carry another path,
rename or mode change: every body line must start with `+`, `-`, space or `\`. Hunks with control or
bidi characters cannot be staged on their own (their text could not be shown faithfully).

## 3. Verdict

Findings (highest severity: Low, fixed). Clean for G5b.
