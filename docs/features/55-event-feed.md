# Feature 55: Event feed (incremental updates instead of re-sending the whole screen)

- **Status**: Draft 2026-10-23 (owner idea; follows `docs/architecture/performance-audit-2026-10-23.md`, bottlenecks 1, 2, 3, 6, 7). Not reviewed yet.
- **Agents**: Claude Code first (its session file is already JSON lines); Antigravity keeps the screen path until its files are understood.

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
