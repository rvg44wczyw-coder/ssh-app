# Review and security pass: event feed (phases 1–4a), 2026-10-23

Scope: `crates/core/src/{feed,feed_read,feed_stream,screen_capture}.rs`, `host_api/{feed_watch,transcript}.rs`, `transcript/jsonl.rs` (incremental parser), the Swift stream wiring in `TerminalSessionViewModel` / `StructuredTranscriptView`. Local only: private tmux sockets, throwaway files, loopback sshd; the user's tmux (pid 35350) was compared before and after.

## rev
- Incremental parser: `parse_transcript_jsonl` is now the same state machine fed line by line, so chunked reads equal a whole-file parse by construction. Tests: every byte cut of a fixture, line-by-line, 7-character chunks, and a **real 3.8 MB session** replayed in 32 KB chunks (equal; worst chunk 0.22 ms).
- Cursor read: complete lines only, `stat` before reading, replace/shrink gives a reset, first read keeps the usual window. Tested on a real `sh` with partial lines, a half multi-byte character, rotation, shrink, missing file.
- Stream: JSON header line plus a byte-exact body; the frame parser gives the same events at any chunking (sizes 1…whole); unknown events are skipped; sizes capped (header 4 KB, body 8 MB). Loopback sshd test: first read, live pane text, append, stop, resume from the cursor without a repeat, bad path refused, compressed capture and "same".
- Capture: one script waits for a stable frame on the host and sends gzip+base64 (7× smaller on repeated text); inflate is bounded; the old `SUM` format is still read; the login shell is a fallback when plain `sh` does not find tmux.
- Deviations from the draft spec: the transcript path is still found by the existing `detect_session_transcript_path` and passed by the app (validated, then the resolved path is checked again), not detected inside the stream; the stream is a `sh` loop, not a helper command (no new binary to pin); applying the same chunk twice is **not** idempotent (the host cursor and `reset` prevent repeats; documented in a test).

## hacker (attacker model: hostile file content, hostile names, hostile host output)
1. **Link into a credential folder** (Low, fixed): the cursor read resolved the path and sent the data before the app compared the resolved path with its blocklist, so a symlink in an allowed folder pointing into `~/.ssh` crossed the wire (then dropped). The script now refuses blocked folders itself (`case` on the resolved path, same list as `validate_transcript_path`). Test: link to a file in a `.ssh` folder gives `NOFILE` and the secret never appears.
2. **Shell injection** through session name or path: three variants (`;`, `$()`, backticks) run against the real script with a marker file; nothing executed. Names pass `mux_arg` rules, paths pass `validate_transcript_path`, and both are quoted and only used as `"$1"`/`"$2"`.
3. **Hostile file content**: 200 000-level nesting, a 2 MB record, ANSI/bidi characters, binary noise: no crash, escapes and overrides stripped, memory bounded by the chunk (8 MB) and pending-line (8 MB) caps and the 3 000-turn window.
4. **Hostile host output**: not JSON, oversized headers or bodies, missing newline after a body, a resolved path in `/etc`, non-numeric cursor: all refused.
5. **Orphans**: when the reader goes away the loop ends at its next write (a change or the quiet-time ping every ~15 s); the production wrapper also kills it on channel close. No processes or temp files were left after the tests.
6. **Not a finding but a limit**: a malicious host can flood the phone with events up to the caps; the host is the user's own machine and already trusted for the terminal.
Verdict: one Low found and fixed; clean after the fix.

## Controversial
- The quiet-time ping period (15 s) is a compromise between noticing a dead reader and waking the radio; it only matters when the channel is half-open, because normal closes stop the loop immediately.
- On every event the phone still reads the whole (compressed) screen: the change set is used for the file part (thinking) and to avoid polling, not yet to splice only the new lines of the screen. Phase 4b (tail splice) is open and measured first.
