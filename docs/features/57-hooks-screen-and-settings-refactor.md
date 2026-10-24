# Feature 57: Agent Hooks Screen, Extra Hook Commands and Settings Refactor

- **Slug**: `hooks-screen-and-settings-refactor`
- **Roadmap**: M46 (extends spec 14 and spec 46 section 3)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (user commands written into agent configuration and run on the host)
- **Status**: Design agreed with the owner 2026-10-24. Not implemented.
- **Owner decisions**: each hook event is ours, the user's own, or ours plus an extra command; the extra command runs **before** our hook by default, with the order, "stop the hook on failure" and the timeout as settings; profiles with extra or custom hook commands sync only inside the vault (spec 56); settings are grouped and long forms collapse into disclosure groups.

---

## 1. Problem

The agent profile screen has about fifteen sections on one page. The hook part lists raw event names (`PermissionRequest`) without saying what each does, and offers either our preset or one custom command that replaces our hook completely. The Settings root is seven flat links, and the pages behind them are long forms with long footers.

## 2. Hooks screen

Profile editor: the "Approval hooks" section shrinks to one row that opens the **Hooks** screen. Everything about hooks lives there.

### 2.1 Layout

- Header: mode, one segment: **Phone approval**, **Notify only**, **Off**.
- "Start from a template" (the existing template list).
- The events of this agent (`hook_events_for`), each a disclosure row. Collapsed: event name, one-line description, on/off switch. Expanded: the three choices of 2.2 and the matcher where the event takes one (`PermissionRequest`, `PreToolUse`, `PostToolUse`).
- Footer: only the warnings that matter (dangerous-skip flag, credential-looking command, lint results). The long explanations move behind an "i" popover.

### 2.2 Per event choice

1. **Ours** (default): the ssh-app hook script handles the event.
2. **Own command**: the user's command replaces our hook for this event.
3. **Ours + extra command**: our hook runs as usual and the user's command runs too.

For "extra" the settings are: order **Before** (default) or **After** our hook; **Stop on failure** off by default; timeout 10 s by default (1 to 60 s).

### 2.3 What each event means (texts live in the core, next to `hook_events_for`)

| Event | Description shown |
|---|---|
| `PermissionRequest` | The agent asks permission for an action. In Phone approval mode the answer comes from your phone. |
| `PreToolUse` | Before every tool call. Can approve or block it. |
| `PostToolUse` | After a tool call finished. Cannot block; for logging and checks. |
| `UserPromptSubmit` | You sent a prompt. Used to add context or record it. |
| `Notification` | The agent is waiting for you or needs attention. |
| `Stop` | The agent finished its turn. |
| `SubagentStop` | A subagent finished. |
| `SessionStart` / `SessionEnd` | A session began or ended. |
| `PreCompact` | The agent is about to compact its context. |

The core returns `hook_event_info(agent, event) -> HookEventInfo { title, summary, can_block, takes_matcher }` so the phone does not duplicate the table per agent. Events an agent does not have are not offered.

## 3. Semantics of the extra command

The agents run several hooks of one event in parallel, so the order of two separate hook entries is not guaranteed. The extra command is therefore run **by our hook script**, which keeps order and failure handling under our control.

- The extra commands of a profile are stored in the host's `~/.ssh-app/guard.json` under `extras` (written by the same guarded writer as the other guard rules, shown in "Review & install" like every other write).
- The script runs the command with `/bin/sh -c`, the agent's event JSON on stdin, the same environment as the hook, a process-group timeout, and output capped (stdout ignored, stderr's first 4 KB logged to `~/.ssh-app/hook.log`).
- **Before**: run the extra command; if it exits non-zero or times out and **Stop on failure** is on, the hook answers "deny" (for blocking events) or exits with the agent's blocking code, and our logic (including the phone approval request) is skipped. If **Stop on failure** is off, the result is logged and our hook runs.
- **After**: our hook runs first; the extra command runs after its decision. Stop on failure is not available (the decision is made); a failure is only logged.
- The extra command never changes the approval answer except by the explicit "stop" rule above. Its stdout is not parsed.
- For events that cannot block (`Notification`, `Stop`, `PostToolUse`, session events) "stop" has nothing to stop; the option is hidden for them.

## 4. Data and migration

`AgentProfile` gains `hookEvents: [HookEventConfig]`:

```swift
struct HookEventConfig: Codable {
    var event: String
    var enabled: Bool
    var mode: Mode            // .ours, .custom, .oursPlusExtra
    var command: String       // custom: the command; extra: the extra command
    var matcher: String
    var order: Order          // .before, .after   (extra only)
    var stopOnFailure: Bool   // extra only, blocking events only
    var timeoutSeconds: Int   // extra only
}
```

