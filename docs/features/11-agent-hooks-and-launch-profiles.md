# Feature 11: Agent Approval Hooks & Auto-Start Launch Profiles

> **Note (2026-10-26):** parts of this document that describe the Unix-socket approval scheme are superseded by the file queue (`~/.ssh-app/run/{pending,decisions}`, signed by the phone key). See specs 24, 57 and 62.

- **Status**: Implemented & Verified (rev + hacker: `docs/security-findings/2026-10-16-agent-hooks-audit.md`)
- **Milestone**: M15
- **Layers**: `crates/core`, `scripts/agent-hooks`, `packages/SshCoreBridge`, `ios/`
- **Supersedes**: the hook sections of Feature 03 (`beforeShellExecution`, `claude-hook.sh`), which do not match the real agent contracts.

---

## 1. Objective

1. Install **approval/notification hooks** into the coding agents' own config files from the app, so a command that needs approval can be answered from the phone.
2. Start an agent **automatically when a tab opens** (`claude`, `agy`, ...) using named **launch profiles**, including the work/personal split the user has today:
   `alias claude-work='CLAUDE_CONFIG_DIR="$HOME/.claude" claude'`, `alias claude-personal='CLAUDE_CONFIG_DIR="$HOME/.claude-personal" claude'`.

### Why aliases cannot be used
The app starts remote commands with `$SHELL -l -c` (login, **non-interactive**): `~/.zprofile` is read (so `claude` and `agy` are on `PATH`) but `~/.zshrc`, where aliases live, is not. Profiles therefore compile to an explicit `env VAR=value binary args` command.

---

## 2. Agent hook contracts (verified against official docs, 2026-10-16)

| | Claude Code | Google Antigravity |
|---|---|---|
| Config file | `<CLAUDE_CONFIG_DIR or ~/.claude>/settings.json` (`hooks` key; merges with project/local/managed levels) | `~/.gemini/config/hooks.json` (top-level key = hook name; also `.agents/hooks.json` per project, `~/.gemini/antigravity-cli/settings.json` CLI-only) |
| Approval event | `PermissionRequest` (fires only when the agent would prompt; no allowlist needed) | `PreToolUse`, matcher `run_command` (file tools are opt-in: `PreToolUse` fires on every tool call, so matching them would send every edit to the phone) |
| Passive events | `Notification`, `Stop` | `Stop` (flat handler list: matcher-less events hold handler objects directly, no `matcher`/`hooks` wrapper) |
| stdin | JSON: `tool_name`, `tool_input`, `cwd`, `session_id`, `hook_event_name` | JSON: `toolCall.name/args`, `workspacePaths`, `conversationId` (**no event name** → passed as argv) |
| Allow | `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}` | `{"decision":"allow"}` (the documented values are `allow`, `deny`, `ask`, `force_ask`; `auto_approve` is not one) |
| Deny | `decision.behavior = "deny"` + `message` | `{"decision":"deny","reason":...}` |
| No opinion | print nothing, exit 0 → native prompt | `{"decision":"ask"}` → prompts in the terminal (respects "Always Allow"). `allow` must never be used here: per the agy docs it auto-approves. Named tradeoff: `PreToolUse` fires for every `run_command`, so the fallback prompt can appear for commands the user's policy would have allowed silently |
| Hook failure | non-blocking; exit **2** (not 1) blocks | exit-code semantics not documented → rely on JSON |
| Timeout | default 600 s | default 30 s |

Researched, **not implemented** (launch profiles only; hooks → error `unsupported`):
- **OpenAI Codex CLI** (`codex`): `~/.codex/hooks.json` or `[hooks]` in `config.toml`; `PreToolUse`, `PermissionRequest`; `ask` unsupported; hooks must be trusted via `/hooks` (hash-bound), so an installer cannot activate them silently.
- **Grok** (`grok`, binary name unverified): `~/.grok/hooks/*.json`; only `PreToolUse` blocks, **fails open**; also reads `.claude/settings.json`.
- Gemini CLI: not researched.

---

## 3. Hook script (`scripts/agent-hooks/ssh-app-hook.py`)

`ssh-app-hook <claude|antigravity> <EventName>`, installed to `~/.ssh-app/bin/ssh-app-hook` (`0700`). Replaces `claude-hook.sh`.

