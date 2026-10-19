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

F2 diffs inside tool cards (reuse the diff viewer), F3 search in the feed, F4 detected agent
from a plain session switches the tab to the feed offer, F5 pinch to change text size.

## 8. Review notes

- Risk: polling reads the whole transcript every 3 s. Accepted for F1 because the core caps the
  size; an offset-based tail in the core is the F2 fix if it shows up in battery or latency.
- Bracketed paste is honoured by Claude Code; Antigravity is unverified. Multi-line text is sent
  as one bracketed paste for every agent; if a device run shows Antigravity splitting it, the
  fallback is a per-agent setting, not a guess made now.
- Rev (doc, self-review): purpose, scope, security and failure paths are stated; no core change,
  so rust-core-dev and swift-bridge-dev steps are empty for F1. Open point: the key row is
  limited to four keys on purpose (the terminal keeps the full set).
