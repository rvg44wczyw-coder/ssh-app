# Feature 59: Background tasks of the agent (one model for Claude Code, Antigravity and the next agents)

- **Slug**: `background-tasks`
- **Target Release**: Milestone 48
- **Roles**: `doc-dev`, `rev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `hacker`
- **Hacker Gate**: Yes (pane text is untrusted display data; the app sends a stop key on the user's tap)
- **Status**: Spec (2026-10-25). Not reviewed. Builds on `docs/architecture/screen-parser.md` and the dialog parser of `52` (slash-command screens).

## 1. Purpose

An agent can start a shell command in the background (`run_in_background` in Claude Code, a "backgrounded" command in
Antigravity) and keep talking. Today the feed shows only a folded call and, when the command ends, a generic "Background
task" note. The owner cannot see that something is still running, what it is, how long it has run, its output, or stop it,
without switching to the raw terminal.

This feature shows the running background tasks of the tab's agent: a counter in the feed header, a screen with the list
and the detail of each task, and a stop action. One model serves every agent; an agent is connected by a table row, not by
new screens.

## 2. What the agents show (live captures, 2026-10-25)

Captured in a private tmux socket (Claude Code 2.1.308, Antigravity CLI 1.4.7), each with one `sleep` started in the
background. Fixtures use `/tmp/project`; the account line of the agy banner is never stored.

| | Claude Code | Antigravity |
|---|---|---|
| In the feed | `Bash(sleep 90)` then `Running in the background (↓ to manage)` | `● Running (sleep 120) (ctrl+o to expand)` |
| Count | footer mode line `auto mode on · 1 shell`; turn summary `… · 1 shell still running` | footer `Gemini 3.8 Flash · medium · 1 task(s) · /tasks`; line above the footer `● [11:40:03] sleep 120 running` |
| Open the list | `↓` focuses the footer pill, `Enter` opens it | slash command `/tasks` |
| List | with one task it opens the detail directly; several: a list (to be captured) | title `Tasks`, subtitle `Agent Backgrounded`, `>` cursor, rows `● [time] command … running` |
| Detail | title `Shell details`: `Status:`, `Runtime:`, `Command:`, `Output:` | title `Task Details`: `Command:`, `Status:`, `Type:`, `Started:`, `Output` |
| Stop | `x` in the detail | `k` (kill), `x` removes it from the list |
| Close | `Esc`, `Enter` or `Space`; `←` goes back | `Esc` goes back |
| Transcript file | tool call with `run_in_background: true`; on completion a user turn `<task-notification>` (task id, status, summary) | not in a file the app reads today (database feed: to be checked) |

## 3. The model

```text
BackgroundTask { command: String, status: Running | Done | Failed | Unknown(String),
                 started: String?, runtime: String?, kind: String?, output: String? }
