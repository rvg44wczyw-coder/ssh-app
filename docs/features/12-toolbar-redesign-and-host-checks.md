# Feature 12: Toolbar Redesign, Server Picker Popup & Host Ollama Check

- **Status**: Implemented (core verified by unit/probe/loopback tests; iOS builds for device; layout of the panel and popup rendered offscreen with `ImageRenderer`; popover chrome, long-press and haptics not yet exercised on a device — no simulator runtime is installed and the iPhone install is blocked phone-side)
- **Milestone**: M16
- **Layers**: `crates/core` (Ollama probe), `packages/SshCoreBridge`, `ios/`
- **Hacker gate**: required (new host command + parsing of host-controlled output)
- **Platform parity**: iOS only; Android has no UI skill yet (see roadmap backlog)

---

## 1. Problem

1. The navigation bar carries nine trailing icons. iOS cannot fit them and collapses the overflow into a system "⋯" button that the app does not control: it shows nothing meaningful, the icons are unlabeled and nobody can tell what they do.
2. Server choice is a `confirmationDialog` (a sheet that appears from nowhere) at startup/new tab, and a `Picker(.menu)` dropdown inside the tmux sessions sheet. Neither shows where the servers are, whether they are reachable, or which is current.
3. The AI assistant offers four hard-coded model names and fails with a raw curl/JSON error when Ollama is not installed, not running, or has no such model.

## 2. Goals / non-goals

Goals: one deliberate "more" button that opens a labeled action panel; a description for every action on long-press; a server picker popup used everywhere a server is chosen; an Ollama check that tells the user what is wrong and how to fix it, and drives the model list from the host.

Non-goals: changing what any action does; Android; installing Ollama for the user (the app only shows the command); a global theme change.

## 3. Toolbar and action panel

### 3.1 Layout
- The trailing toolbar contains exactly **one** button: `ellipsis.circle` ("More actions"). It is always visible, so iOS never creates its own overflow.
- The leading `rectangle.stack` (tmux sessions) moves into the panel. The title (agent/tab name • server) stays centered and is not interactive (switching a running tab to another server is deliberately not added: it reconnects the tab and an accidental tap on a title would be costly).
- Tapping "More actions" opens the **action panel** as a popover (`presentationCompactAdaptation(.popover)`, so it also stays a popover on iPhone): a 3-column grid of tiles, each an icon in a tinted circle with a one-line label underneath. Tapping a tile closes the panel and performs the action.

### 3.2 Actions (single source of truth)
`AppAction` is an enum (`CaseIterable`) with `title`, `systemImage`, `tint`, `hint`. The panel, the hint bubble and the accessibility labels all read from it; there is no second list.

| Action | Icon | Title | Hint (long-press text) |
|---|---|---|---|
| chatToggle | `bubble.left.and.bubble.right.fill` / `terminal.fill` | Chat / Terminal | Switch this tab between the raw terminal and a readable chat-style transcript of the agent session. |
| tmuxSessions | `rectangle.stack` | tmux sessions | List, open and close the tmux sessions that keep running on the host. |
| aiAssistant | `wand.and.stars` | AI assistant | Describe what you want in words; the local Ollama model on the host turns it into a shell command you can review before running. |
| snippets | `bolt.fill` | Snippets | Saved commands and fuzzy search over your shell history. |
| agentProfiles | `play.rectangle.on.rectangle.fill` | Agents | Start Claude Code, Antigravity and others in a new tab, choose what starts automatically, and manage their approval hooks. |
| fileBrowser | `folder.fill` | Files | Browse the host's folders, preview files, and insert a path into the terminal. |
| telemetry | `gauge.with.dots.needle.33percent` | Host monitor | CPU, memory, disk and running agent processes on the host. |
| webPreview | `globe` | Web preview | Open a dev server running on the host (localhost ports) inside the app. |
| diffViewer | `arrow.triangle.pull` | Git diff | Review the uncommitted changes of a repository on the host. |
| publicKey | `key` | Public key | Show or hide the key banner so you can copy this phone's public SSH key to a host. |
| settings | `gearshape` | Settings | Servers, SSH key and app options. |

### 3.3 Long-press hint
- A long-press (0.4 s) on a tile shows a **hint card** at the bottom of the panel with the action's title and `hint`, with a light haptic. The tile is *not* activated by that press.
- Discoverability: while no hint is shown, a one-line footer in the panel says "Press and hold an icon to see what it does".
- The hint stays until another tile is long-pressed, the panel closes, or 6 s pass. A plain tap on any tile still performs its action immediately.
- Accessibility: each tile is an accessibility button with `label = title` and `hint = hint` (VoiceOver users get the description without a gesture); Dynamic Type grows the tiles (the grid falls back to 2 columns at accessibility sizes).
- The panel is also usable with the hardware keyboard (focusable tiles, Return activates).

