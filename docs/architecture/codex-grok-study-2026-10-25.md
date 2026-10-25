# Codex and Grok: what they store and show (study for connecting them, 2026-10-25)

Live runs in a private tmux socket with the agent's folder set to the app's own directory (one harmless `sleep`, no edits), plus
the documentation Grok ships in `~/.grok/docs/user-guide`. Versions: Codex CLI 0.164.0, Grok Build 1.0.70. Account names, ids and
tokens are not recorded here (`auth.json` of both agents was never opened). Goal: the same level of connection as Claude Code and
Antigravity: feed from the session file, screen parser, settings and command screens, hooks, background tasks.

## 1. Codex

**Session files.** `~/.codex/sessions/YYYY/MM/DD/rollout-<local time>-<uuid>.jsonl`; index `~/.codex/session_index.jsonl`
(`id`, `thread_name`, `updated_at`); prompt history `history.jsonl`. Other state is SQLite (`state_5`, `thread_history_1`, `logs_2`):
not needed for the feed.

**Line format**: `{timestamp, ordinal, type, payload}`.

| `type` / `payload.type` | Meaning |
|---|---|
| `session_meta` | session id, `cwd`, workspace roots |
| `event_msg` / `task_started`, `task_complete` | turn bounds; `task_complete.last_agent_message`, `duration_ms` |
| `event_msg` / `item_completed` | `item.type`: `UserMessage`, `AgentMessage` (`phase: final_answer`), `CommandExecution` (`process_id`, `command`, `cwd`, `source`, `status`) |
| `event_msg` / `token_count` | token usage (context meter) |
| `response_item` / `message`, `custom_tool_call`, `custom_tool_call_output` | the model's side: roles `developer`/`user`/`assistant`; tool calls |
| `turn_context` | approval policy, sandbox, model |
| `world_state` | AGENTS.md text and environment (large; skip) |

Notes for a parser: tools are called through a JavaScript wrapper (`custom_tool_call` named `exec`, input
`const r = await tools.exec_command({cmd:"sleep 120",workdir:"…",yield_time_ms:1000});`), so the command text is in JS; the
structured `CommandExecution` item is the reliable source (command as `["/bin/zsh","-lc","sleep 120"]`). The `UserMessage` is also
present as a `response_item`; the first user `message` holds the AGENTS.md text, not something the person typed. A message sent while
a turn runs is to be checked (queue behaviour not captured).

**Screen.** Banner `>_ OpenAI Codex (v0.164.0)`, folder, a `›` prompt line, footer `<model> · <folder> · <task title>` and
`← for agents · ? for shortcuts`. Answers start with `•`; `Worked for 4s • 11:55` closes a turn.

**Background terminals.** Footer line `1 background terminal running · /ps to view · /stop to close`. `/ps` prints a list
"Background terminals • <command>" into the feed; `/stop` stops **all** of them ("Stopping all background terminals."): there is
no per-task stop. Start-up asks to trust changed hooks (see §3).

## 2. Grok Build

**Session files.** `~/.grok/sessions/<project path, URL-encoded>/<session uuid>/` with `summary.json`, `updates.jsonl`,
`chat_history.jsonl`, `prompt_context.json`, `system_prompt.txt`; `sessions/session_search.sqlite`; `active_sessions.json`;
`logs/unified.jsonl`; `memory-v2/`. Per-project prompt history `prompt_history.jsonl`. Formats of `updates.jsonl` and
`chat_history.jsonl` are not read yet (next step).

**Screen** (full-screen TUI, rounded boxes): header row `main ~/folder … 1.8K / 256K │ [Dashboard]` (git branch, folder, context
used/size); the conversation; a boxed prompt `│ ❯ … │` with the model in the border (`Grok 4.7 (high)`); a hint row
`Enter:send │ Opt+Enter:newline │ Shift+Tab:mode │ Ctrl+x:shortcuts`. A working turn shows a spinner line
(`⠋ Running 2 pre_tool_use hooks… 33s … [stop]`). A "Help improve Grok" opt-in strip sits under the conversation (off by default;
never opted in during these runs).

**Slash menu** (`/` opens a fuzzy list, `❯` cursor, name + description): `/usage /tutorial /settings /dashboard /workflows
/plugins /btw /voice …`; the full list is in `04-slash-commands.md`: `/new /resume /compact /context /session-info (/status)
/fork /rewind /copy /export /model /effort /context-window /always-approve /auto /multiline /theme /mcps /hooks /skills /goal
/loop /workflows`.