The old fields (`hookKindRaw`, `customHookCommand`, `customHookEvents`, `customHookMatcher`, `approvalEvents`, `approvalMatcher`) are read once when a profile decodes and converted: Phone approval / Notify only become `mode = .ours` for the old event set; Custom becomes `mode = .custom` for the old custom events. The old fields are then no longer written. A profile exported or synced by an older app keeps working because the encoder keeps writing the legacy fields for one release.

The Rust `HookPreset` gains the per-event configuration; `plan_hooks` produces the entries and the `guard.json` extras; validation of commands (`validate_custom_hook`, lint, credential check, `shell_syntax_problem`) applies to custom and extra commands alike.

iCloud: profiles with a custom or extra command stay local unless the vault is on and "hook commands" is ticked (spec 56 section 4.1); inside the vault they are tier B entries.

## 5. Settings refactor

### 5.1 Root

Grouped by meaning (grouped `List` sections with headers):

- **Connections**: Servers, SSH keys.
- **Agents**: Agents (profiles, hooks, permissions).
- **Terminal & look**: Terminal, Appearance.
- **Security & data**: Security (Face ID lock), Sync & backup, Modules.
- **Pro** (hidden while payments are off).

### 5.2 Pages

- Long pages split into disclosure groups, collapsed by default except the one or two main groups; the page remembers what was open for the session only.
- Footer texts: one line at most; longer help behind an "i" button (popover).
- Nothing is removed; content is re-ordered and folded.
- `SettingsView.swift` (838 lines) is split into one file per page: `ServersPage`, `KeysPage`, `TerminalPage`, `AppearancePage`, `SyncBackupPage`, `SecurityPage`, `ModulesPage`. New files are registered by hand in `project.pbxproj`.
- System components only (`DisclosureGroup`, `Form`, `LabeledContent`, `Menu`, popovers); new screens use Liquid Glass like the other new screens.

### 5.3 Related small fix

The approval banner shows the agent's raw tool input; JSON `\uXXXX` escapes appear as `пр…` instead of text (seen on the owner's phone 2026-10-24). The banner decodes the escapes for display (display only; the approved action is unchanged).

## 6. Security

- Extra and custom commands run on the host with the user's rights, as the hook script does today; nothing new is executed without the user pressing "Review & install", which shows each hook entry and each extra command.
- Commands are never interpolated into other commands: they are stored as data in `guard.json` and passed as one argument to `/bin/sh -c`; the agent config entries contain only our script path and event name for extra mode.
- Timeouts bound a hanging command; output is capped; a failing extra command cannot change the approval except through the explicit stop rule.
- A synced or imported profile cannot carry a command to another device outside the vault; an imported command needs confirmation (spec 56 section 5.3).
- `guard.json` writes use the existing guarded writer (symlink-aware, atomic, mode-preserving, backup).

## 7. Tests

- Rust: event info table complete for every agent and event; plan output for each mode and order; `guard.json` extras round trip; the script (Python tests in `scripts/agent-hooks`): before/after order, stop on failure, timeout, output cap, non-blocking events ignore stop; shell-injection attempts in command text (quotes, `$()`, newlines, a leading `-`) stay data; migration of old fields.
- Swift: `HookEventConfig` decode of old profiles; the legacy fields still written for one release; settings pages render (snapshot-free: build and navigation tests).
- Tests never touch the real tmux (private socket, `env_remove("TMUX")`).

## 8. Order of work

1. Spec review (`rev`).
2. Core: `hook_event_info`, per-event preset, plan and guard extras, script change, tests; `rev`, `hacker`.
3. Bridge bindings; `rev`.
4. UI: Hooks screen, profile editor slimming, settings regrouping and page split, banner fix; `rev`, `hacker`.
5. Merge; device checks on the owner's command.

## Revision notes (`rev`, 2026-10-24)

- Controversial, kept as a named tradeoff: running the extra command inside our script means "Own command" and "Off" modes have no stop/order semantics (they replace or remove our hook); the screen shows order and stop only for the extra mode.
- Controversial, kept: the extra command is stored in `guard.json` on the host rather than in the agent config, so it is not visible in the agent's own settings; "Review & install" and a read-back of `guard.json` in the Hooks screen keep it visible to the user.
- Resolved: a failed or hanging extra command cannot change the approval except through the explicit "stop on failure" rule.