- Talks to the approval broker over `~/.ssh-app/run/control.sock` (override `SSH_APP_SOCKET`), one JSON line each way: `{"method":"request_approval"|"notify","agent","event","tool","command","cwd","session"}` → `{"approved":true|false}`.
- **Failure policy (defer to native policy)**: broker missing, untrusted, error, timeout (120 s) or malformed reply ⇒ no opinion ⇒ the agent applies its own prompt/policy. `auto_approve` is only ever emitted after an explicit phone approval; only an explicit phone *deny* blocks.
- **Socket trust check**: before connecting, the script `lstat`s the socket and its directory and refuses (no opinion) unless both are owned by the current user, not group/other accessible (`0600` / `0700`), and the socket is a real socket (not a symlink).
- **Trust boundary (stated tradeoff)**: the hooks, the script and the socket live in the user's account, as does the agent itself. A compromised/prompt-injected agent running as that user could edit its own hook config; the hooks are a *convenience and safety net*, not a sandbox. The phone signature check by the broker (Feature 03) protects against forged approvals from outside the account, not from inside it.
- **Tradeoff (python3 dependency)**: the script needs `python3` on the host. On a Mac without Command Line Tools `/usr/bin/python3` triggers an install dialog and the hook would time out. The long-term fix is a small native `ssh-app-ctl` that is both broker and hook; until then installation checks `python3 --version` and refuses to install if it is missing.
- No command allowlist/bypass (the old `cat*`/`echo*` patterns allowed `cat x; rm -rf ~`).
- `command` is truncated to 2000 chars.
- **Broker**: implemented as the hook-owned approval queue of `13-host-approval-broker.md` (M17): the hook queues the request in `~/.ssh-app/run/pending` and waits for a decision signed by the phone; the socket protocol described above is superseded.

---

## 4. Rust core (`crates/core/src/agents.rs`)

### 4.1 Types (UniFFI)
`AgentKind { ClaudeCode, Antigravity, Codex, Grok }`, `EnvVar { key, value }`,
`AgentLaunchProfile { id, name, agent, binary, env, args, working_dir }`,
`AgentHooksStatus { config_path, hooks_installed, script_installed }`.

### 4.2 Launch profiles
- `default_launch_profiles()` → Claude Code (`claude`), Antigravity (`agy`), Codex (`codex`), Grok (`grok`). Personal/work variants are **user data**, not presets: a Claude profile with `CLAUDE_CONFIG_DIR=~/.claude-personal`.
- `build_agent_launch_command(profile)` → `[cd <dir> && ]env K=V... <binary> <args>; exec "${SHELL:-/bin/zsh}" -l`. The trailing login shell keeps the tmux session alive when the agent exits. Values starting `~/` become `"$HOME"'/...'`; everything else is POSIX single-quoted. The result is `SessionConfig.command` (tmux initial command).
- `validate_launch_profile` rejects: empty/control-char names; binaries outside `[A-Za-z0-9._/~-]` or starting with `-`; env keys not `^[A-Za-z_][A-Za-z0-9_]*$`; `LD_*`, `DYLD_*`, `BASH_ENV`, `ENV`, `SHELLOPTS`, `PS4`, `IFS`, `PROMPT_COMMAND`; secret-looking keys (`*KEY*`, `*TOKEN*`, `*SECRET*`, `*PASSWORD*`, `*PASSWD*`, `*CREDENTIAL*`: values would appear in the host's `ps` output and in unencrypted iOS storage, so set them in the host's shell profile instead); control characters in values/args/dir.
- **Host shell environment (`load_shell_rc`)**: the app's login non-interactive shell never reads `~/.zshrc`, so PATH entries from nvm/npm/asdf or exports defined there are missing and the binary may be `command not found`. With `load_shell_rc = true` the command becomes `[cd <dir> && ]"${SHELL:-/bin/zsh}" -i -c '<env K=V binary args, single-quoted>'; exec "${SHELL:-/bin/zsh}" -l`. Quoting is unchanged inside the inner command, so hostile values stay literal. Defaults and new profiles: `true`; profiles saved before the option existed decode as `false` (no behavior change). Aliases are still not used (`env` execs the binary found on PATH), and a `.zshrc` that prints output only adds noise to the tab. Tradeoff: a heavy `.zshrc` slows start-up of every tab; it can be turned off per profile.
- **Environment check**: `build_binary_probe_command(binary, interactive)` + `parse_resolved_binary(output)`; session method `check_agent_environment(binary) -> AgentEnvCheck { login_path, interactive_path }` runs `$SHELL -l -c 'command -v <bin>'` and `$SHELL -i -c ...` (stdin `/dev/null`, stderr dropped). The probe runs under a host-side watchdog (`sh -c` wrapper kills the probe shell after 8 s, so no stuck process remains on the host) and the client gives up after 10 s. Each probe has a 10 s limit (a blocking `.zshrc` must not freeze the UI): a login-probe timeout is an error, an interactive-probe timeout sets `interactive_timed_out` (shown as a warning). Only the last line that is an absolute path counts (banners, `alias ...` lines, control characters, bidi/zero-width characters and lines over 4096 chars are ignored). The binary uses the same charset validation as profiles. Not covered: Keychain-stored credentials (an SSH session may not be able to unlock the login keychain; log in once inside the tab or use an API key set in `~/.zprofile`).
- `agent_config_dir_env(agent)` → name of the env var selecting the config dir (`CLAUDE_CONFIG_DIR` for Claude, none otherwise); `profile_config_dir(profile)` → its value in a profile (lets the UI install hooks into the right config for each profile).

