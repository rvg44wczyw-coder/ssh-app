# 19. Hooks: templates, host guard, command analysis, drift warning, session registry

Follows `15-hooks-capability-analysis.md` (what hooks can do) and `14-agent-permissions-and-hook-settings.md`.
Goal (decided 2026-10-17): use everything the agents' hooks allow, behind explicit switches.

## 1. Templates (`hook_templates.rs`, `HookTemplatesView`)
33 ready-made setups, picked in the profile editor (Approval hooks → Templates). A template is a `HookPreset` plus text; picking one only fills the profile, the usual **Review & install** preview shows what is written.

| Category | Templates |
|---|---|
| Approvals (phone, signed answer) | everything · shell only · file changes · shell + web · MCP tools · requests only (no notices) |
| Notifications | notify only · finished · needs attention · Mac banner / sound / speech on finish · sound on attention · sub-agent done · before compact |
| Guards (block on the host, no phone) | rm -r · force push · sudo · curl/wget piped into a shell · git reset --hard / clean · .env and key files · agent + hook config · lock files · .git folder |
| Logging | tool calls · prompt lengths · session start/end · register sessions for the app |
| Workflow | snapshot the working tree on stop (`refs/ssh-app/snapshot`, nothing else touched) · format Rust / Python / Go / Swift after edits |

Guards are Claude-only (`exit 2` + stderr is Claude's "block this tool call"; payload names are Claude's). Notification/Stop templates fit every agent. Every template is tested: it plans for each agent it lists, custom commands never `eval`/`sh -c`/backtick event text, loggers never write command text, and each guard is run against allowed and blocked inputs.

New Claude events usable in custom hooks: `PostToolUse`, `UserPromptSubmit`, `SessionStart`, `SessionEnd`, `SubagentStop`, `PreCompact`. The script-based presets can also install `SessionStart`, `SessionEnd`, `UserPromptSubmit` and (phone approval) `PreToolUse` as **extra** events (off until switched on; PreToolUse replies use `permissionDecision`).

## 2. Host guard (`host_guard.rs`, `~/.ssh-app/guard.json`, `HostGuardView`)
Rules enforced by the hook script itself, before anything reaches the phone, with the permission-rule syntax (`Bash(rm:*)`, `Read(~/.ssh/**)`, `WebFetch`):
- **deny** blocks, **ask** hands the decision to the user (Claude PreToolUse `ask`, Antigravity `ask`), **allow** lets it run without a prompt (Claude, Antigravity, Codex). `deny` > `ask` > `allow`.
- **Context** (Claude): text the user wrote is added with `additionalContext` at `SessionStart` and with every `UserPromptSubmit`. The prompt text is never echoed.
- **Stop message** (Claude): `{"decision":"block","reason":…}` asks the agent to do one more round; never when `stop_hook_active` is set (no loops).
- 14 presets (block destructive / secrets / agent config / shell downloads / package installs / sudo / git writes; always ask for network and push; allow read-only and the dev loop; house-rules and briefing context; run tests before stopping). Presets merge into the editor.
- Written with folder `0700`, file `0600`; read only if the file is a regular, owned, non-symlink file in a private folder. Unreadable or malformed content is an error in the app (never overwritten) and means "no guard" in the script.
- Not offered, on purpose: `updatedInput` (rewriting a tool call into something the user never saw), `systemMessage`, `continue:false`.
- Agent support: Claude all; Antigravity deny/ask/allow; Codex deny/allow; Grok deny only.

## 3. Command analysis on the approval card (`hook_safety.rs`)
The card shows the length and number of lines, orange flags (chains, several lines, command substitution, pipe into an interpreter, sudo, recursive delete, network, force push, config/hook/shell-startup files, non-ASCII look-alikes, long command) and, for a long command, **"Ends with:"** the last 120 characters so the end cannot hide below the scroll box.
The custom-command editor lints the command (`eval`, `sh -c`, backticks, `$(`, unquoted `$var`, `xargs`, pipe into a shell).

## 4. Drift warning
`AgentHooksStatus.installed` carries the ssh-app entries found on the host; `hooks_drift(planned, installed)` lists missing / changed / extra ones. The profile row shows an orange warning when the host config differs from the profile (not when nothing is installed).

## 5. Session registry
`SessionStart` / `SessionEnd` (template "Register sessions for the app") make the script note, in `~/.ssh-app/run/sessions`, which tmux session each Claude session runs in (`ctl sessions`, `list_agent_sessions`). The Host guard screen lists them. Using it to replace the process-tree guessing in the sessions menu is the next step.

## 6. Verification and limits
- Rust: 148 unit tests, 25 hook-script tests (real script, temp `SSH_APP_HOME`, `TMUX` removed); `clippy -D warnings`. Swift: tests added, built, not run (no simulator here).
- **Not verified on the real tools:** Codex and Grok are not installed on this machine; their rows stay "documented contract". Claude's `PreToolUse` `ask`/`allow`, `additionalContext`, Stop `block` follow the documented hook format but were exercised only against the script, not a live Claude session.
- A deny/ask/allow **prefix rule is not a sandbox**: see `docs/security-findings/2026-10-17-host-guard.md`.

## 7. Still open
- Use the session registry in the sessions menu.
- Verify Codex / Grok hooks on the real binaries.
- A "protect agent config" rule for shell writes (a hook can only see the command text).
- Per-profile guard files (today one guard per host).
