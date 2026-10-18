# Feature 30: context menus on quick-command buttons (model lists from Claude and Antigravity)

Status: implemented (2026-10-18): helper `models`, core `fetch_agent_models`, long-press menus and an editor in the quick-command bar. The `/model` syntax inside a running agy session is still unverified.

## Goal
A long press on a quick-command button (for example `/model`) opens a menu of choices; tapping one
sends the command with that choice (`/model opus`). The lists should come from the agents
themselves where they can, otherwise from a list the user edits.

## What the agents offer (checked on the dev host: Claude Code 2.1.302, agy 1.4.2)
| Source | Can we read it? | How | Caveats |
|---|---|---|---|
| Antigravity | yes, officially | `agy models` prints `id<TAB>label` lines (13 models: Gemini 4 Argon / 3.8 Flash, Claude Opus/Sonnet 5.5, GPT-OSS 120B, each with Low/Medium/High variants) | needs network and agy's own login; takes a few seconds, so cache it in the agent for hours. We run the CLI, we never touch its token. `agy --model <id>` takes the same ids. How `/model` inside a running agy session takes an argument is unverified |
| Claude Code | partly | `claude --model` accepts the aliases `fable`, `opus`, `sonnet` (and full ids); inside a session `/model <alias>` | there is no `claude models` command. `~/.claude/cache/model-catalog/*.json` holds the signed catalog the CLI itself uses (ids, display names, effort levels) but its format is internal and versioned; read it only as an optional bonus with a built-in fallback list |
| Ollama | yes | `ollama list` on the host | already used for the local-model profiles |

## Design
- Helper subcommand `ssh-app-host models <agent>` (agent: `claude`, `antigravity`, `ollama`) returns
  `{"models":[{"id","label"}], "source":"cli|cache|builtin"}`; the agent caches it (6 h) so a menu
  opens instantly and offline. Claude: built-in aliases first, enriched from the catalog cache when
  its `version`/shape is the expected one. Antigravity: parse `agy models`, fall back to the last
  cached list. Ollama: `ollama list`.
- Quick commands get an optional `menu`: a source (`models` for the active tab's agent, or a fixed
  list the user writes) and a template (`/model {choice}`). Long press shows the menu; a short tap
  keeps today's behaviour. The editor gets "Menu" under a command.
- Which agent's list: the active tab's agent kind (already known per tab).
- Values from a host list are validated (`[A-Za-z0-9._:/-]`, short) before they are typed into the
  terminal, so a hostile model name can never inject keystrokes.

## Open
- Confirm the syntax of `/model` inside agy; until then the Antigravity menu can only start a new
  session with `--model` (a profile option) rather than switch a running one.