### 4.3 Hook installation (pure, unit-tested)
- `agent_hooks_config_path(agent, config_dir)`, `merge_agent_hooks(agent, existing_json, hook_dir)`, `remove_agent_hooks(agent, existing_json)`, `agent_hooks_installed(agent, existing_json)`, `agent_hook_script()`.
- **Idempotent**: our entries are recognized by the `ssh-app-hook` marker in `command`, removed, then re-added; user hooks and unrelated settings are preserved (`serde_json` `preserve_order`, so key order is kept).
- **No shell injection into the agent**: the hook dir is embedded in a command the agent's shell runs, so it is restricted to letters, digits, space and `. _ - / ~`; the command uses `python3 -I` (isolated mode, ignores `PYTHON*` env and user site).
- **Never destructive**: invalid JSON, a non-object root, or a `hooks` value of the wrong type ⇒ error, file untouched.
- Claude: `PermissionRequest` (matcher `*`), `Notification`, `Stop`; handler timeout 130 s. Antigravity: key `ssh-app` with `PreToolUse` + `Stop`.

### 4.4 Session methods (`SshSessionHandle`)
- `agent_hooks_status(agent, config_dir, hook_dir)`, `install_agent_hooks(agent, config_dir, hook_dir)`, `uninstall_agent_hooks(agent, config_dir)`.
- Temp files are written with `rm -f` + noclobber (no symlink following). Install: check `python3` → resolve the config path with `realpath` (a symlinked `settings.json` from a dotfiles manager is written through, never replaced) → read → merge → backup once as `<file>.ssh-app.bak` → atomic write (tmp + `mv`) preserving the original file mode (new files `0600`) → write script `0700` in `~/.ssh-app/bin` (dir `0700`). Uninstall removes only our entries (script is left; removing it is separate).
- All remote paths are quoted with `$HOME` expansion, never interpolated raw.

## 4.5 Hook presets and in-app preview (addendum)

Goal: the user must **see in the app exactly what will be written** into the agent's config, and choose between several presets or supply their own command.

### Types (UniFFI)
- `HookPresetKind { PhoneApproval, NotifyOnly, Custom }`
- `HookPreset { kind, custom_command, custom_events, custom_matcher }` (custom fields used only by `Custom`).
- `PlannedHook { event, matcher?, command }` — one entry as it will appear in the config.

### Presets
| Preset | Claude Code | Antigravity | Uses hook script |
|---|---|---|---|
| PhoneApproval (default) | `PermissionRequest` (matcher `*`), `Notification`, `Stop` | `PreToolUse` (`run_command`), `Stop` | yes |
| NotifyOnly | `Notification`, `Stop` | `Stop` | yes |
| Custom | user-chosen events | user-chosen events | no |

