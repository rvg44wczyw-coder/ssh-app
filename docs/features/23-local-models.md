# Feature 23: Local models — launch OpenCode / Aider on the host's Ollama, and the assistant as its own card

Status: spec (2026-10-18). Owner request: "open the assistant as a separate session" and
"launch OpenCode, or otherwise work with local models".

## 1. Goal

The host (the Mac) runs Ollama. Today the phone can (a) ask the host model for a shell command or
chat (AI assistant, M9/M19) and (b) start Claude Code / Antigravity / Codex / Grok. It cannot start
an agent that works *with the local model*, and the assistant is a screen reachable only from a
button, not a place in the list of sessions.

Two stages, shipped separately:

1. **Local agents**: launch presets for OpenCode, Aider and plain `ollama run` bound to a model
   chosen from the host's list.
2. **Assistant card**: a card per server in the sessions grid that opens the assistant; its chat
   history belongs to the card, not to a terminal session.

## 2. Stage 1 — local agents

### 2.1 Tools, how they are launched and what approvals they can have

Owner decisions (2026-10-18): local agents **must have phone approvals where the tool allows it**,
and **all** tools are added. Hook support was researched per tool (official docs; none of the
agents below can be driven interactively in this environment, so hook behaviour is documented, not
observed):

| Tool | Launch on the host | Approvals mechanism | Phone approval |
| :--- | :--- | :--- | :--- |
| OpenCode | `ollama launch opencode --model M` | JS plugin `~/.config/opencode/plugins/ssh-app.js`; `tool.execute.before` calls the host script and **throws** to block | yes |
| Qwen Code | `ollama launch qwen --model M` | Claude-style hooks in `~/.qwen/settings.json` (`PermissionRequest`, `Notification`, `Stop`); same reply shape as Claude's | yes |
| Crush | `crush` (model comes from Crush's own config; it has no model flag) | `hooks.PreToolUse` in `crush.json`; exit 0 + `{"decision":"allow"}` / exit 2 + stderr to block; `timeout` raised to 120 s (default is 30 s) | yes (tool call only; no notify/stop events) |
| Cline | `ollama launch cline --model M` | SDK plugins only; plugin hooks have a hard 3 s timeout and fail open | **no** |
| Aider | `env OLLAMA_API_BASE=http://127.0.0.1:11434 aider --model ollama_chat/M` | none (asks y/n in its own terminal) | no |
| Goose | `env GOOSE_PROVIDER=ollama GOOSE_MODEL=M goose session` | none found (own permission modes) | no |
| Ollama chat | `ollama run M` | none (a chat, runs no tools) | n/a |

Tools without approvals show a line in the editor ("This tool has no approval hook; it asks in
its own terminal") and the first-launch confirmation of section 2.3. Unverified details are
marked in the code and the preview, as for the Codex and Grok installers: the user path of Qwen's
`settings.json` and of `crush.json` (`~/.config/crush/crush.json`) are not stated in the docs
fetched; the installer shows the target file in the preview before writing, like every installer.

`ollama launch <integration> --model M` is Ollama's own supported way to point an agent at a
local model. **Side effect to state in the UI**: it writes the integration's config on the host.
Nothing is written by the app itself except the hook files above, through the same guarded
config writer as the other agents (hash check, backup, preview).

### 2.2 Types

- Core: new `AgentKind` variants `OpenCode`, `QwenCode`, `Crush`, `Cline`, `Aider`, `Goose`,
  `OllamaChat` (one per tool, because the hook mechanisms differ). The first three have hook
  installers; the other four return a typed "no hooks for this agent" error from every hook API.
  The sites (exhaustive matches today): `agent_hooks.rs` (config path, hook key, event lists,
  matcher, defaults, optional events), `host_guard.rs` (`host_guard_agent_support`),
  `permissions.rs` (merge), `agents.rs` (`agent_config_dir_env`, `permission_mode_args`). None
  panics. In Swift, `SavedAgentProfile.raw/kind` map every kind explicitly (today an unknown
  string falls through to `.grok`, which would silently give a local profile Grok's hooks), and
  `supportsHooks` / `supportsPermissions` / `approvalEventChoices` are per kind.
- `build_local_agent_profile(tool, model) -> AgentLaunchProfile` (UniFFI export). `tool` is one of
  `opencode`, `qwen`, `crush`, `cline`, `aider`, `goose`, `ollama`; anything else is `InvalidInput`. `model` must pass the existing
  `is_model_name` rule (made `pub(crate)`; it is private in `ai.rs` today; 1-128 chars, `[A-Za-z0-9._:/-]`) **and must not start with `-`** (the rule
  alone allows it, and `ollama run <model>` takes the model as a positional argument, so a leading
  dash would be read as an option). It is also passed through `shell_quote` like every other
  argument.
- `SavedAgentProfile` gains `localTool: String` and `localModel: String` (default empty;
  decoded with `decodeIfPresent`, so saved profiles keep loading). For local-tool profiles the
  stored `binary`/`args`/`env` are regenerated from these two fields on save and on launch, so a
  model rename is one edit.

### 2.3 UI

- Profile editor: for a local-tool profile the "Agent" section shows **Tool** (OpenCode / Aider /
  Ollama chat) and **Model** with a *Choose from host* menu (`checkHostOllama`, same quick probe as
  the assistant). The hook, permission and approval sections are replaced by one line: "This agent
  has no approval hooks; commands run without the phone".
- The default profile list gets three presets (one per tool) with an empty model; launching one
  with an empty model asks for a model first (menu of the host's models) and remembers it. When
  the host cannot be asked (not connected, Ollama down) the sheet offers a text field instead, with
  the same validation as `build_local_agent_profile`.
- First launch of any local-agent profile shows a one-time confirmation: "This agent runs commands
  on the host without phone approval; `ollama launch` also writes the tool's config on the host."
  The acknowledgement is stored per profile.
- Tab/AI-CLI detection: `opencode` and `aider` are already in `pane_context`'s AI program list
  (shell commands get the `!` prefix). `ollama run` is a model REPL where `!` is not a shell
  escape: a pane whose foreground program is `ollama` is classified `Unknown` and "send shell
  command" asks first instead of typing into the model chat.
- The sessions grid "New session" menu already lists profiles, so nothing else changes there.

### 2.4 Errors and edge cases

- Ollama not installed / not running / no models: the model menu shows the same states as the
  assistant ("not installed", "not running", "no models"); launching is not blocked (the host may
  start Ollama later), the tool's own error appears in the terminal.
- `opencode` / `aider` not on the host: the shell prints `command not found` in the tab (as for
  any profile); the existing binary probe can be run from the editor.
- A model name that fails validation is refused with a typed error, never passed to the shell.

### 2.5 Tests

- Rust: each tool's exact command; hostile model names (`; rm -rf ~`, `$(x)`, leading `-`, spaces,
  newline, 129 chars) rejected; the built command contains the model only inside single quotes;
  hook APIs return unsupported for the no-hook kinds (no panic, typed error).
- Swift: Codable round-trip of an old profile without the new keys.

## 3. Stage 2 — the assistant as a card

### 3.1 Behaviour

- The sessions grid shows one **AI card per server** ("AI · <server>", wand icon, last answer or
  "Ask the host model" as preview). Tap opens the assistant screen. The toolbar wand button stays.
- Chat and command-mode state (`chatMessages`, `chatError`, `isChatLoading`, `activeAiSuggestion`,
  `aiError`, `isAiLoading` and their tasks, all in `TerminalSessionViewModel` today) move into an
  `AssistantSession` per server id. There is no chat persistence today: history is **in memory
  only** and is lost when the app is killed (stated in the card menu; persisting transcripts to
  disk is out of scope because they can contain pasted secrets). Closing the screen keeps the
  history; a card menu has *Clear*.
- Requests call `handle.chatHostOllama` / `queryAiAssistant` on the borrowed handle directly. When
  the app is backgrounded the sockets close (zero-battery rule): an in-flight request ends with a
  visible error, the history stays, nothing is retried in the background.
- The card appears after the first use of the assistant for that server (not for every server up
  front), so the grid does not fill with unused cards.
- The assistant needs an SSH connection. It borrows the handle of a connected terminal session on
  that server at the time of each request; when there is none it starts one terminal on that
  server (as the button does today). A dedicated connection per assistant is **not** opened, to
  keep one connection per server and the zero-battery rule (nothing runs in the background).
- "Send commands to" (already built) stays: the target session is picked on the assistant screen.

### 3.2 Why a card and not a tab

A tab is a tmux session; the assistant has none, and adding a fake tab type would leak into
tmux naming, restore-on-launch, the session menu and the usage strip. A card is only a grid entry.

### 3.3 Tests

- Card per server appears/disappears with the server list; clear keeps other servers' history.
- History survives closing and reopening the screen and a terminal reconnect.

## 4. Security

- No new network surface: the same SSH exec channel, commands built in Rust with validated
  arguments.
- `ollama launch` changes host config by design; the UI says so before first use.
- OpenCode, Qwen Code and Crush get phone approvals through the same SSHSIG-verified queue as
  Claude (the host script is the only verifier; the plugin/hook only asks it). The JS plugin is a
  fixed text generated by the app (no user input interpolated), calls the installed script by
  absolute path with `execFile` (no shell) and fails **closed**: an error, a missing script or a
  timeout throws, so the call is blocked rather than allowed.
- Cline, Aider, Goose and Ollama chat run without the phone gate; the editor and the first-launch
  confirmation say so. The host guard cannot protect them.
- The model list comes from the host and is validated (`is_model_name`) before it is shown or used.

## 4a. Review notes (rev, 2026-10-18)

Named trade-offs, not defects:

- `ollama launch` couples the preset to Ollama's version and lets Ollama write the tool's config.
  The alternative, `opencode` with an inline provider config, avoids the side effect but is
  unverified for the installed OpenCode 2.0.30. Kept `ollama launch` as primary; the preset list is
  data, so the alternative can replace it without touching the rest.
- One `AgentKind` per tool (not a shared bucket): the hook mechanisms differ per tool, so a bucket
  would only move the per-tool branching into string checks.
- The card appears after first use of the assistant for that server (see section 3.1).

## 5. Decisions taken

1. Approvals for local agents: required where a hook exists (OpenCode, Qwen Code, Crush).
2. All tools added; Goose, Aider, Cline and Ollama chat are launch-only because no usable
   approval hook exists (Cline's 3 s fail-open plugin timeout cannot wait for a phone).

## 6. Implementation notes (stage 1, 2026-10-18)

Done: core (`AgentKind` x7, `local_agent_tools`, `build_local_agent_profile`, hook installers for
Qwen Code / Crush / OpenCode, host-script protocols), Swift (presets seeded once, editor with a
tool picker and a "Choose from host" model menu, one-time confirmation per profile, hook notes).

Deviations from the sections above:
- An empty model is allowed for every tool: the tool or Ollama's own picker chooses (`ollama
  launch opencode` with no `--model`). The "ask for a model first" step of 2.3 is therefore not
  needed; the text field plus the host menu cover it.
- Crush takes no model (no flag exists); the editor hides the field and the core refuses one.
- Custom hook commands are refused for Crush and OpenCode (their formats are not Claude's); only
  the phone approval / notify presets exist. Crush has no notify preset (one event only).
- Removing OpenCode's hooks leaves an inert plugin of ours: the config writer cannot delete files.

Verified: the generated OpenCode plugin runs under `node` against a fake script (deny blocks, allow
and no-opinion pass, missing/crashing/garbage script blocks); the host script answers Qwen, Crush
and OpenCode with the documented shapes, incl. host-guard deny (`tests/hook_script.rs`,
`tests/opencode_plugin.rs`); core unit tests for every command line and hostile model names; Swift
tests compile (`build-for-testing`), they have not been run (no simulator).

Not verified (no way to drive these TUIs here): Qwen Code's and Crush's real user config paths,
Qwen's `PermissionRequest` reply being honoured, Crush honouring exit-0 JSON with `timeout: 130`,
OpenCode awaiting an async `tool.execute.before` for up to 135 s, and `ollama launch <tool>
--model` end to end.

Stage 2 (assistant card) is not started.