**Settings** (`/settings`, `F2`, `Ctrl+,`): one modal, sections with `▸` rows; value kinds: toggle (`on`/`off`), choice (`value ›`),
number. Sections: Appearance (tool call density, compact mode, default screen mode, timestamps, timeline sidebar, theme,
Mermaid, thinking blocks …), Mouse, Editor & Input (queue behaviour, vim keys, multiline, prompt suggestions, voice), Agent &
Approval (permission mode Ask/Plan/Auto-review/Always-approve, remember approvals, default permission, plan mode), Privacy, Models
(default model, subagent model inheritance), Advanced. Keys: `↑/↓/j/k`, `Space` toggle, `Enter` edit, `→` expand, `/` search, `d`
reset, `F2/Esc` close.

**Other screens seen**: `/model` is a picker list (`❯ Grok 4.7 (current) …`), `/session-info` is a boxed read-only screen (path, model,
API backend, turn, context tokens; `c` copies the id), `Ctrl+x` is a collapsible shortcuts list (groups: Input, Conversation
Navigation, Conversation Actions, Panels, Session, Dashboard).

**Background commands.** Started by the model with `run_terminal_command` and `block_until_ms: 0`; read with
`get_command_or_subagent_output`, stopped with `kill_command_or_subagent`; `Ctrl+B` sends the running foreground command to the
background; completion adds `Task started` / `Task completed` / `Task failed` / `Task killed` rows to the feed. The task list UI
was not reached in the live run (the hook below held the turn); see `20-background-tasks.md` and `/dashboard`.

## 3. Hooks (what changes in `docs/features/11-agent-hooks-and-launch-profiles.md`)

**Grok** (`10-hooks.md`): the spec line "only `PreToolUse` blocks, fails open" is **incomplete**:
- Events: `UserPromptSubmit` (can block a prompt), `PreToolUse` (can deny), `PostToolUse` (cannot block but may replace the tool
  output the model sees), `Stop` and `SubagentStop` (can block the end of a turn), plus passive events. Any hook failure
  (timeout, crash, malformed output) is **fail-open**; only an explicit deny, a reply past a read limit or an unusable
  `updatedInput` blocks.
- Hook files load from `~/.grok/hooks/*.json`, project `.claude/settings.json`-style files, `~/.claude/settings.json`,
  `~/.cursor/hooks.json`, `config.toml`, plugins. **Trust gate**: in a folder that is not trusted (`/hooks-trust`, `--trust`,
  `~/.grok/trusted_folders.toml`) the user's own hooks are held back silently. The app must check the trust state and tell the
  user instead of showing hooks as installed.
- Our installed file `~/.grok/hooks/ssh-app.json` (`PreToolUse` for `Bash`, `Notification`, `Stop`; timeout 130 s) held a live
  turn at "Running 2 pre_tool_use hooks…" while waiting for the phone's approval: it works as designed; a live check with
  hooks needs the phone, or the approval, in the loop.

**Codex**: `~/.codex/hooks.json` (our `PermissionRequest` and `Stop` entries). At start Codex shows "Hooks need review: 2 hooks are
new or changed" with three choices (review, trust all, continue without trusting: "hooks won't run"). So after the installer
writes or changes `hooks.json`, **nothing runs until the user trusts the hooks in Codex's own TUI**. The app must say so after
installing and detect an untrusted state (where Codex stores the trust is to be found; not read here).

## 4. What the connection needs, per agent

| Piece | Codex | Grok |
|---|---|---|
| Transcript source | `rollout-*.jsonl` (turns, final answers, `CommandExecution`) | `sessions/<project>/<uuid>/updates.jsonl` / `chat_history.jsonl` (format to read) |
| Screen parser | banner, `›` prompt, `•` answers, footer | boxed TUI, header row, boxed prompt, spinner line |
| Settings screens | `/permissions`, `/model`, `/status` (to capture) | `/settings` sections (captured), `/model`, `/session-info`, `Ctrl+x` |
| Commands menu | Codex's own list (to capture from `/`) | list in `04-slash-commands.md` (+ skills) |
| Background tasks | `/ps` list, `/stop` all | feed rows + dashboard (UI to capture); kill by tool |
| Hooks | `hooks.json` + trust review at start | `~/.grok/hooks/*.json` + folder trust |
| Per-task stop | none (all-or-nothing) | by task id through the agent |

Open: the Codex queue behaviour and its approval screens, Grok's task list UI, the two session formats beyond the first lines,
where each agent keeps its hook trust.
