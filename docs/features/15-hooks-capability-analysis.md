# Feature 15: Hooks — what we can write, and what the model can make them do

**Status:** analysis (2026-10-17); most of §5 and §1's "what we can still add" are implemented, see `19-hooks-templates-and-host-guard.md`. Based on the code in `crates/core/src/agent_hooks.rs`,
`scripts/agent-hooks/ssh-app-hook.py` and features 11, 13 and 14. Items marked *(unverified)* were
not exercised against the real agent.

A hook is a command the **agent harness** (not the model) runs at a lifecycle event. The model
never executes a hook. It does influence three things: the JSON a hook receives, the files a hook
reads, and, if allowed, the files that configure hooks. The analysis is split along those lines.

## 1. What we write to the host

| Agent | File | Container | Events we install | Command line (all identical shape) |
|---|---|---|---|---|
| Claude Code | `<CLAUDE_CONFIG_DIR or ~/.claude>/settings.json` | `hooks` (grouped `{matcher, hooks[]}`) | `PermissionRequest`, `Notification`, `Stop` | `python3 -I "$HOME/.ssh-app/bin/ssh-app-hook" claude <Event>` |
| Antigravity | `~/.gemini/config/hooks.json` | key `ssh-app`; `PreToolUse` grouped with matcher `run_command`, others flat | `PreToolUse`, `Stop` | same, `antigravity` |
| Codex | `~/.codex/hooks.json` | hooks file; hash-based trust via `/hooks` *(unverified)* | `PermissionRequest`, `Stop` | same, `codex` |
| Grok | `~/.grok/hooks/ssh-app.json` | one file | `PreToolUse`, `Notification`, `Stop` | same, `grok` |

Every handler is `{"type":"command","command":…,"timeout":130}`. Rules we keep:

- Only the fields we own are touched; other settings and other people's hooks survive a merge
  (`merge_planned_hooks`, `strip_container`, tested with foreign hooks in the file).
- The script is copied to `~/.ssh-app/bin/` (0700 folders, 0600 files); the command uses `python3 -I`
  (isolated mode: no `PYTHONPATH`, no user site, no script-directory imports).
- Writes are atomic, back up once (`*.ssh-app.bak`) and refuse if the file changed since it was read
  (lost-update protection, `CONFIG_CHANGED`).
- Presets: **phone approval**, **notify only**, **custom command**. The custom command is the only
  free-form field and is what the user typed; the app warns when it looks like it contains a
  credential, because it is stored unencrypted on the phone, written into the agent config and
  visible in the host's process list.

**What we can still add (not done):**

| Idea | Agent support | Value | Risk |
|---|---|---|---|
| `PreToolUse` with `permissionDecision: "deny"` for a rule set evaluated on the host (no phone needed) | Claude, Codex, Antigravity, Grok | Hard guard that works offline | A rule bug blocks work; needs a clear reason string |
| `UserPromptSubmit` that appends context (branch, dirty state) | Claude | Saves a round trip | Context is prompt-injection surface (§3) |
| `SessionStart` / `SessionEnd` to register the tmux session for the sessions menu | Claude | Exact agent-to-session mapping instead of process-tree guessing | One more thing that can fail at start |
| `PostToolUse` with the file diff size | Claude | "what did it just change" notifications | Noisy |
| `SubagentStop`, `PreCompact` notifications | Claude | Awareness | Noisy |

## 2. What a hook receives and can answer

**Receives** (stdin JSON): tool name, tool input (for Bash the full command, for edits the path and
content), `cwd`, session id, transcript path. Our script keeps at most 2000 characters of the
command and 512 of other fields, strips control characters and bidi/zero-width characters before
anything is shown on the phone.

**Can answer** (stdout JSON), by agent:

| Agent | Allow | Deny | Hand back to the user | Notes |
|---|---|---|---|---|
| Claude | `hookSpecificOutput.decision.behavior = allow` | `deny` + message | no output | `PermissionRequest` only fires when Claude would have asked; `allow` there skips the prompt |
| Codex | same as Claude | same | no output | trust is per hook hash; editing the hook invalidates it |
| Antigravity | `decision: allow` (auto-approves) | `deny` | `ask` | our fallback is `ask`, never `allow` |
| Grok | cannot allow | `{"decision":"deny"}` | continues into Grok's own flow | fail-open on timeout |

`PreToolUse` decisions can, per the Claude Code hook docs *(not exercised here)*, also **rewrite the tool input** (`updatedInput`) and inject
`additionalContext`. We use neither. Both are powerful: a hook that rewrites a command is running
code the user never saw.

