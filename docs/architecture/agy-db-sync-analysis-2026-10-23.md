# Antigravity (agy) database sync: analysis (2026-10-23)

Study of `~/.gemini/antigravity-cli` on the owner's Mac (agy CLI, 31 conversations, one running), read-only, on copies that were deleted afterwards. Payloads were decoded as raw protobuf (no schema); nothing from the owner's conversations is stored in the repo. Goal: can the app sync a conversation from agy's database instead of reading the screen, so that the event feed (`docs/features/55-event-feed.md`) works for agy too?

## 1. Verdict

**Yes, feasible, and better than the screen path** (full thinking, full tool output, exact status, hook decisions), at the price of a private, unversioned format. Recommended: a read-only decoder in the host helper behind a capability flag, with the screen path as fallback per item. Estimated gain: the step payload is 0.8–10 KB of which the useful part is typically 100–5 000 characters; reading new steps is one indexed query.

## 2. Where things are

| What | Where | Use |
|---|---|---|
| One conversation | `conversations/<uuid>.db` (SQLite, WAL, `user_version` 1, 0.4–60 MB) | steps |
| Index of all conversations | `conversation_summaries.db`, table `conversation_summaries` | find the tab's conversation, cheap change detection |
| History of typed inputs | `history.jsonl` (`display`, `workspace`, `conversationId`, `type`) | cross-check |
| Files the agent made | `brain/<uuid>/…` | not needed |

**The agy process does not keep its conversation database open** (checked with `lsof` on two running agy processes: only the folder and the summaries database are open). The current idea "the one the process has open" does not work for the CLI. Instead:

`conversation_summaries` has `conversation_id`, `title`, `preview`, `step_count`, `last_modified_time`, `workspace_uris` (JSON list of `file://` folders), `status` (`CASCADE_RUN_STATUS_RUNNING` / `_IDLE`), `not_fully_idle`, `killed`, `last_user_input_time`, `parent_conversation_id`, `nesting_depth`. A tab maps to the row whose `workspace_uris` equals the pane's current folder, not killed, newest `last_modified_time`; when several agy run in one folder the screen decides (the last user message on the screen equals `preview`/the last user step, as `transcript_matches_screen` does for Claude). `step_count` and `status` tell whether anything changed **without opening the conversation database** at all.

## 3. Steps table

`steps(idx PRIMARY KEY, step_type, status, has_subtrajectory, metadata, error_details, permissions, task_details, render_info, step_payload, step_format)`; index on `status` and `step_type`. `idx` grows by one per step and rows are not deleted: the cursor is `max(idx)`.

| `step_type` | Meaning | Fields (protobuf path in `step_payload`) |
|---|---|---|
| 14 | user input | text `19.2` (also `19.3.1`); the rest of the payload (≈ 3 KB) is the context: skills, tools, workspace |
| 15 | model step | thinking `20.3` (English), answer `20.1` (repeated at `20.8`), tool calls `20.7.x` (JSON arguments) |
| 132 | tool execution | tool name `5.4.2`, arguments JSON `5.4.3`, short description `140.1.2`, **full output `140.2.1`** ("Command exited with code 0 …"), type url `140.2.6.1` (`…tools.RunCommand`) |
| 101 | task notification | `114.x` (title, type `task_notification`) |
| 17 | other (seen twice) | not decoded |
| all | ids, time | step id `5.35`, conversation ids `5.20.1` / `5.20.4`, timestamps `5.1.1` seconds + `5.1.2` nanos; the app's own hook decision text appears at `5.34.5` ("Approved from ssh-app", "ssh-app broker unavailable…") |

`status` column: **3 done, 9 running or waiting (for example a command waiting for approval), 7 failed or denied**; `error_details` then holds the text ("tool call denied by pre-tool hook: …", "429 Too Many Requests"). A step changes in place from 9 to 3 or 7, so the sync query is

```sql
SELECT idx, step_type, status, step_payload, error_details FROM steps WHERE idx > :cursor OR status NOT IN (3, 7) ORDER BY idx;
```
(append for new `idx`, update for known `idx` whose status or payload changed).

## 4. What the screen cannot give and the database does

- full thinking text (the screen shows "Thought for 5s" folds) and the whole tool output (the screen folds "Ran 5 commands");
- exact status of a step and a failure reason, timestamps, the answer text without terminal wrapping or table redrawing;
- the hook decisions of this app (`5.34.5`), a source for the approval history.
What stays on the screen: the input box draft, the spinner line, permission questions, `/` menus (live part).

## 5. Risks and how to contain them

1. **Private format.** Field numbers could change with an agy update. Mitigation: decode only the paths in the table; an unknown shape gives "unrecognised step" and that item falls back to the screen; one sanitised fixture per agy version; capability `agy-events` is announced only if `user_version` is 1 and the first and last steps decode.
2. **Concurrent writer.** WAL allows a read-only reader while agy writes; the helper opens with `SQLITE_OPEN_READONLY`, never writes, never checkpoints, and treats `SQLITE_BUSY` as "try later". The database must be copied-free and not on a network share.
3. **Size and privacy.** Each payload carries the context (skills, tool lists, file paths). The helper extracts the fields above and **never sends the raw payload**; text caps as in feature 55; control characters stripped on the phone.
4. **SQLite in the helper.** `rusqlite` with the bundled C library, feature-gated; the pinned helper grows by about 1 MB. The system `sqlite3` binary is not assumed.
5. **Subagents.** `parent_conversation_id` / `nesting_depth` exist. Owner decision 2026-10-23: subagents are in the release as a separate item (`docs/features/55-event-feed.md` §9): a child conversation is read like the parent and shown as a sub-feed linked from the parent's tool call.
6. **Hosts.** Only hosts where agy runs and the helper is installed (macOS and Linux). Windows/WSL unverified.
7. **Path safety.** The helper reads only `~/.gemini/antigravity-cli/{conversation_summaries.db,conversations/<uuid>.db}` where `<uuid>` is a canonical UUID taken from the summaries row, resolved to a real path inside that folder, never a path from the app.

## 6. Open questions to verify with live runs (private tmux socket, a throwaway folder)

- Whether `20.1` is always the whole final answer or sometimes a partial text while streaming (the running step 9 case).
- Meaning of `step_type` 17 and of `has_subtrajectory`.
- Whether an interrupted turn (Esc) leaves a step in status 9 forever.
- Whether `/clear` or `/compact` starts a new conversation id or appends a checkpoint step.
- Behaviour on an agy update: keep the fixtures and rerun the decoder tests when `agy --version` changes.

## 7. Plan

1. Spike in `crates/host`: `agy_feed` module (read-only SQLite, summaries lookup, the query above, field extraction) with tests on **synthetic databases built by the test** with the same schema, plus a sanitised real fixture; a replay test compares the events with a full read.
2. Map steps to `TranscriptTurn` (user, assistant with thinking and text, tool call with result) in `ssh-core` so the same `FeedState` works; compare against the screen feed on a live conversation in a private tmux (agy asked in ask mode, answers given by hand).
3. Enable per item behind a setting; keep the screen path.
