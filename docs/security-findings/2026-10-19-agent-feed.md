# Agent feed (feature 31, phase F1) — security pass

Scope: `FeedSubmit.bytes`, `FeedMarkdownView`, live tail in `TerminalSessionViewModel`. Code
analysis only; no live attack run (no new host command or network surface, iOS-only change).

1. **Bracketed-paste escape (InputValidation, would be Medium)** — text containing `ESC[201~`
   could close the paste early so the rest runs as keystrokes. Mitigated: both paste markers are
   removed from user text before wrapping (`FeedSubmit`). Covered by `AgentFeedTests`.
2. **Untrusted transcript text (InputValidation, Low)** — rendered as inline Markdown only; no
   HTML, no images; links are not opened automatically (taps use the system's handling).
3. **Request rate (DoS/battery, Low)** — live tail reloads every 3 s only while the view is
   visible; transcript lookup (a host command) at most every 20 s. Reads are size-capped in the
   core (`read_remote_transcript`).
4. **Control keys row (Low)** — fixed bytes (Esc, Ctrl-C, Up, Enter) into the user's own pane.

Verdict: clean (no Medium or higher open). Unverified on device: Antigravity's handling of
bracketed paste.