**Our failure policy** (tested): no queue, no `ssh-keygen`, no valid answer in time, or any error →
the hook has *no opinion*. It never force-approves; `allow` is printed only after a decision signed
by the phone's Ed25519 key verified against `~/.ssh-app/allowed_signers` with the hook's own copy of
the request.

## 3. What the model can do with this

The harness runs hooks with the user's rights, so anything a hook command can do, the *hook author*
chose. The model's levers are indirect:

1. **Controlling the text shown for approval.** The command, path and tool name come from the
   model. Mitigations present: length caps, control/bidi stripping, the request digest is computed on
   the host from its own copy (a forged phone answer for a different request is rejected). Remaining:
   a long command can hide its dangerous tail below the visible part. *Action:* show the tail and the
   total length, and flag multi-line / `;`/`&&`/`|` chains.
2. **Editing the hook configuration.** If the agent may write `settings.json` /
   `hooks.json`, it can remove our hooks (silently disabling phone approval) or add its own. The
   protect-secrets template denies `~/.ssh/**`, `.env*`, `*.pem`, `*.key` but **not** the agent config
   files. *Action:* add a template "protect agent config": deny `Edit`/`Write` on the config
   directory's `settings*.json`, `~/.ssh-app/**`, `~/.gemini/config/hooks.json`, `~/.codex/hooks.json`,
   `~/.grok/hooks/**`; also surface in the app when the installed hooks no longer match
   (the status check already compares; make it a visible warning, not only a row).
3. **Writing into the queue.** `~/.ssh-app/run` is 0700 and decisions need a valid signature, so the
   model running as the same user can create *requests* (spam) but not valid *decisions*. It can fill
   the pending directory: capped at 100 files, stale entries swept. It could also read the pending
   files (same user): they contain only the command summary the agent already knows.
4. **Hook output as an injection channel.** Anything a hook prints on stdout with
   `additionalContext` / `systemMessage` enters the model's context. We print only decisions.
   A *custom command* preset that echoes file content would let repository text steer the model.
5. **Slow hooks.** Timeout 130 s on our side; the agent applies its own default (Grok 5 s default,
   which is why we write 130). A model that triggers many tool calls can stack many waiting
   approvals; the app queues them and shows "N waiting".
6. **Environment.** Hooks inherit the agent's environment (`CLAUDE_CONFIG_DIR`, API keys). The script
   does not log or transmit it. `python3 -I` blocks the `PYTHONPATH` / sitecustomize route.

## 4. What the model can execute *through* a hook, concretely

| Path | Possible? | Why |
|---|---|---|
| Run arbitrary code via our hook command | No | The command string is fixed (`python3 -I …/ssh-app-hook <agent> <Event>`); arguments come from the config, not from model output |
| Run code via the **custom** hook command | Only what the user typed | Model output reaches it as stdin JSON; a command that `eval`s or shell-interpolates that JSON would be injectable. The preview shows the exact line; the app should warn on `eval`, `sh -c`, `$(`, backticks |
| Add or change a hook | Yes, if it can write the config | See §3.2; this is the real hole |
| Approve its own request | No | Decisions need the phone's signature |
| Delay or deny work | Yes (noise) | Bounded by caps and timeouts |

## 5. Proposed work, in order

1. **"Protect agent config" permission template** *(template done 2026-10-17: `protect-agent-config`, also in `project-work`, with a suggestion `agent-config`; limit: a shell command can still write those files, only `Edit` is denied)* and a visible warning when installed hooks differ
   from the plan *(warning still to do)* (closes §3.2). Small, no new surface.
2. **Command preview hardening** on the approval card: full length, tail visible, chained-command
   flag (closes the readable part of §3.1).
3. **Custom-command lint**: warn on `eval`, `sh -c`, `bash -c`, `$(`, backticks, unquoted `$…`.
4. **Host-side rule hook** (`PreToolUse` deny rules, works without the phone), reusing the
   permissions rule syntax. Needs a design note first because a wrong rule blocks the agent.
5. **`SessionStart` registration** to map tmux sessions to agents exactly (feeds the sessions menu
   introduced with the usage report).
6. Verify Codex and Grok hooks and the `grok` binary name against the real tools; until then the
   Codex/Grok rows above are the documented contract, not observed behaviour.

## 6. Goal: use everything hooks can do

Decision (2026-10-17): the app should be able to do everything the agents' hooks allow, behind explicit user switches and with the safeguards of §3 and §4. Order of work: the §5 list, then the table in §1 ("What we can still add"), then `updatedInput` / `additionalContext` only as visible, user-confirmed features (never silent). Each step follows the project chain (`doc-dev` → `rev` → code → `rev` → `hacker`), and anything that changes what the agent runs or sees goes through `hacker` before it ships.
