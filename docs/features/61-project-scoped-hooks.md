# 61. Hooks per project, not per agent

Status: draft (2026-10-26, owner's request). Builds on `11-agent-hooks-and-launch-profiles.md`, `57-hooks-screen-and-settings-refactor.md`.

## 1. Problem

The installer writes the phone-approval hooks into the agent's **user-level** config (`~/.claude/settings.json`, `~/.gemini/config/hooks.json`, …). From then on **every** session of that agent on the host asks the phone, including terminals that were never meant to be tied to it: with no phone connected the hook waits for its timeout (120–130 s) before the agent falls back to its own prompt. The owner wants the phone approval only where he chose it.

## 2. Decision

Two scopes, **project is the default**, user-level stays available as an explicit choice:

| Scope | Where it is written | Affects |
|---|---|---|
| Project (default) | the agent's project-level file under the session's folder | only sessions started in that folder |
| All sessions | the agent's user-level file (today's behaviour) | every session of that agent |

The two levels must not both be on (every request would reach the phone twice). Owner's rule (2026-10-26): a project install **switches "All sessions" off**, with a warning and an explicit confirmation ("sessions in other folders will no longer wait for the phone until you enable them one by one"). The core refuses by default (`disable_user_level = false`), the app asks and then passes `true`; only our entries are removed from the user-level file.

## 3. Project-level targets

| Agent | Project file | Status |
|---|---|---|
| Claude Code | `<folder>/.claude/settings.local.json` (personal, not committed by convention) | documented |
| Antigravity | `<folder>/.agents/hooks.json` | documented |
| Codex | `<folder>/.codex/hooks.json`, loaded only when the project layer is trusted, hooks reviewed with `/hooks` | documented (OpenAI hooks docs) |
| Grok | `<folder>/.grok/hooks/ssh-app.json` (any `.json` in that folder is read), needs a one-time `/hooks-trust`; fail-open | documented (docs.x.ai hooks) |
| Qwen Code | `<folder>/.qwen/settings.json` (overrides the user file) | documented |
| OpenCode | `<folder>/.opencode/plugins/ssh-app.js` | documented (v2 docs: discovery location unchanged) |
| Crush | none yet | project file not confirmed: user-level only |
| Cline, Aider, Goose, Ollama chat | no hook mechanism | launch-only |

Rows from the docs were not run live (owner, 2026-10-26: live checks together with the phone verification); Crush stays user-level until its file is confirmed.

For the committed-file risk: the installer adds nothing to `.gitignore`; for Claude it uses `settings.local.json`, which Claude Code itself creates ignored. For agy's `.agents/hooks.json` there is no local variant: the preview says the file is meant to stay out of git and offers to append it to `.git/info/exclude` (never `.gitignore`).

## 4. Interface (core)

- `agent_hooks_config_path(agent, config_dir)` gains a sibling `agent_project_hooks_path(agent, folder) -> Result<String>`; error `unsupported` for rows not yet verified.
- `install_agent_hooks_with_preset`, `preview_agent_hooks`, `installed_agent_hooks`, `uninstall_agent_hooks` get a `scope: HookScope { User, Project(folder) }` (UniFFI enum) instead of overloading `config_dir`. `folder` goes through `validate_dir` and must be absolute or `~/`-relative; the same path rules as `config_dir`.
- `agent_hooks_overview(folders) -> [AgentHooksOverview]` runs on the connected server and returns, per agent, whether it is installed there and the state of its user-level file and of every given folder in one call, so the screen shows where hooks are active.
- Hook commands are unchanged (`python3 -I "$HOME/.ssh-app/bin/ssh-app-hook" <agent> <Event>`).

## 5.0 Hooks belong to a server

Hooks live in files on one host, and hosts differ: Claude on one server, Antigravity on another, Grok on a third. So the unit is **(server, agent, scope)**, never "the agent" alone:

- The Hooks screen opens **under a server** (the connected one; Settings → server → Hooks), not globally.
- Its agent list is what is installed **on that server**: the host answers `command -v` for the binaries of the registry rows (`agent_registry.rs`, login shell as everywhere) in one call, `agent_hooks_overview` below carries it. Agents not found are hidden behind "Show all"; a hidden agent can still be set up (it may be installed in a path the login shell does not see).
- Projects are per server too: the working folders of saved profiles are offered only for the servers the profile belongs to. Today `AgentProfile` has no server field (profiles are shared by all servers). Added: `serverIds: [String]`, empty = every server (migration keeps existing profiles at "every server"); the profile editor gets a "Servers" row.
- The install state is read from the host each time the screen opens and cached per server for the session; nothing about hooks is stored on the phone besides the profile settings of spec 57.
- A new server offers nothing automatically: hooks are installed only from this screen or the in-feed offer.

### 5.0.1 Categories

Servers are not the only way people split their agents (owner, 2026-10-26: "или категоризированы"). A profile and a project can also carry a **category** (free text label, e.g. "work", "personal", "experiments"; `AgentProfile.category`, empty = none):

- The Hooks screen groups its rows by category inside a server (a section per category, "Other" last); the project rows take the category of the profile whose folder they are.
- A category header has a bulk action "Enable / Remove for all in this category" with the same confirmation list as "Remove everywhere".
- A category can be set on a folder that has no profile too (stored on the phone as `folder → category` per server, a small dictionary next to the profiles).
- Category never changes where hooks are written; it only groups the screen and the bulk actions.

### 5.0.2 The host helper discovers them

The phone must not be the only source of the list (owner, 2026-10-26): the host helper (`ssh-app-host`) finds the agents and the projects by itself and reports them with the hook state, so a server set up from another phone, or sessions started at the laptop, still show up.

