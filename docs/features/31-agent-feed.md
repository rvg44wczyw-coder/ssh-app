# Feature 31: Agent feed (reading and typing comfort for AI sessions)

Status: spec (2026-10-19). Builds on `07-structured-transcript-dual-mode.md` (Chat mode,
`StructuredTranscriptView`) and `20-git-log-and-ai-cli-detection.md` (pane detection).

## 1. Problem

The raw terminal is a poor place to read an agent's output and to write to it: the history jumps
while output streams, text is a character grid (no reflow, partial selection), and the input is
the terminal's own line (no cursor placement by finger, no multi-line editing, no undo). The
owner wants smooth scrolling, the comforts of a text editor for input and of a reader for output.

Decision (owner, 2026-10-19): keep SwiftTerm for TUIs and shells; make the existing **Chat mode**
the comfortable surface for Claude / Antigravity sessions ("feed"). Not in scope: replacing
SwiftTerm, parsing the terminal grid.

## 2. What exists and what is wrong with it

| Part | Today | Problem |
|---|---|---|
| Loading | `loadTranscript()` once on open, manual Refresh | stale: needs a tap to see new output |
| Scrolling | scrolls to the last turn on every count change, animated | yanks the reader away while reading history |
| Text | `Text`, not selectable, no markdown | cannot copy a paragraph, code blocks are plain |
| Input | one-line `TextField`, sends `text + "\n"` | no multi-line, no draft kept, `\n` is not Enter for TUIs |

## 3. Design (phase F1, iOS only; the core API is unchanged)

1. **Live tail.** While the feed is visible and the session connected, reload the transcript
   every 3 s (a `.task` that ends with the view, so nothing runs in the background or off-screen).
   A reload that returns the same turns (count and last turn's content length) changes nothing,
   so no re-render and no scroll. Errors keep the previous turns.
2. **Scroll that respects the reader.** The view sticks to the newest turn only while the reader
   is at the bottom (`defaultScrollAnchor(.bottom)` plus tracking of the bottom marker). When the
   reader scrolled up, new turns do not move the view and a "Latest ↓" button appears (with a dot
   when something new arrived). No animation on streaming updates.
3. **Reading.** Content is selectable (`textSelection`), rendered as Markdown (inline and fenced
   code blocks; code in monospace with a copy button). Thinking and tool output are selectable
   too. Syntax colours in code blocks reuse `SyntaxService` (feature 29) when enabled.
4. **Writing.** A multi-line field (grows to 6 lines, scrolls after that) with the system's
   editing: cursor by finger, selection, paste, undo, autocorrect. Return inserts a newline; the
   send button submits. The draft is kept per session in memory while the app lives.
   Submitting: single line → text + `\r`; multi-line → bracketed paste (`ESC[200~ … ESC[201~`)
   then `\r`, so the agent takes it as one prompt, not one prompt per line.
   A short row of keys above the field: Esc, Ctrl-C (interrupt), ↑ (history), Enter-only.
5. **Zero battery drain.** No timer outside the visible feed; the task is cancelled on
   disappear, on leaving Chat mode and on background (the connection is closed by lifecycle code
   anyway).

### 3.1 Text selection (changed 2026-10-19)

SwiftUI `textSelection` selects inside one `Text` only. The feed is therefore ONE native
`UITextView` (`FeedTextView`) built from an attributed string: selection runs across turns with
the system handles, magnifier and Copy / Share / Look Up. "Thinking" and tool headers are links
that expand or collapse their body. Cost: no card chrome, and code blocks are not syntax-coloured
yet (monospace on a tinted background); colours return when `SyntaxService` is wired into the
builder.

### 3.2 Look and formatting (2026-10-19)

The feed imitates Claude Code's terminal: monospace; `>` on a tinted band for what you wrote;
`●` for the agent's text; `● Tool(args)` with a status-coloured dot and `⎿` result lines folded to
four lines (`… +N lines` expands); `∴ Thinking…` dim and italic. Prose is formatted: headings,
bullet and numbered lists with hanging indents, block quotes, rules, tables as aligned columns,
inline bold / italic / `code` / strike-through. Fenced code is syntax-coloured when feature 29 is
on (colours arrive asynchronously, the text is rebuilt once they are in).

### 3.3 What the feed shows (2026-10-19)

Everything in the session file is shown, but only the exchange with the user is open; the rest is
folded, as in the Claude CLI.

| Record | Shown as |
|---|---|
| user message | bubble pinned to the right (about 3/4 of the width) |
| assistant text | left, full width, formatted Markdown |
| thinking, tool call + result | folded (`∴ Thinking…`, `● Tool(args)` + `⎿` lines) |
| hooks, attachments, meta messages, turn durations, compaction / checkpoints, system and error messages | one folded line per run: `▸ Hook · 3× Attachment · Worked for 8s` |
| `<system-reminder>`, `<command-name>`, `<local-command-stdout>` ... (known tags only) | folded block under a name ("System reminder", "Command output"), not the raw tag |
| API telemetry, mode, titles, snapshots, queue operations, subagent (sidechain) records | not shown (bookkeeping, no content) |

Shell commands (Bash / run_command) with more than one step are drawn as a tree: `&&` / `||` / `;`
steps are siblings, a pipeline hangs under its first command, a here-document is its folded body
(`… +N lines`); over 12 steps the rest folds.

Shell commands open in stages, each tap goes on: closed (header with the step count only) → steps as a
tree + a 4-line result → everything (all steps, heredoc bodies, full result) → closed again.
```
● Bash(4 steps)
├─ cd /a
├─ && cargo fmt
├─ && cargo clippy -p x -q 2>&1
│  └─ | head -30
└─ ; cargo test -p x
   └─ | tail -3
```

The core (`transcript.rs`) reads Claude Code's own format (`type` + `message`; one reply is written as
several records with one `message.id` and is joined into one turn) and Antigravity's real shapes
(`args` JSON-encoded strings, `GENERIC` tool results, `<USER_REQUEST>` wrapper, checkpoints, errors).
`TranscriptTurn.label` marks service turns. Transcript lookup also follows `CLAUDE_CONFIG_DIR` and
`~/.claude-*` (previously only `~/.claude`, so an account in another folder showed an old file).

