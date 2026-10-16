# Security audit: M15 increments — interactive `.zshrc` launch, host binary check, hook presets & preview

- **Date**: 2026-10-16
- **Target**: branch `main`, uncommitted working tree on top of `b800ac6`
- **Auditor**: `hacker` (adversarial, own loopback sshd and temp dirs only)

## 1. Scope

Reviewed:
- (A) `crates/core/src/agents.rs`: `build_agent_launch_command` (`load_shell_rc` → `$SHELL -i -c '<quoted inner>'`), `build_binary_probe_command`, `parse_resolved_binary`, `validate_binary`; `session.rs::check_agent_environment` (10 s probe timeout).
- (B) `agents.rs`: `HookPreset`/`PlannedHook`, `plan_agent_hooks`, `validate_custom_hook`, `merge_planned_hooks`, `installed_hook_entries`, `hooks_section_of`, `remove_agent_hooks`; `session.rs::{preview_agent_hooks, installed_agent_hooks, install_agent_hooks_with_preset}`; iOS `HookPreviewSheetView.swift` and the profile editor (code reading).

Attack classes applied: InputValidation, SandboxEscape (command injection), MetadataLeak, DoS. Ruled out: MITM/Replay/Downgrade/KeyConfusion/Timing/WeakRandomness (no handshake, key derivation, comparison or randomness in this scope; the SSH transport itself was audited earlier).

Live tests actually run (harness in the session scratchpad, not in the repo):
1. **Interactive-mode quote fuzz**: 400 random hostile values (quotes, `$()`, backticks, `;`, `&&`, `|`, globs, history `!`, `<()`, redirections, U+2028, `'; touch CANARY; '`) injected into `args`, env values and `working_dir`, executed through a real `zsh -i -c` (empty `ZDOTDIR`) and `sh`; both `load_shell_rc` on/off. Result: **0 injections**, 0 content mismatches (canary file never created).
2. **Pure-function attacks** on config handling and validation (results below).
3. **Live loopback sshd** with `SetEnv SHELL=<fake shell>` to model a host whose interactive shell hangs or floods output; measured timeout behaviour, orphan processes and client memory.
4. All spawned processes (`sshd`, fake shells, tmux sessions) were checked and none were left running.

Code analysis only: race between preview and install (item 6), iOS views.

## 2. Findings

### 1. DoS — Medium — unbounded buffering of command output (confirmed live)
- **Location**: `session.rs` exec helpers (`execute_command_on_active_session` / `execute_remote_command_internal`: `output.extend_from_slice` with no cap), newly reachable with attacker-influenced output through the interactive probe in `check_agent_environment`.
- **Scenario**: a host whose `~/.zshrc` (or compromised shell) prints without end. Tapping "Check on host" makes the app buffer everything until the 10 s timeout.
- **Verified**: fake interactive shell running `yes`: **client RSS 9 MB → 1370 MB in 10 s**. On iOS this would hit the jetsam memory limit and kill the app. The 10 s timeout works (returned in 10.03 s with `interactive_timed_out = true`) but does not bound memory.
- **Fix direction**: cap bytes read per exec call (e.g. 64 KB for probes; larger caps for other callers), close the channel when the cap is hit.

### 2. DoS — Low — the host process is not killed when the probe times out (confirmed live)
- **Location**: `session.rs::check_agent_environment` (`tokio::time::timeout` drops the future only).
- **Scenario**: an rc that blocks (prompt, slow plugin, infinite loop). Each timed-out check leaves the remote `zsh -i` running.
- **Verified**: after the timeout the spawned `sleep 300` (hang mode) and the `yes` flood process were still alive; one orphan per tap, indefinitely for a true hang.
- **Fix direction**: wrap the probe in a host-side time limit (e.g. `perl -e 'alarm 8; exec @ARGV'` / a `sleep`+`kill` watchdog in the command) or request a PTY-less exec with `signal`/channel close that terminates the process group.

### 3. InputValidation — Low — marker `# ssh-app-hook-custom` is not always a comment (confirmed)
- **Location**: `agents.rs::plan_agent_hooks` (`format!("{} # {HOOK_MARKER}-custom", …)`).
- **Scenario**: a custom command ending in a backslash: `echo hi \` becomes `echo hi \ # ssh-app-hook-custom`; the shell treats the escaped space as part of a word, so `#` and the marker become **arguments** (verified: stdout `hi  # ssh-app-hook-custom`). Commands ending in `&&`, `|` or with an unbalanced quote (`foo 'bar`) produce a syntax error at hook time (verified exit status 2): the user believes the hook protects them, but it never runs. Removal still works (marker stays inside the stored string).
- **Fix direction**: reject trailing `\`, trailing `&&`/`||`/`|`/`;` and unbalanced quotes at validation (`sh -n -c` on the host in `preview_agent_hooks`), or attach the marker structurally, e.g. `sh -c '<command>' ssh-app-hook-custom` with the command quoted by `shell_quote`.

### 4. InputValidation — Low — uninstall/install removes foreign hooks that share an entry or substring with the marker (confirmed)
- **Location**: `agents.rs::is_ours` / `strip_claude`; `ANTIGRAVITY_HOOK_KEY`.
- **Scenario / verified**:
  - Claude: an entry `{"hooks":[ {foreign}, {…ssh-app-hook} ]}` is removed **as a whole**, deleting the user's sibling hook (`sibling_lost=true`).
  - A foreign hook whose command merely contains the substring (`/home/u/code/ssh-app-hook-tests/run.sh`) is deleted by install **and** uninstall (`foreign_marker_path_lost=true`).
  - Antigravity: a user's own top-level `"ssh-app"` key is overwritten by install and deleted by uninstall.
- Backup (`.ssh-app.bak`, once) limits the damage; the preview shows only "Will be installed", not what will be **removed**.
- **Fix direction**: remove only individual handlers that match a stricter marker (exact `…/ssh-app-hook" <agent> <Event>` pattern or the `-custom` suffix at end of string), keep siblings, and list "will be removed" entries in the preview.