`Custom` runs the user's own command (agent passes the event JSON on stdin) for the chosen events. What the command prints/exits with is interpreted by the agent under its own hook contract (section 2): for example a custom `PermissionRequest` or `PreToolUse` command that denies blocks the tool call, and one that prints nothing leaves the agent's default behaviour. Allowed events: Claude `PermissionRequest`, `PreToolUse`, `Notification`, `Stop`; Antigravity `PreToolUse`, `Stop` (`hook_events_for`). The matcher applies to tool events only (empty = agent default: `*` / `run_command`) and may contain only `[A-Za-z0-9_*|.:-]`. The command must be 1–1000 chars, single line, no control characters, and is stored with a trailing ` # ssh-app-hook-custom` shell comment: that marker is how uninstall and status recognise (and remove only) our entries. `is_our_command` is exact (script invocation `python3 -I "…/ssh-app-hook" <agent> <Event>` or the marker suffix), never a substring match, and removal works per **handler**: a foreign handler sharing an entry with ours, or one that merely mentions `ssh-app-hook` in a path, is kept. Antigravity uses the same container logic under the `ssh-app` key, so a user's own handlers there survive too. Duplicate events are collapsed. Validation (best effort, `shell_syntax_problem`) rejects commands for which the appended comment would not be a comment: trailing `\\`, unterminated quotes, unbalanced parentheses, trailing `|` / `&&`. Config files that repeat a JSON key are refused like invalid JSON (a rewrite would silently drop the earlier value).

Trust note: a custom command is typed by the app user and executed by the agent on the host with the user's own rights, exactly as if typed in the terminal; it is never executed by ssh-app itself and is written into the config as a JSON string (no shell interpolation by the app).

### Functions
- `plan_agent_hooks(agent, preset, hook_dir) -> [PlannedHook]` (pure; validates), `hook_events_for(agent)`, `hook_preset_uses_script(preset)`, `default_hook_preset()`.
- `merge_planned_hooks(agent, existing, planned)` — replaces previous ssh-app entries, keeps everything else (same guarantees as `merge_agent_hooks`, which is now `plan(default) + merge_planned`).
- `installed_hook_entries(agent, existing) -> [PlannedHook]` — the ssh-app entries currently present in a config (Claude: every `hooks.<event>` entry we own; Antigravity: the `ssh-app` key). Custom entries are recognised by the marker; entries whose command was edited by hand to drop the marker are not ours any more.
- `hooks_section_of(agent, config_json) -> String` — only the part of a config that holds hooks (Claude: the `hooks` object; Antigravity: the `ssh-app` object), pretty-printed. **The preview never shows other settings**: Claude's `settings.json` commonly carries an `env` block with API keys or an `apiKeyHelper`, which must not be rendered on screen, screenshots or logs.
- Session: `preview_agent_hooks(agent, config_dir, hook_dir, preset) -> String` is a **dry run** (reads the config, returns the hooks section the install would produce, writes nothing); `installed_agent_hooks(agent, config_dir) -> [PlannedHook]` (reads, extracts ours); `install_agent_hooks_with_preset(...)`; `install_agent_hooks` = default preset. `python3` is required only when the preset uses the script, and the script is written only then. The preview is informational: install **re-reads the config, re-plans and re-merges** (the file may have changed in between) and still refuses an unparsable config.
- Timeout: every written entry (including `Custom`) carries `timeout: 130` s, the agent's cap for the hook process; a custom command is responsible for answering quicker than that if it should not stall the agent.
- Status does not know which preset produced the installed entries; the UI therefore shows the *installed* entries (`installed_hook_entries`) next to the *planned* ones so a difference is visible.

