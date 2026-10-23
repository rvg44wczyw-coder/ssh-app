# Feature 55: Event feed (incremental updates instead of re-sending the whole screen)

- **Status**: Draft 2026-10-23 (owner idea; follows `docs/architecture/performance-audit-2026-10-23.md`, bottlenecks 1, 2, 3, 6, 7). Not reviewed yet.
- **Agents**: Claude Code first (its session file is already JSON lines). Antigravity: studied 2026-10-23, see §8; events are possible through its SQLite store, but only after a spike.

## 1. Purpose

Today every change makes the app fetch the whole pane history (up to megabytes), parse it, rebuild all turns and compare them. The owner's idea: send **events** (JSON, one per change) and add the newest message to the feed without redrawing the rest. The agents already write events: every line of the Claude session file is one JSON record, and the app only needs the new ones.

## 2. Model

Two independent streams, both as small events with a sequence number:

| Stream | Source | Events | Carries |
|---|---|---|---|
| **Turns** | the agent's session file (JSONL), read from a saved byte offset | `append` (new turns), `update` (a turn whose tool call got its result or whose text grew), `reset` (file replaced or shorter) | typed turns with stable ids |
| **Live** | the pane (the last ~60 lines only) | `live` with `status`, `draft`, `prompt`, `dialog`, `aside` — sent only when they differ from the last one | what the screen shows and the file does not (spinner, input box, questions, `/btw`, dialogs) |

The feed on the phone is `turns` (from events, keyed by id) plus the live part. Older history stays on the host until the user scrolls up (`history(before: id, n)`).

```
host helper (long-lived exec, NDJSON)        core (Rust)                  Swift
 file offset + inode ──► {"seq":41,"ev":"append","turns":[…]} ─► FeedState.apply ─► @Published change set
 pane tail checksum  ──► {"seq":42,"ev":"live","status":…}    ─► FeedState.apply ─► only the live card
```

## 3. Behaviour

- **Cursor**: the app keeps `(inode, offset, seq)` per session. On start and after every foreground return it sends the cursor; the host answers with the missed events as one batch (`since`), then continues streaming while the connection lives. Background: the socket is closed as today (zero battery drain); nothing runs on the host without a phone session.
- **Gap or mismatch** (seq jump, inode change, offset beyond the file): the host sends `reset` with the last N turns; the app replaces its list once. Same path when the file is rotated or `/clear` starts a new file.
- **Update by id**: the app replaces one row by id; SwiftUI re-renders that row only. Appends add rows at the end; the scroll position is kept unless the user is at the bottom.
- **Live part** is polled **on the host** (local, no radio) every 300 ms while a phone is connected; an event is sent only on change. This replaces the 350 ms stability loop of the phone.
- **Fallback**: a host without the helper, an agent without a file, or a protocol mismatch uses today's screen path unchanged. The helper's `hello` caps list `"feed-events"`.
- **Caps**: one event ≤ 256 KB, one turn text ≤ 100 KB (longer is cut with a mark), ≤ 200 turns per `append`, ≤ 50 events per second (coalesced). The helper reads only the transcript path it detected itself for the session (never a path given by the app) and refuses symlinks outside `~/.claude/projects`.

## 4. Interface (sketch)

```rust
// host helper: `ssh-app-host feed --session <tmux name> [--since <inode>:<offset>:<seq>]` → NDJSON on stdout
pub enum FeedEvent {
    Append { seq: u64, turns: Vec<TranscriptTurn>, cursor: Cursor },
    Update { seq: u64, turn: TranscriptTurn, cursor: Cursor },
    Reset  { seq: u64, turns: Vec<TranscriptTurn>, cursor: Cursor },
    Live   { seq: u64, live: LiveState },
}
pub struct Cursor { pub inode: u64, pub offset: u64 }
// core: `FeedState::apply(event) -> FeedChange { added: Vec<Id>, updated: Vec<Id>, reset: bool }` (pure, tested)
// UniFFI: `watch_feed(session, cursor, listener)`, `FeedListener::on_change(change, turns_by_id)`
```

Parsing of file lines stays in `ssh-core` (`parse_transcript_jsonl`, 16 ms for 3.8 MB today); the helper may send raw new JSON lines and let the core parse them, which keeps one parser.