### 3.4 Behavior details
- Actions that need a live session (`webPreview`, `aiAssistant`, `snippets`, `telemetry`, `fileBrowser`, `agentProfiles`, `chatToggle`) are shown disabled with the hint "Connect to a host first" when `activeViewModel` is missing or not connected; they are never hidden, so the panel looks the same on every launch.
- State that used to live on the toolbar icons keeps its meaning: the chat toggle tile shows the current mode; the public-key tile shows a filled key while the banner is visible.
- Presenting an action first closes the panel, then calls the existing `presentSheet` (no behavior change, same single `ActiveModalSheet` state).

## 4. Server picker popup

`ServerPickerPopup`: a centered card on a dimmed backdrop (tap outside or Cancel dismisses), presented as an overlay of the main view with a short scale+fade transition. It replaces the startup/new-tab `confirmationDialog` **and** the `Picker(.menu)` in the tmux sessions sheet.

- Content: title and one-line purpose ("Open new tab on…", "Show tmux sessions of…"), then one row per server: name, `user@host:port`, a status dot (green reachable, red unreachable, gray unknown/checking), a checkmark on the current server.
- Unreachable servers stay selectable except in the startup and new-tab flows, which only offer reachable ones as before (`ServerPingService.checkReachableServers`); the popup there shows a "checking…" row state while pings run.
- Used in exactly three places: startup choice, new tab, and the tmux sessions sheet header ("Server: <name> ▾" rendered as a button). Statuses come from `ServerPingService` run when the popup opens (3 s per server, in parallel); until a result arrives a row is gray.
- Accessibility: the card is a modal accessibility container with the title as its label; rows are buttons; Cancel is reachable by the escape gesture.
- No new persistence: choosing a server uses the existing `handleServerChosen`, `setSessionServer` and `refreshSessions` paths.

## 5. Host Ollama check (rust core)

### 5.1 Types
`OllamaHostStatus { installed: bool, binary_path: Option<String>, version: Option<String>, running: bool, models: Vec<String> }`.

### 5.2 Functions
Module: `crates/core/src/ai.rs` (next to the existing Ollama response parser); the watchdog wrapper is factored out of `agents.rs::build_binary_probe_command` into a `pub(crate)` helper `watchdog_sh(script, secs)` in a small shared module `hostprobe.rs`, used by both probes.

- `build_ollama_probe_command() -> String` (fixed text, no inputs). Shape: `sh -c '<watchdog_sh(8 s) around: "${SHELL:-/bin/zsh}" -l -c "<probe>" </dev/null 2>&1>'`, so the 8 s host-side watchdog kills a hung probe and the login shell supplies PATH from `.zprofile`. The probe looks for `ollama` with `command -v`, then the known locations `/opt/homebrew/bin/ollama`, `/usr/local/bin/ollama`, `/Applications/Ollama.app/Contents/Resources/ollama`; prints `OLLAMA_PATH=<path>`, `OLLAMA_VERSION=<the line of `ollama --version 2>&1` that contains `version is`>` (with the server down the real CLI prints `Warning: could not connect to a running Ollama instance` first and then `Warning: client version is 0.12.3`, so stderr is merged and the `version is` line is picked), and `OLLAMA_TAGS=<single-line body of curl -s -m 2 http://127.0.0.1:11434/api/tags>`; absent parts print nothing.
- `parse_ollama_probe(output: String) -> OllamaHostStatus` (pure): takes the **last** well-formed value per marker, ignoring banner noise.
  - `binary_path`: absolute, no control/bidi characters, ≤ 4096 chars; otherwise ignored.
  - `version`: the text after `version is` (covers `ollama version is X` and `client version is X`), ≤ 64 chars, charset `[A-Za-z0-9._+-]`; otherwise `None`.
  - `running`: the tags body is a JSON object with a `models` array. `models`: each `name` must match `[A-Za-z0-9._:/-]{1,128}`, others are dropped; at most 100 names, sorted, de-duplicated.
  - `installed = binary_path.is_some() || running` (a running server on the loopback port means it is there even if the binary lives somewhere unusual).
  - Garbage, truncated JSON or oversize input never panics and yields `installed = false` as appropriate.
- Session: `check_host_ollama() -> OllamaHostStatus`, 10 s client timeout (a timeout is an error, not "not installed"); output is capped by the exec cap.

Limitations: the outer command is parsed by the SSH user's login shell before `sh` runs it; like every exec command in the app (`[ ]`, `$()`, `||`) it assumes a POSIX-compatible login shell (zsh/bash/sh), and fish/nu are untested. The inner hops (`-l -c`, then `exec sh -c`) are why the probe still works when the login shell's rc files do unusual things to `PATH`. Only the default endpoint `127.0.0.1:11434` is checked, the same one `query_host_ollama` uses; a server started with a custom `OLLAMA_HOST` is reported as "not running" and the message says "not reachable on 127.0.0.1:11434".

