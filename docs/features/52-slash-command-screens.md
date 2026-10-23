# Feature 52: `/btw` answers and native screens for slash-command menus

- **Status**: Draft 2026-10-22 (owner request: "catch the `/btw` output; every command with its own menu and settings should be a session-settings screen"). Not reviewed yet.
- **Agent**: Claude Code first (dialect `Claude` of `crates/core/src/screen`). Antigravity and Ollama have no such menus; they are untouched.
- **Builds on**: the screen feed (`screen/`, feature 03/17), `ScreenPrompt` cards, the session settings sheet (`SessionPermissionsSheetView` and the tab dropdown).

## 1. Purpose

Claude Code answers a few commands not in its transcript but in a **full-screen overlay or dialog drawn on the pane**:

| Command | What the pane shows (Claude Code 2.1.308, captured in a private tmux, 100×40) |
|---|---|
| `/btw <question>` | The question line, `· Answering…`, then the answer text; footer `↑/↓ to scroll · c to copy · f to fork · Esc to close`. Not written to the session file; gone after `Esc`. |
| `/model` | `Select model`, numbered list (`❯` cursor, `✔` current), effort line `◐ Medium effort ←/→ to adjust`, footer `Enter to set as default · s to use this session only · Esc to cancel`. |
| `/permissions` | Tab header `Permissions  Recently denied  Allow  Ask  Deny  Auto mode  Workspace`, a search box, numbered rules. |
| `/config` | Tab header `Settings  Status  Config  Usage  Stats`, a search box, rows `name   value` with `❯` cursor. |
| `/mcp` | `Manage MCP servers`, grouped list with state glyphs (`✔ ◯ ⚠`) and `needs authentication`. |
| `/status` | Same header as `/config`, rows `Key: value`. |

Today the app shows none of this usefully: the feed turns the overlay into a folded "Terminal output" turn, or into a generic prompt card when it looks like a menu. The owner wants (1) the `/btw` answer caught and shown as an answer, and (2) each such command opened as a **native screen** reachable from the session settings, driven by keys sent to the pane.

## 2. Behaviour

### 2.1 `/btw`

- The parser recognises the overlay by its last line (`… Esc to close`) and the echoed `/btw <question>` line above the text.
- `ScreenFeed.aside: Option<ScreenAside>` with `question`, `answer` (empty while `pending`), `pending: bool`. While present, the overlay lines are **not** parsed as turns (they would pollute the feed).
- The app shows it as a card above the input (like `FeedPromptCard`): the question, a loader while pending, the answer as Markdown, buttons **Copy** (`c` is not sent; the app copies its own text) and **Close** (sends `Esc`). A scrolling answer longer than the pane is read by sending `PageDown` (the pane shows only what fits); the card says "more below" when the footer offers `↑/↓ to scroll`.
- Closing the card, or the overlay vanishing from the screen, removes it. Nothing is stored.

### 2.2 Command screens

- `ScreenFeed.dialog: Option<ScreenDialog>` for the full-screen dialogs above. A dialog has `kind` (`model`, `permissions`, `config`, `mcp`, `status`, `unknown`), `title`, `tabs` (names + the active one), `search` (text or none), `rows`, `selected`, `hints` (the footer), `more_above` / `more_below` (`↑ 10.` / `↓ … +3 models`).
- A row is `{ number?, label, value?, detail?, state }` where `state` is `current` (✔), `cursor` (❯), `on/off` (`true`/`false` values of `/config`), `warning` (⚠), `off` (◯).
- `ScreenPrompt` (permission questions, choices) keeps priority: a dialog is only reported when it is not a prompt and the pane is in the Claude dialect.
- The app has one generic **CommandScreen** (a `List` with the system components: `Picker`/`Toggle`/`LabeledContent`, search field, tab picker) fed by `ScreenDialog`; per kind it only picks icons and the footer actions (for `/model`: "Set as default" = `Enter`, "This session only" = `s`; effort via `←/→`).
- Opening: a new group **Claude commands** in the session settings (`Model`, `Permissions`, `Settings`, `MCP servers`, `Status`) sends `/<command>` + `Enter` and shows the screen when the dialog appears (timeout 3 s → a message, the typed command is cleared with `Ctrl-U`). Choosing a row moves the cursor with arrow keys to the row (known `selected`) and sends `Enter`; toggles in `/config` send `Space`/`Enter` as the footer says; closing sends `Esc`.
- If the dialog on the pane changes under the screen (the user used the terminal), the screen re-renders from the new capture; the screen never assumes a row index it did not read in the last capture.

### 2.3 Not in scope (decisions to confirm with the owner)

- Commands without a dialog (`/compact`, `/clear`, `/cost`) stay as quick commands; their text output is already in the feed.
- Editing `permissions` rules (typing a rule) is shown read-only; typing rules stays in the terminal tab.
- No writes to `~/.claude/settings.json` from the app: only what the dialog itself does.

## 3. Interface

```rust
pub struct ScreenAside { pub question: String, pub answer: String, pub pending: bool, pub scrollable: bool }
pub enum DialogKind { Model, Permissions, Config, Mcp, Status, Unknown }
pub struct DialogTab { pub name: String, pub active: bool }
pub enum RowState { Plain, Current, Cursor, On, Off, Warning, Disabled }
pub struct DialogRow { pub number: Option<u32>, pub label: String, pub value: Option<String>, pub detail: String, pub state: RowState }
pub struct ScreenDialog { pub kind: DialogKind, pub title: String, pub tabs: Vec<DialogTab>, pub search: Option<String>,
                          pub rows: Vec<DialogRow>, pub selected: u32, pub hints: String, pub more_above: bool, pub more_below: bool }
// ScreenFeed gains: pub aside: Option<ScreenAside>, pub dialog: Option<ScreenDialog>
```

All text from the pane is untrusted display data: control characters are already stripped by `parse_screen_feed`; lengths are capped (row label 200, value 200, rows ≤ 200, answer ≤ 20 000 chars); nothing from a row is ever executed or written to a file.

## 4. Constraints and invariants

- Pure functions over text, like the rest of `screen/`; no I/O; no `unwrap()` in public paths.
- A dialog or aside never hides a real permission prompt: the prompt is parsed first.
- Keys are sent only after the user's tap on a row; the app never auto-selects.
- Battery: the existing screen watch/poll is reused; no new timers (the dialog is read from the same capture).
- Security: a row label cannot contain a key sequence that the app would send; the app sends only the fixed keys (arrows, Enter, Space, Esc, `s`, `←/→`, PageDown), never row text. `/btw` text typed in the app is a normal prompt line (already escaped as input text).

## 5. Tests

Fixtures are the real captures above (stored as test strings, with the account name removed): `/btw` pending and answered, `/model` (cursor on a middle row, `↓ … +3 models`), `/permissions`, `/config`, `/mcp`, `/status`; a screen where a permission prompt and a dialog coexist (prompt wins); a dialog cut at 40 rows; garbage with `Esc to cancel` that is not a dialog (kind `Unknown`, not shown as a screen). Swift: the key plan for "move cursor from row a to row b" is a pure function with tests (up/down counts, with the `↓ … +N` window).

## 6. Dependencies and plan (dev-chain)

`doc-dev` (this) → `rev` → `rust-core-dev` (parser, tests) → `rev` → `swift-bridge-dev` (regenerate bindings) → `rev` → `ios-ui-dev` (aside card, `CommandScreen`, settings group, key plan) → `rev` → `hacker` (screen text is untrusted: caps, key injection) → merge.
