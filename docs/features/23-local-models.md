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

### 2.1 What gets launched

| Preset | Command run on the host (inside the session's tmux, login/interactive shell as for other profiles) |
| :--- | :--- |
| OpenCode | `ollama launch opencode --model <model>` |
| Aider | `env OLLAMA_API_BASE=http://127.0.0.1:11434 aider --model ollama_chat/<model>` |
| Ollama chat | `ollama run <model>` |

`ollama launch <integration> --model <m>` is Ollama's own supported way to point an agent at a local
model (`ollama launch --help` lists `opencode`, `claude`, `codex`, …). **Side effect to state in
the UI**: `ollama launch` writes the integration's config on the host (that is what it is for).
Nothing is written by the app itself.

Not verified live: the three commands above were checked for syntax against `ollama launch --help`,
`aider --help` and the installed `opencode` / `aider` binaries, not run as interactive sessions.

### 2.2 Types

- Core: `AgentKind::LocalModel` ("agent that works with a local model"). It has **no hooks**: no
  phone approvals, no permission editor, no host guard (OpenCode and Aider have no compatible hook
  protocol; this is shown in the profile editor, not silently ignored). Every `match` on
  `AgentKind` that lists hook behaviour returns "unsupported" for it.
- `build_local_agent_profile(tool, model) -> AgentLaunchProfile` (UniFFI export). `tool` is one of
  `opencode`, `aider`, `ollama`; anything else is `InvalidInput`. `model` must pass the existing
  `is_model_name` rule (1-128 chars, `[A-Za-z0-9._:/-]`) **and must not start with `-`** (the rule
  alone allows it, and `ollama run <model>` takes the model as a positional argument, so a leading
  dash would be read as an option). It is also passed through `shell_quote` like every other
  argument.
- `SavedAgentProfile` gains `localTool: String` and `localModel: String` (default empty;
  decoded with `decodeIfPresent`, so saved profiles keep loading). For `LocalModel` profiles the
  stored `binary`/`args`/`env` are regenerated from these two fields on save and on launch, so a
  model rename is one edit.

### 2.3 UI

- Profile editor: for a `LocalModel` profile the "Agent" section shows **Tool** (OpenCode / Aider /
  Ollama chat) and **Model** with a *Choose from host* menu (`checkHostOllama`, same quick probe as
  the assistant). The hook, permission and approval sections are replaced by one line: "This agent
  has no approval hooks; commands run without the phone".
- The default profile list gets three presets (one per tool) with an empty model; launching one
  with an empty model asks for a model first (menu of the host's models) and remembers it.
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
  hook APIs return unsupported for `LocalModel` (no panic, typed error).
- Swift: Codable round-trip of an old profile without the new keys.

## 3. Stage 2 — the assistant as a card

### 3.1 Behaviour

- The sessions grid shows one **AI card per server** ("AI · <server>", wand icon, last answer or
  "Ask the host model" as preview). Tap opens the assistant screen. The toolbar wand button stays.
- Chat and command history live in an `AssistantSession` (per server id, in memory plus the
  existing chat persistence if any), not in `TerminalSessionViewModel`. Closing the screen keeps the
  history; a card menu has *Clear*.
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
- Local agents run **without** the approval hooks. The editor and the card text state this; the
  host guard cannot protect them. A user who wants gated execution keeps using Claude/Codex.
- The model list comes from the host and is validated (`is_model_name`) before it is shown or used.

## 5. Open questions for the owner

1. Is "no approvals for local agents" acceptable, or should they only be offered inside a host
   guard / sandbox directory?
2. Which tools beyond OpenCode and Aider (Goose, Crush, Qwen Code, Cline via `ollama launch`)?
   The design makes each a one-line preset.