### iOS
- The preset is part of the saved profile (kind, custom command, events, matcher); older saved profiles decode as `PhoneApproval`. Multiple presets = multiple profiles (e.g. "Claude personal: notify only", "Claude work: custom audit command").
- Profile editor section **Approval hooks**: preset picker; for `Custom`: command field, event toggles, matcher field. Live list of planned entries (event, matcher, command).
- Sheet **Hook preview** (per profile, opened from the profile row's hooks line): (1) target config path, (2) **Currently installed** entries, (3) **Will be installed** entries (event, matcher, command), (4) the resulting hooks section from `preview_agent_hooks` when connected (hooks only, see above), (5) the hook script text (`agent_hook_script`) when the preset uses it. **Install** is available only from this sheet, so nothing is written unseen; **Remove** is there too. Errors (invalid preset, unparsable config) are shown inline.
- Same agent + same config dir in two profiles with different presets: hooks live in the agent's config, not in the profile, so the last install wins. The sheet's *Currently installed* list shows what is really active; the editor does not try to arbitrate.

### Tests
Example: `plan_agent_hooks(ClaudeCode, Custom{command:"/usr/local/bin/audit", events:["PreToolUse","Stop"], matcher:"Bash"}, None)` →
`[PlannedHook{event:"PreToolUse", matcher:Some("Bash"), command:"/usr/local/bin/audit # ssh-app-hook-custom"}, PlannedHook{event:"Stop", matcher:None, command:"/usr/local/bin/audit # ssh-app-hook-custom"}]`.

Unit: preset→events matrix, custom marker/dedupe/matcher default, custom install → status → uninstall round-trip preserving other settings, validation matrix (empty, newline, no events, wrong event for agent, bad matcher, unsupported agent). `hooks_section_of` never contains foreign keys (an `env` block with a secret in the input is absent from the output); `installed_hook_entries` round-trip. Loopback sshd: preview writes nothing and returns no foreign settings; custom preset installs without the script and without python3 requirement; uninstall removes it.

---

## 5. iOS

- `AgentLaunchProfile` list persisted in `UserDefaults` (like `ServerProfile`); editor in Settings: name, agent, binary, env vars (pre-filled `CLAUDE_CONFIG_DIR` field for Claude), args, working dir.
- **New-tab flow**: choosing a profile sets `TabSession.command = build_agent_launch_command(profile)` and the tab title/agent type; a "Shell only" option remains the default.
- Editor section **Host shell environment**: toggle "Load ~/.zshrc" and a "Check on host" button (needs a live session) showing one of: found without rc / found only after loading rc (warns if the toggle is off) / not found (probably an alias; use the real binary path).
- Per-profile **Hooks** row: status (installed / not installed) and a **Review & install** button that opens the Hook preview sheet (no direct Install; see 4.5); for Claude profiles it targets that profile's config dir. A hook-install failure is shown, never silently ignored.
- Hooks for Codex/Grok are shown as "not supported yet".

---

## 5.1 Examples

Profile `{name:"Claude personal", agent:ClaudeCode, binary:"claude", env:[CLAUDE_CONFIG_DIR=~/.claude-personal]}` →
`env CLAUDE_CONFIG_DIR="$HOME"'/.claude-personal' claude; exec "${SHELL:-/bin/zsh}" -l`

`merge_agent_hooks(ClaudeCode, "{\"model\":\"opus\"}", None)` →
```json
{ "model": "opus",
  "hooks": {
    "PermissionRequest": [{ "matcher": "*", "hooks": [{ "type": "command",
      "command": "python3 -I \"$HOME/.ssh-app/bin/ssh-app-hook\" claude PermissionRequest", "timeout": 130 }] }],
    "Notification": [ ... ], "Stop": [ ... ] } }
```
Antigravity adds a top-level `"ssh-app": { "PreToolUse": [{ "matcher": "run_command", ... }], "Stop": [...] }`.

## 6. Security

- Profile values are untrusted input: strict validation + single-quote escaping; no alias or shell-snippet fields.
- Hook installation modifies agent configs that control command execution: backup, atomic write, refuse on unparsable files, remove only own entries.
- Hook script: type-guarded input, catch-all → "no opinion", overall socket deadline; no network, local socket only (`0600`, dir `0700`), bounded input/summary size, no `eval`.
- Fail-to-native-prompt ensures a broken broker degrades to the agent's built-in safety, not to auto-approve.
- Hooks can be disabled by the user's own policy (`disableAllHooks`, `allowManagedHooksOnly` for Claude); the UI status reflects only whether *our entries exist*, not whether the agent honours them.

- **Known limitation (permission modes)**: Claude only fires `PermissionRequest` when it would prompt. In `bypassPermissions` / auto-accept modes (e.g. a profile with `--dangerously-skip-permissions`) there is no prompt, so nothing is sent to the phone. The profile editor warns when `args` contain such a flag. Gating those modes would need a `PreToolUse` hook (fires on every call), which is deliberately not installed by default (approval fatigue).
- Install only locks down directories it creates (`0700`) and `~/.ssh-app`/`~/.ssh-app/run` for the default location; it never `chmod`s an existing or parent directory.

## 7. Tests

- Unit: launch command (quoting, `~` expansion, hostile values), validation matrix, merge idempotency, user-hook preservation, removal round-trip, refuse-on-invalid, unsupported agents.
- Script: live run against a fake Unix-socket broker for both agents (allow, deny, no broker, broker garbage, timeout), checking the exact JSON.
- Loopback sshd: launch-command execution (`~` expansion, hostile value stays literal in login and interactive/rc modes); `check_agent_environment` for a present and a missing binary; install → status → idempotent re-install → uninstall on a temp config dir; invalid config left untouched; backup created.

## 8. Design notes (rev)
- Hook config vs launch profile: the preset is stored with the profile for convenience, but it is applied to an (agent, config dir) pair. This is a named tradeoff, not an oversight; separating "hook sets" from profiles was rejected as extra UI for a single-user app.
- The `# ssh-app-hook-custom` marker is a shell comment because agent configs have no place for extra metadata (unknown keys may be rejected); even if a user's command leaves a quote open, the marker text is still inside the stored string, so removal works.

- Default `load_shell_rc = true` runs the user's whole `.zshrc` in every new tab (slower start, possible banner noise or prompts) in exchange for PATH/exports working out of the box. Alternative considered: capturing PATH once from the check and storing it in the profile; rejected because it goes stale when nvm/asdf versions change. The per-profile toggle and the 10 s probe limit are the mitigation.
- The hook preview shows the whole `hooks` section, including the user's own hooks; their commands may contain tokens. It is displayed only on the user's own device and never logged, which is accepted; secrets in `env`/`apiKeyHelper` are excluded.

- `shell_syntax_problem` is a small heuristic, not a shell parser: it can reject valid one-liners (a `case … in a) …;; esac` has an unmatched `)`, an apostrophe inside a trailing `# comment` looks like an unterminated quote). Accepted tradeoff: a false rejection is visible and fixable in the editor (wrap the logic in a script on the host and call that), whereas a false acceptance is a hook that silently never runs. Checking with `sh -n` on the host in the preview was considered and left as an improvement.
- Pre-M15 manual hook entries (the old README's `claude-hook.sh`) are not recognised as ours and are never touched.
- All exec calls cap their captured output at 4 MB (`MAX_EXEC_OUTPUT`) and close the channel when reached, so a runaway command or hostile rc cannot exhaust the client's memory.
- Known gap (audit item 6): install is read → merge → atomic write; a write by the agent between read and write is overwritten. A content-hash compare before `mv` is a backlog item.
- The custom-command editor warns when the text looks like it contains a credential (stored unencrypted on the phone, written to the agent config, visible in the host's `ps`).

## 9. Open items
- Host broker: done in Feature 13.
- Antigravity `run_command` argument key and exit-code semantics are not documented; verify on the installed `agy`.
- Codex/Grok hook installers.


## Addendum: Codex and Grok (M18)

| | Codex | Grok |
|---|---|---|
| Config file | `$CODEX_HOME` (profile config dir) or `~/.codex/hooks.json`, `hooks` key | `~/.grok/hooks/ssh-app.json` (the installer owns this file), `hooks` key |
| Approval event | `PermissionRequest`, matcher `*` (same reply as Claude, no output = normal prompt) | `PreToolUse`, matcher `Bash` by default (only blocking event) |
| stdin | `tool_name`, `tool_input`, `cwd`, `session_id` | `toolName`, `toolInput`, `cwd`, `sessionId` |
| Reply | Claude shape | deny: `{"decision":"deny","reason":...}`; otherwise print nothing, exit 0 |
| Passive | `Stop` | `Notification`, `Stop` |
| Trust | hooks are skipped until reviewed with `/hooks` (hash-bound) | project hooks need `/hooks-trust`; personal hooks: not documented |
| Timeout | default 600 s | default 5 s (we write 130) |

Named tradeoffs: Grok cannot be approved from the phone, only denied; neither agent has been run live (sources: developers.openai.com/codex/hooks, docs.x.ai/build/features/hooks).

## Addendum: presets, folders and several SSH keys

- A profile is a full preset: agent, binary, config dir, start directory, optional git branch
  (`git switch` runs in the start directory first; a failure is echoed in the tab and the agent
  still starts). Any number of profiles per agent (swipe → Duplicate). Long-press `+` in the tab
  bar lists them for one-tap launch.
- Start and config directories can be chosen with a directory picker over the host's file listing
  (paths under the host's home are stored as `~/...`).
- SSH keys: the original key is the "Default key"; more keys are generated or imported in
  Settings (unencrypted OpenSSH Ed25519, validated by the core) and stored in the Keychain under
  their own account. A server profile picks its key (`ServerProfile.keyId`, `nil` = default).