### 5.3 Example
Output
```
OLLAMA_PATH=/opt/homebrew/bin/ollama
OLLAMA_VERSION=ollama version is 0.12.3
OLLAMA_TAGS={"models":[{"name":"qwen2.5-coder:7b"},{"name":"llama3.2:latest"}]}
```
→ `installed: true, binary_path: "/opt/homebrew/bin/ollama", version: "0.12.3", running: true, models: ["llama3.2:latest","qwen2.5-coder:7b"]`.

## 6. AI assistant sheet integration (iOS)

On open (and on a "Re-check" button) the sheet calls `check_host_ollama` and shows one of:

| State | Shown | Generate |
|---|---|---|
| checking | spinner "Checking Ollama on the host…" | disabled |
| not installed | "Ollama is not installed on this host." + `brew install ollama` (copyable) and ollama.com/download | disabled |
| installed, not running | "Ollama is installed (v…) but not running." + `ollama serve` or "open the Ollama app" | disabled |
| running, no models | "No models yet." + `ollama pull qwen2.5-coder:7b` (copyable) | disabled |
| ready | chips built from the host's `models`; the last used model (UserDefaults key `ai_last_model`) is preselected if still present, otherwise the first | enabled |
| check failed | the error text and Re-check | disabled |

Results from an older check never overwrite a newer one (each check carries a token; a stale result is dropped). The hard-coded model list is removed. The selected model name is passed unchanged to the existing query; it already came from the validated set. Commands shown for copying are constants.

## 7. Security

- The probe command is a constant; the only host-controlled data entering the app is its output, which is parsed with strict charsets and size caps (5.2) and displayed as plain `Text` (never interpreted, no markdown/links). Model names later go into a JSON payload built by `serde_json`, never into a shell string.
- The probe runs on the user's own host under their own account and is read-only (`command -v`, `--version`, a loopback GET); it starts no server and installs nothing.
- The 8 s host watchdog and 10 s client timeout bound a hung `ollama --version`; output is capped like every exec call.
- The action panel and popup add no new capability: they call the same handlers; disabled actions cannot be triggered while disconnected.

## 8. Tests

- Unit (`parse_ollama_probe`): the example; not installed (empty output); installed without server; running without models; noise and duplicate markers (last wins); hostile model names (`; rm -rf`, spaces, newline, 200 chars) dropped; > 100 models truncated; truncated/invalid JSON; bidi path rejected; version cleaning.
- Unit (`build_ollama_probe_command`): is a single `sh -c` string, contains the three markers, the watchdog and the known paths, has no placeholders.
- Probe behavior (local subprocess, like the binary-probe test): run `build_ollama_probe_command()` through `sh` with `SHELL` pointing at a wrapper script that puts a fake `ollama` first on `PATH`: (a) fake prints a version and the tags JSON is served by a tiny loopback listener → parsed status; (b) fake `ollama` that sleeps forever → the call returns within the watchdog and the fake process is gone. (A fake binary cannot be injected through sshd: the login shell resets `PATH`.)
- Loopback sshd: `check_host_ollama` returns without error and `installed` equals whether `ollama` resolves on the test machine.
- iOS: `xcodebuild` for device; rendering of the panel, hint and popup verified in the simulator (screenshots) when the simulator slice builds, otherwise on the device.

## 8.1 Implementation notes
- Files: `Models/AppAction.swift`, `Views/ActionPanelView.swift`, `Views/ServerPickerPopup.swift`; `MainTerminalView` (single "More" button, popover, popup overlay, `performAction`), `TmuxSessionsSheetView` (server button + popup), `AiAssistantSheetView` (host check states).
- Rows/tiles/Cancel use tap gestures with the button accessibility trait instead of `Button`, so the same views can be rendered offscreen for review (`ImageRenderer` cannot draw `Button`/`ScrollView`); VoiceOver sees them as buttons with label and hint.
- The panel is a snapshot taken when it opens (connection state, chat mode, key banner); it does not live-update while open, which is acceptable for a menu that closes on the first tap.
- Tiles are `focusable()` with Return/Space handlers (hardware keyboard), as specified in 3.3.
- The package still lists macOS, but the sources already use iOS-only API unguarded (`UIColor`, `navigationBarTitleDisplayMode`); the toolbar button likewise assumes iOS.
- The server list uses `ViewThatFits`: it sizes to its content and scrolls only when it would not fit.
- Side fix found by rendering: `Text("…:\(port)")` formats the number by locale (port 2222 appeared as `2.222`); the three places that did this now use `Text(verbatim:)`.

## 9. Open items
- Android panel/popup/Ollama check (no `android-ui-dev` skill yet).
- Switching a running tab's server from the title is out of scope (reconnect risk).
- A "pin" option to keep one or two favorite actions next to "More" is deliberately not part of this change (the request is to hide all of them behind the button); revisit after use.