## 5. Constraints and invariants

- Events are untrusted display data on the phone (same caps and control-character stripping as the screen path).
- Idempotent apply: the same event twice (after a reconnect) changes nothing; out-of-order events are rejected by `seq`.
- Sequence of events equals the full read: a test replays a real session file as events and checks that the final turns equal `parse_transcript_jsonl` of the whole file.
- Tests never touch the real tmux (private sockets) and never read the owner's real session files, only fixtures.
- Battery: no timers on the phone for this feed while the stream is open; the helper stops with the connection.

## 6. Plan (dev-chain)

1. **Phase 1 (host read, no protocol change)**: `read_remote_transcript` takes `(inode, offset)` and returns only new bytes; `stat` guard for background tabs (bottleneck 1). Core keeps the parsed turns and appends.
2. **Phase 2**: one host script for the live tail: stable frame, tail only, plain `sh` (bottlenecks 2–4).
3. **Phase 3**: the streaming helper command `feed`, `FeedState`, `watch_feed`, events to Swift.
4. **Phase 4**: SwiftUI keyed by id, change sets instead of whole-array replace; measure with the in-app timing label before and after.
`doc-dev` → `rev` → `rust-core-dev` → `rev` → `swift-bridge-dev` → `rev` → `ios-ui-dev` → `rev` → `hacker` → merge, per phase.

## 7. Open questions for the owner

- Claude's feed is built from the screen today (it shows tool groups, system lines and `/btw` that the file lacks). With events, the file becomes the source of turns and the screen only the live part. Some screen-only items would need to be rebuilt from the live stream. Phase 1–2 keep the screen as the source, so nothing regresses; phase 3 decides per item.

## 8. Antigravity (agy) — what its storage looks like (studied 2026-10-23, read-only, on a copy)

- agy (CLI) writes **no JSONL transcript**. Each conversation is a SQLite database `~/.gemini/antigravity-cli/conversations/<uuid>.db` (journal mode WAL, 31 files, 0.5–60 MB). The app's agy feed is therefore built from the screen only, and the thinking comes from the screen too.
- Tables: `steps` (`idx` integer primary key, `step_type`, `status`, `step_payload` blob, `metadata` blob, `render_info`, …), `gen_metadata`, `executor_metadata`, `trajectory_meta`, `parent_references`. A 282-step conversation was 4.7 MB.
- **Append-only by `idx`**: the natural cursor is `max(idx)`. Status is a column (3 = done; one other value was seen on the last step), so a step that is still running changes in place; the query `WHERE idx > :cursor OR status <> 3` (indexed) gives exactly the `append` and `update` events. Cost: microseconds, no file tail.
- Payloads are **protobuf** with no schema shipped. By decoding the wire format (no schema): field 1 = step type (14 user input with the text at 19.2, 15 model step, 132 tool call with the tool name `run_command`, its call id and JSON arguments at 5.4, 101 task notification), field 5 = timestamps and ids (conversation id, step number). Texts of model steps and thinking sit in nested fields (20.x, 140.x) that are not named anywhere.
- **Consequences**
  - Good: a tail of steps is tiny and ordered; thinking, tool calls with results and timestamps exist in structured form (better than the screen, which only shows "Ran 5 commands (…)").
  - Risk: the format is private and unversioned; a field renumbering in an agy update silently breaks the decoder. Mitigation: decode only a few well-tested fields, treat anything unknown as "unrecognised step → fall back to the screen path for that turn", a fixture per agy version, and `hello` reports `agy-events` only when the decoder recognises the database's `user_version`/step shapes.
  - The helper needs SQLite: `rusqlite` (bundled) in `ssh-app-host`, opened **read-only** (WAL needs the `-shm` file; never write, never checkpoint). The system `sqlite3` binary is not assumed to exist on Linux hosts.
  - Which database belongs to the tab: the one the agy process of the pane has open (as in the current detection), never "the newest file".
- **Plan for agy** (after Claude phases 1–3): spike with a decoder over fixtures taken from real conversations (sanitised), compare its turns with the screen feed on the same conversation, and only then switch agy per item. The live part (status, draft, prompts) stays screen-based for both agents.