## 4. Security

- The text typed goes to the user's own session, as today. Bracketed-paste end marker inside the
  user's text (`ESC[201~`) is stripped, so pasted content cannot close the paste early and have
  the remainder interpreted as keystrokes.
- Transcript content is untrusted display data: Markdown is rendered inline-only (no links are
  opened automatically; link taps go through the system's confirmation), no HTML, no images.
- No new remote command; no new core surface.

## 5. Failure behaviour

No transcript found (plain shell): the existing empty state stays and the field still sends text
to the pane. Reload errors are shown once in the header, not replacing turns.

## 6. Tests

- Unit (Swift, compile-checked; the suite only compiles in CI today): `FeedSubmit.bytes(for:)`
  (single line, multi-line, stripped end marker), `TranscriptDigest` equality.
- Manual on device: stay scrolled up while new output arrives; select a paragraph; paste a
  3-line prompt to claude and see it as one prompt.

## 7. Later phases

Done 2026-10-19: F2 Edit / Write tool calls show a red/green diff when expanded, F3 search
(system find bar from the magnifier in the header), F4 a "Feed" button over the terminal when an
agent runs in the pane. F5 pinch to change text size done (2026-10-19, simulator build only; factor 0.75-1.8 kept in `feedTextScale`). Syntax colours in Edit/Write diffs done (same highlighter and cache as code blocks; simulator build only).

## 8. Review notes

- Risk: polling reads the whole transcript every 3 s. Accepted for F1 because the core caps the
  size; an offset-based tail in the core is the F2 fix if it shows up in battery or latency.
- Bracketed paste is honoured by Claude Code; Antigravity is unverified. Multi-line text is sent
  as one bracketed paste for every agent; if a device run shows Antigravity splitting it, the
  fallback is a per-agent setting, not a guess made now.
- Rev (doc, self-review): purpose, scope, security and failure paths are stated; no core change,
  so rust-core-dev and swift-bridge-dev steps are empty for F1. Open point: the key row is
  limited to four keys on purpose (the terminal keeps the full set).

## 9. Feed from the tmux screen (built 2026-10-19)

Owner: the chat is a transcription of the tmux data, formatted and folded, not a reading of the
agent's session file; thinking stays, in its place, folded.

What the pane gives (checked on real captures): with Claude Code in its normal renderer
(`/tui default`; the fullscreen renderer uses the alternate screen and tmux keeps no history,
`history_size 0`) and with Antigravity CLI (normal buffer), `tmux capture-pane -p -J -S -` returns
the whole conversation up to `/clear`. Claude's screen has no thinking and folds tool calls
("Ran 1 shell command"); agy's screen has a "○ Thought for 6m (title…)" line and folded tool
groups ("● Ran 5 commands (…)"). `-J` does not join lines the agent wrapped itself, so the parser
reflows paragraphs.

Built:
- `crates/core/src/screen.rs`: `parse_screen_feed(text)` -> `ScreenFeed {dialect, turns, status,
  draft}`. One turn per screen element in screen order (your input, reply text, tool call, folded
  tool group, thinking line, service line); the input box, status bar and the spinner are `draft`
  and `status`; unknown text is a folded "Terminal output" turn. Tests on excerpts of real captures.
- `merge_thinking(screen, file)`: Claude's thinking comes from the session file and is inserted
  right before the reply text or tool call it preceded (matched by text / tool name + start of the
  arguments, moving forward only), so the narrative order is kept. Thinking that cannot be placed
  is dropped, never put at the end. The file is read at most every 15 s.
- `capture_session_screen(session, known_sum)`: one host command; the host compares a checksum and
  answers `SAME` when the screen did not change, so a quiet screen costs no traffic.
- App: the poll tries the screen first and falls back to the file when nothing is recognised (plain
  shell). The live line of the agent is shown above the input.

Known limits: the file (thinking, full tool arguments) covers its last 500 lines; Claude's folded
tool groups carry no arguments; agy thinking is the title line only. Terminal width at the time a
line was printed decides the wrapping, the reflow is a heuristic (paragraphs whose lines reach the
widest line are joined; lists, tables, headings and code are not).

## 10. Attachments from the feed (built 2026-10-19/20)

A paperclip left of the message field: **Photos & videos** (system picker, out of process, up to 6) and **Files** (document picker). After picking, the app asks **"Keep on the host?"**: *Keep permanently* stores the file as `~/.ssh-app/inbox/<8 hex>-<name>` (stays until deleted); *Temporary* stores it in `${TMPDIR:-/tmp}/ssh-app-<uid>/` on the host, which the system empties. Each item goes over a new channel of the tab's SSH session (`upload_attachment`), file mode 0600, folder 0700 (created with `umask 077`; the folder must be a real directory owned by the user, never a link: refused otherwise), name reduced to `[A-Za-z0-9._-]` (80 chars), at most 200 MB, written to a `.part` file and moved in. The absolute host path is put into the message at its end. A chip shows progress and failures. Tests: name sanitising, command + path parsing, the real `sh` command with a temporary `HOME` / `TMPDIR` (content, mode 0600, folder 0700, no leftover `.part`, a symlinked folder is refused and nothing lands behind it). The app never deletes the permanent files. Not yet run on a device.