### 5. InputValidation — Low — duplicate JSON keys are silently dropped on rewrite (confirmed)
- **Location**: `parse_object` / `render`. Input `{"model":"a","model":"b", "hooks":{"Stop":[…],"Stop":[]}}` is rewritten with the first duplicates gone (`first_kept=false`).
- Parsing nested depth 200 000 on a 512 KB stack returned an error (no overflow); a 5000-digit number is preserved exactly; error messages do not echo file content (no secret in messages for invalid JSON).
- **Fix direction**: detect duplicate keys and refuse, same as invalid JSON.

### 6. Race — Low — lost update between read and write of the agent config (code analysis)
- **Location**: `session.rs::install_agent_hooks_with_preset` / `write_remote_config`: read → merge → atomic `mv`. A write by the agent or the user in that window (settings.json is also written by Claude's `/config`) is overwritten. The preview is informational only (install re-reads), so a stale preview cannot install stale content; only the short read→write window matters.
- **Fix direction**: compare a content hash (or mtime+size) on the host immediately before `mv` and abort on change.

### 7. MetadataLeak — Low — secrets pasted into a custom command are stored and shown in plain text
- **Location**: `validate_custom_hook`: no secret heuristic (unlike env keys). `curl -H 'Authorization: Bearer sk-…'` is accepted (verified). The value is kept in iOS `UserDefaults`, written to the agent's `settings.json`, shown in the preview and visible in the host's `ps` when the agent runs the hook. The preview also displays the user's own foreign hooks' commands (verified `foreign hook command visible`), which can contain tokens; accepted since it is the user's own device and nothing is logged.
- **Verified secure**: `hooks_section_of` never returns `env`, `apiKeyHelper` or any key outside `hooks`/`ssh-app` (secret in `env` absent from the output, also through `preview_agent_hooks` on loopback sshd).
- **Fix direction**: warn in the editor when the command looks like it contains a credential (`Bearer`, `token=`, `sk-`), recommend reading it from the host's environment.

### 8. InputValidation — Info — display spoofing and size of probe output
- `parse_resolved_binary` accepts bidi override characters (`/usr/bin/\u{202e}gnp.exe`) and a 50 MB single line (verified). Requires a hostile host shell, which already controls everything the host says; impact is only a misleading label. Fix direction: reject Unicode format characters and cap the length (4096).

## 3. Attack results that held (no finding)
- **Quoting/injection**: interactive and non-interactive launch commands: 400/400 hostile cases literal; binary charset validation rejects `a; rm -rf ~` and `-x`.
- **Custom command abuse**: newline, CR, NUL, NEL (U+0085), ESC, length > 1000 rejected; U+2028 accepted but does not split commands in `sh` (verified as one word); matcher charset rejects spaces, `;`, `$()`, `(…)`, emoji (`Bash\n` is trimmed before use). A catastrophic-regex-like matcher (`.*.*.*.*y`) is accepted; the matcher is evaluated by the agent against short tool names, so no practical ReDoS.
- **Config corruption**: non-object/array `hooks`, deep nesting, huge numbers handled without panic; secrets absent from error text.
- **Probe timeout**: the UI is released after 10 s as designed.

## 4. Remediation (same day, re-verified)

| # | Status | Verification |
|---|---|---|
| 1 | Fixed: `MAX_EXEC_OUTPUT` = 4 MB in both exec paths | Live rerun: flood probe returns in 85 ms, client RSS 9 → 19 MB (was 1370 MB); unit test `exec_output_is_capped` |
| 2 | Fixed: `sh -c` watchdog kills the probe shell at 8 s | Live rerun: hung shell gone after the call (was alive); unit test starts a never-exiting shell and checks it was killed |
| 3 | Fixed: validation rejects trailing `\`, unterminated quote, unbalanced parens, trailing `\|`/`&&` | Re-ran attack: `foo 'bar` now refused; unit test matrix accept/reject |
| 4 | Fixed: exact command match, handler-level removal, shared container logic for Antigravity | Re-ran attack: `sibling_lost=false`, `foreign_marker_path_lost=false`, foreign `ssh-app` handlers kept; unit tests |
| 5 | Fixed: duplicate keys refused | Re-ran attack: error `duplicate key "Stop"`; unit test incl. no false positives |
| 6 | Open (backlog): read→write window | — |
| 7 | Mitigated: editor warning for credential-looking commands; preview excludes `env`/`apiKeyHelper` (verified) | UI code; core preview verified live |
| 8 | Fixed: bidi/zero-width and > 4096-char paths rejected | Re-ran attack: bidi path → `None`; unit test |

## 5. Verdict

Findings, highest severity **Medium**; all but item 6 fixed and re-verified (see section 4). Original assessment: (items 1–2 are the ones worth fixing before relying on the host check; items 3–5 are data-loss/correctness edge cases that preview/backup soften). No injection, privilege escalation or secret disclosure found in the launch command, hook planning or preview.
