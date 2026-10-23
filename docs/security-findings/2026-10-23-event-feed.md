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

## Addendum: Antigravity from its database (rev + hacker, 2026-10-23)

Scope: `crates/host/src/agyfeed.rs` (decoder, `--raw` modes), `feed_stream.rs::agy_stream_script`, `feed_read::is_agy_path`, `host_api/feed_watch.rs::watch_agy_feed`, Swift hookup.

Attack classes and what was done:

- **InputValidation / injection** — the folder reaches SQL only after `workspace_uri` (absolute, plain characters `[A-Za-z0-9/._- ]`, no `..`, ≤1024); conversation ids pass `valid_uuid` before they become a file name; `from` is a number. Live: the real helper was run with `/a'b`, `/tmp/../etc`, `../../etc/passwd`, `abc`, a 20-digit offset and missing arguments (`--raw`): every call printed nothing and exited non-zero. The script additionally refuses a conversation id that is not `[0-9a-f-]`.
- **Sandbox / privilege** — the script runs only the pinned helper (`-f`, not a symlink, SHA-256 equal to the app's pin); without it the stream says `bye nohelper` and the app reads the session file as before. The helper opens agy's databases with `sqlite3 -readonly` and never writes.
- **DoS** — protobuf walker is bounded (4096 fields, 400 KB blobs, 200 000 chars of text), one answer ≤ 1 MB, the reader never blocks on a running step; the phone side keeps the 8 MB frame cap and the 3000-turn window. A hostile database yields truncated text, not a crash (test `hostile_payloads_do_not_panic_or_blow_up`).
- **Path** — the virtual path `agy:<uuid>` is accepted by the phone only in that exact shape (`is_agy_path`); any other path still goes through `validate_transcript_path`.
- **MetadataLeak** — nothing new is sent to push/relay; the conversation text travels only over the existing SSH channel.

Findings: none open. Notes (Low, accepted): (1) the folder is the pane's current directory, so an agy started in another folder than the pane's is not found and the file fallback is used; (2) a conversation reset (new id in the same folder) restarts the feed from the first step, which the phone shows as a reset.

## Addendum: subagents (rev + hacker, 2026-10-23)

Scope: `crates/core/src/subagent_feed.rs`, `FeedState::new_subagent`, `TranscriptParser::subagent`, `list_subagents` / `read_subagent_since`, `SubagentsSheetView.swift`.

- **PathTraversal** — the phone builds the file as `<session>/subagents/agent-<id>.jsonl` itself; `<id>` must match `^a[0-9a-f]{8,40}$` (tests: `..`, `/`, newline, upper case, too long) and both paths pass `validate_transcript_path`. The host's list script accepts only `*.jsonl` session files outside the blocked folders, does not follow a `subagents` folder or a meta file that is a link (live test with links to a file/folder elsewhere: nothing listed) and prints only ids of that shape, which the phone checks again. The cursor read resolves and re-checks every file as for the main feed.
- **InputValidation** — meta files are read up to 4 KB, parsed as JSON, text fields lose control characters and are cut at 300 characters; `toolUseId` must be `[A-Za-z0-9_-]`; depth > 3, a 21st agent, duplicates and non-JSON are dropped (tests).
- **DoS** — at most 20 agents are listed; the sub-feed reads by cursor with the same caps as the main feed (8 MB chunk, 3000 turns); the screen polls every 3 s **only while it is open** (the task ends with the view), nothing runs for background tabs.
- **Trust** — a subagent's text is display data only; nothing from it is executed or written. Its approvals stay in the main pane.
- Real data (read-only, in memory, three subagent files of the owner's `~/.claude`): 143–287 lines → 77–106 turns, every tool call linked to its result, the worst 32 KB chunk 2 ms; the same files read as a main session give 0 turns (side chain left out).

Findings: none open. Notes (Low, accepted): (1) "running" is "its file grew in the last 60 s" by the host's clock, so a long silent tool call shows as finished until the next line; (2) Antigravity child conversations (`parent_conversation_id`) are not shown yet: the helper's `agy-conv` takes the top-level conversation only.

## Addendum: Antigravity child conversations (rev + hacker, 2026-10-23)

Scope: helper `agy-children`, `agy_list_script` / `agy_read_script` / `parse_agy_children` in `subagent_feed.rs`, Swift hookup.

- The folder is validated as for `agy-conv`; ids in SQL come from the index and pass `valid_uuid` before use; title/agent text travels hex-encoded (tabs and line breaks cannot break the rows), is cleaned of control characters and cut at 300. Depth ≤ 3 (recursive query), killed conversations and the 21st child are left out; verified on a synthetic index (`children_of_the_folders_conversation_are_listed_with_depth_and_caps`) and on the owner's real index (read only: children found with depth 1).
- The read script runs only the pinned helper (digest compared, not a link); the conversation id given to the one-shot read is checked on the phone (`8-4-4-4-12` lower-case hex) before it reaches the command line, so `../x` and `x; rm -rf /` are refused (tests).
- A real child conversation (10 steps) decodes to 10 records.
- Findings: none open. Note (Low): the parent is the conversation of the pane's folder, same limitation as the main agy stream.