BackgroundSummary { count: u32, running: Vec<String> }     // from the footer / status lines
```

- `ScreenFeed.background: Option<BackgroundSummary>`: parsed on every poll from the footer of the dialect (count) and, for
  agy, the status lines above it (command per task). `None`: the agent shows nothing about background tasks.
- A new `DialogKind::Tasks` for the screens of §2 (list and detail). A detail is a `ScreenDialog` whose `note` carries the
  fields and whose `rows` hold the output lines; a list is a `ScreenDialog` whose rows are tasks. The generic dialog
  renderer already shows these; `Tasks` only adds the stop button.
- All text is clipped (command 200 characters, output 20,000, rows 200) and stripped of control characters, as the other
  dialog parsers do. Nothing parsed here is ever executed.

## 4. Per-agent table (`AgentTraits`)

| Field | Claude | Antigravity | Generic |
|---|---|---|---|
| `openTasksKeys` | `↓`, then `Enter` (focus the footer pill and open it) | `/tasks` + `Enter` (a slash command) | none: the chip opens nothing |
| `stopTaskKeys` | `x` (inside the detail) | `k` (inside the detail) | none |
| `closeTasksKeys` | `Esc` | `Esc` | none |

A new agent is connected by a parser for its count and screens plus one row of this table. Where the row is missing, the
header chip is shown as a counter only.

## 5. The screen (iOS)

- **Header chip**: "N tasks" (a glass chip next to the git branch) while `background.count > 0`; hidden at 0. Tapping it
  opens the sheet and sends `openTasksKeys`.
- **Sheet** (`presentationDetents` medium/large, Liquid Glass): the list of tasks (command, status, runtime); a task opens its
  detail (command, status, started, output in a monospace block with Copy); a **Stop** button with a confirmation, which
  sends `stopTaskKeys` for the task that is open, then re-reads the screen.
- Closing the sheet sends `closeTasksKeys`, so the agent's own screen is never left open behind the app.
- The sheet only drives the agent's own screen. It never starts, builds or runs a command of its own.
- **Feed note**: the `<task-notification>` turn of Claude is shown as "<command> finished (exit 0)" using the task id to find
  the call that started it; unknown ids keep the generic "Background task" text.
- Zero-battery rule: the tasks are the host's; nothing runs on the phone, and the sheet reads the pane only while open.

## 6. Security

- The pane is untrusted: a task's command or output can contain text that looks like a prompt, a key sequence or a dialog.
  It is rendered as text only, with control characters removed; it never becomes input to the agent.
- The stop key is sent only after the user taps Stop in the detail of a task, and only while the matching dialog (`Tasks`,
  detail) is on screen; if the screen is anything else (a permission question, a different dialog) no key is sent.
- The open keys go only to a tab whose pane was recognised as that agent (`feedRecognized`); never into a shell. For Claude the
  open sequence starts with `↓`, which in a box that holds text only moves the cursor: it is sent only while the agent's box
  is empty (no draft on screen, the feed field empty), and the footer pill must be on screen first.
- The count in the header is advisory: a hostile output cannot make the app send any key by itself.

## 7. Tests

- Core: fixtures from the captures of §2 (idle, one task running in the feed, the list, the detail, with several tasks once
  captured), a "no tasks" screen, counts of 0/1/12, `1 shell` vs `2 shells`, text with escape sequences and a fake prompt
  inside the output. No panic on any truncation of a fixture.
- Swift: traits table, the key sequences, the "do not send stop unless the detail is on screen" rule (pure function), chip
  visibility.
- Live checks (each needs the owner's permission for that task): Claude with two shells (the list), Claude `/tasks`, agy
  with a finished task and with several, agy database feed for tasks.

## 8. Phases

| Phase | Content | Done when |
|---|---|---|
| 1 | Core: `BackgroundSummary`, `DialogKind::Tasks`, parsers for both dialects on fixtures | tests green, no `unwrap` in public paths |
| 2 | Bridge regeneration | bindings build |
| 3 | iOS: traits, chip, sheet, stop with the screen rule, notification note | simulator build + Swift tests |
| 4 | rev, hacker, device run | findings fixed |
| 5 | Further agents (Codex, Grok, ...) by table row after their live captures | per agent |

## 9. Review (`rev`, 2026-10-25), applied

- Documentation checklist: purpose, interfaces, edge cases and tests are present; the model and the per-agent table are
  explicit. Gap closed: the stop and open conditions are now stated as pure rules (§6) so they can be tested.
- Security: untrusted pane text (§6) and the "no key unless the matching screen is up" rule cover the main risk. Added: the
  Claude open sequence is guarded against a non-empty box.
- `[controversial]` Reading the count from the footer text ties the feature to wording that an agent update can change
  (`1 shell`, `1 task(s)`). Accepted as a named tradeoff: a changed footer degrades to "no chip", never to a wrong key; each
  wording has a fixture, and a new wording is a one-line parser change.
- `[controversial]` Driving the agent's own screens by keys (instead of reading the host's process list) is fragile but is the
  only source that also works for tasks of the agent that the host's `ps` cannot attribute. The host process list stays
  available as the fallback if the keys fail (server-level Processes screen, spec 54).

## 10. Open questions

- Claude with several shells: the list layout is not captured yet (needs a Claude run with two tasks).
- Whether Claude's `/tasks` slash command exists in this version; the footer pill is the observed path.
- Antigravity's transcript source for tasks (database feed) is not checked; the count is read from the screen only.

## Revision notes

- 2026-10-25: first version from the live captures of Claude Code 2.1.308 and Antigravity CLI 1.4.7; review applied (§9).
