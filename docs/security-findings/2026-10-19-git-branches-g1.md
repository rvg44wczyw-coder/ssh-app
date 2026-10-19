# Git branches (feature 28, G1) — security pass

Scope: `gitops.rs` (validators, prelude), `gitbranch.rs` (scripts, parser), `GitBranchesSheetView`.
Live: the loopback test `git_branch_operations_run_in_a_temporary_repo_on_loopback_sshd` ran the
real scripts against a temporary repository through the test sshd (private tmux, private HOME);
real tmux pid and HOME unchanged afterwards.

1. **Option / shell injection through names (InputValidation)** — branch and start-point names go
   through `validate_ref_name` (no leading `-`, no whitespace, control or shell metacharacters,
   no `..`, `@{`, `.lock`) and are passed as `$3`/`$4` of a fixed script; the operation is a fixed
   keyword chosen by a Rust enum. Host side repeats `git check-ref-format --branch`. Tested with
   `-f`, `a;b`, `x y`, `a..b` (refused before any host call). No finding.
2. **Data loss (Medium if unmitigated)** — branch delete saves the tip under
   `refs/ssh-app/backup/<ts>-<name>` first; a refused delete removes that backup again; `-D` only
   after a second confirmation. Deleting the current branch refused (script and git). Verified live.
3. **Hostile repository config (SandboxEscape)** — every call runs git with `core.fsmonitor=false`,
   `log.showSignature=false`, `gpg.program=false`; `GIT_EDITOR=true`, `GIT_TERMINAL_PROMPT=0`.
   Repository hooks (post-checkout) are the user's own and do run on `switch`, as in the terminal;
   stated in the spec, accepted.
4. **Output (MetadataLeak/XSS-like)** — names and subjects pass `display_safe` and length caps;
   remote refs failing validation are dropped from the list.
5. **Resource use (DoS)** — 300 branches, 100 KB list output, 20 KB op output, 20–25 s watchdog.

Verdict: clean. Unverified on device: the sheet itself.