New helper call `hooks-overview [--folder F]…` (cap 64 folders), read-only, one JSON reply, also served by the host agent (`m: "hooks"`):

- **Agents on the host**: the registry rows' binaries via `command -v` in a login shell, plus any agent seen running in tmux (the usage probe's detector).
- **Projects, from what already exists on the host**, without reading file contents beyond the folder name:
  - running panes: `pane_current_path` of every tmux pane with a detected agent;
  - Claude: folder names under `<config dir>/projects/` (decoded; only folders that still exist);
  - Codex: `cwd` of the newest rollouts' first line (`session_meta`), never the account fields;
  - Grok: folder names under `~/.grok/sessions/` (percent-decoded);
  - agy: keys of `~/.gemini/antigravity-cli/cache/last_conversations.json`.
  Each project carries `agent`, `last_used` (mtime) and `running` (a live pane there). The list is capped (64 newest per agent) and every path is canonicalised; paths outside the user's home or non-directories are dropped.
- **Hook state per target**: absent / installed (ours, marker found) / foreign hooks only / needs review (Codex `/hooks`, agy project trust), read from the user-level file and from each project file.
- The phone merges its own knowledge (profile folders, categories) with this list; a project known to both appears once.

The helper only reports. Installing and removing stay in the core through the existing hook install path, so the helper gains no write access to agent configs. The helper is pinned by sha256: adding the call is a new pinned build.

## 5. UI: one place that manages both levels

The owner's addition (2026-10-26): hooks must be manageable for the agent as a whole **and** for every project the phone knows. One screen, "Hooks", per agent:

```text
Claude Code
  All sessions            ● installed   [Remove]     <- user-level file, off by default
  Projects
    ~/rust/ssh-app        ● installed   [Remove]
    ~/work/api            ○ not set     [Enable]
    ~/Terminals           ○ not set     [Enable]
  Other folder…                                     <- type or pick a folder on the host
```

- **Project list** = the folders the phone knows for this agent: the working folders of its saved agent profiles (`AgentProfile.workingDir`), the folders of the open tabs (`pane_current_path`, git worktrees included) and the folders where hooks were installed before (found by the status call below). Duplicates collapse by resolved path.
- Every row shows its state (installed / not set / needs review in the agent / foreign hooks present) read from the host, and has Enable / Remove; the row opens the same preview and per-event choices as the existing sheet (spec 57), for that target file.
- "All sessions" is a row of the same list, collapsed under a warning ("Every session of this agent will wait for the phone") and off unless the user turns it on; enabling it while project rows are on offers to remove those (they would double up).
- Bulk: "Remove everywhere" lists every installed target before doing anything.
- A new tab in a folder without hooks: a quiet one-line offer in the feed header ("Approve from the phone in this project"), not a modal.
- Status call: `agent_hooks_overview(folders[]) -> [AgentHooksOverview { agent, found, targets: [HookTarget { scope, folder?, file, state }] }]` for the connected server: which agents exist there and their targets, one SSH round trip (cap 64 folders).

## 6. Security

- The folder is the tab's `pane_current_path`, validated and quoted like every path the app sends; the file written is a fixed name under it, never taken from input.
- The project file is writable by anyone with write access to the repo; the hook it runs lives in `~/.ssh-app/bin`, not in the repo, and the command string is generated by the installer. An existing foreign hook entry in the file is preserved exactly as for the user-level file (merge, marker-based removal).
- Trust gates of the agents (Codex `/hooks`, agy project hooks) stay in force; the app only reports "needs review".

## 7. Tests

Golden snapshot of every project preview; merge/remove round trip on a file with foreign hooks; refusal when user-level hooks are present; path validation (traversal, control characters, relative folder); loopback sshd test on a temp folder (never the real `~/.claude`).

## 8. Order of work

`rev(doc)` → core (`HookScope`, project paths for Claude and agy) → bridge → UI (segment, header offer) → `hacker` → live verification of the Codex and Grok project paths → their rows.

## 9. Open

- [ ] Helper `hooks-overview` (discovery sources above) and its golden tests on fixture homes.
- [ ] Categories: `AgentProfile.category`, folder → category dictionary, grouping and bulk actions (confirm with the owner that a free-text label is what he means).
- [ ] `AgentProfile.serverIds` migration and the Servers row in the profile editor.
- [ ] Codex and Grok project-level hook paths, live check (owner, 2026-10-26: later, together with the phone verification, when a phone build is possible; no agent is launched for it before that).
- [ ] Whether the "no phone connected" wait itself should shrink (the hook could ask the broker whether any phone session is attached and return no opinion at once); separate item, spec 11 §3.

## Progress (2026-10-26)

| Part | State |
|---|---|
| Core: `HookScope`, `agent_project_hooks_path`, `*_in` calls, refusal to double user-level hooks (Claude, agy) | done, tests + loopback sshd test |
| Host helper `hooks-overview` and core `agent_hooks_overview` | done, tests, run on the real home read-only |
| Bridge regenerated, Swift `HooksOverviewView` (Settings → Agents → Hooks), simulator build green, 192 Swift tests green | done; **not seen on a device** |
| Security pass | `docs/security-findings/project-hooks-and-quota-probes-2026-10-26.md`: Low only |
| Update 2026-10-26 | project paths for Codex, Grok, Qwen Code and OpenCode added from the docs; the helper lists all seven hook-capable agents; installing for a project asks to switch "All sessions" off (`disable_user_level`) |
| Not done | per-server profile binding (`serverIds`) and categories (§5.0, §5.0.1); in-feed offer; `.git/info/exclude` offer for agy; Live check of the Codex/Grok/Qwen/OpenCode project files (later, with the phone); Crush project file; the "no phone attached, return at once" hook change |

