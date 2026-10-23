# Performance audit: parser and the feed pipeline (2026-10-23)

Method: `cargo run --release -p ssh-core --example profile_parser` (synthetic Claude output: prompt, reply with list, tool call with results, box table, code block, summary; plain and with `-e` colour codes), `... -- <file.jsonl>` for a real session-file tail, and timing scripts on this Mac (the SSH host) inside a private tmux socket (the user's tmux was not touched: pid 35350 before and after). Numbers are one release build on an M-series Mac. **Not measured:** the Swift side (UniFFI decoding, SwiftUI layout) and the real network; the in-app timing label (`cap / file / cycle ms`) is the tool for those.

## 1. How fast is the parser?

| Screen | Lines | Size (styled) | `parse_screen_feed` plain | styled (`-e`, what the app sends) | `merge_thinking` | `transcript_matches_screen` |
|---|---|---|---|---|---|---|
| 5 turns | 220 | 10 KB | 0.15 ms | 0.38 ms | 0.06 ms | 0.05 ms |
| 20 turns | 865 | 36 KB | 0.23 ms | 0.58 ms | 0.09 ms | 0.09 ms |
| 100 turns | 4 305 | 179 KB | 1.0 ms | 2.8 ms | 0.45 ms | 0.46 ms |
| 400 turns | 17 205 | 720 KB | 4.1 ms | 11.3 ms | 1.9 ms | 1.8 ms |
| 1500 turns | 64 505 | 2.7 MB | 15 ms | 44 ms | 8.1 ms | 7.1 ms |

- Cost is linear in the size of the capture (≈ 60 MB/s styled), no quadratic path found; colour handling (`style::restyle_turns`) is about two thirds of the styled time.
- Session-file parser (`parse_transcript_jsonl`) on a real 3.8 MB tail (868 turns): **16 ms**.
- Conclusion: **the parsers are not the bottleneck.** A typical screen (a few hundred lines) parses in well under 1 ms; even a 17 000-line history is 11 ms. Everything below costs 10 to 1000 times more than parsing.

## 2. Bottlenecks of the architecture, worst first

| # | Where | Measured / reasoned cost | Why it hurts | Direction |
|---|---|---|---|---|
| 1 | Session-file read (`read_remote_transcript`): `tail -c 32000000 \| grep -aEv … \| tail -n 3000 \| tail -c 4000000` | **≈ 1.1 s of host CPU and 3.8 MB on the wire per read** (real 200 MB session file) | Run on every new turn on screen and every 15 s per open feed, **and every 30 s for every background tab** (`refreshUnreadOnce`). With 5 tabs: ~5.5 s of host CPU and 19 MB every 30 s, on cellular | Remember the file's size/inode and read only the new bytes (`tail -c +N`), or `stat` first and skip when unchanged; the unread counter and the thinking need only the last records; do it in the host helper |
| 2 | Screen capture `tmux capture-pane -e -p -J -S -` (whole history) every time the screen changed | 43–53 ms host for 20k lines vs 5 ms for the visible part; ≈ 1.7 MB per 20k lines on the wire, repeated every 3 s while the agent streams | Bandwidth and radio time grow with the age of the session; the unchanged case is cheap (`SAME`) but streaming is the common case | Capture only the tail (last N lines plus `#{history_size}`), keep older lines in the app; or gzip on the host; or a delta by line count |
| 3 | Stability loop in `reloadFromScreen`: up to 3 extra captures 350 ms apart after a change | up to ~1 s extra latency and up to 4× the bytes of #2 per poll | The feed lags behind and repeats the full transfer while the agent redraws | Let the host script do it (capture, sleep 150 ms, capture, compare) and return only the stable frame; or compare checksums only |
| 4 | Every exec is `$SHELL -l -c 'sh -c …'` with a watchdog | login shell 27 ms here vs 4 ms plain `sh`; a heavy `~/.zshrc` costs 200 ms or more per call; plus an SSH channel open/close round trip per call | A feed cycle is 2–5 execs; the fixed cost is paid each time. The capture script only needs `PATH`, which it sets itself | Capture and probes through plain `sh -c` (PATH is set in the script); keep the login shell only where the user's environment is needed; or one long-lived helper stream (`watch_host_agent` exists) instead of polling |
| 5 | `cksum` of the whole capture as change detector | part of the 53 ms above; 32-bit, so a change is missed with probability 2⁻³² per poll | Small, but it hashes megabytes to learn "nothing changed" | Hash the tail plus `#{history_size}`, or use tmux's `#{pane_...}` counters |
| 6 | Polling cadence in the feed: 3 s, then 5 s, then 10 s when quiet; 6 s attention watch; 3 s sessions-screen preview refresh (`TimelineView`) | each poll is an SSH exec + parse + layout | Radio and battery: it is the dominant cost on the phone, not CPU | Event-driven updates from the host (`tmux -C` control mode or `pipe-pane` into the helper) so the phone is told when something changed |
| 7 | Whole feed rebuilt and copied through UniFFI on every change (all turns, all tool calls) | not measured on device; Rust side clone/compare of 868 turns is 0.2 ms | Cost is in Swift decoding, `!=` over the array and SwiftUI diffing; grows with history | Send only the last N turns plus a count; cache turn views by id; measure with the in-app label first |
| 8 | Parsing the full history on each poll | 11 ms at 17k lines, 44 ms at 64k lines (styled) | Unbounded growth for very long sessions | Parse only lines after the last stable turn boundary; cache the parsed prefix by line hash. Low priority |
| 9 | Sessions screen: all cards re-render every 3 s; per-server runtime probe | small | Fine for a few sessions | Pause when the screen is not visible (already tied to the view) |

## 3. Proposed order of work (each through the dev-chain, tests on private sockets only)

1. **#1 incremental/guarded session-file read** (largest win, host and wire; also the cheapest: `stat` before reading).
2. **#3 + #4 + #2 one host script** that returns a stable, tail-limited frame through plain `sh` (cuts execs and bytes together).
3. **#6 event-driven updates** (bigger design change; needs its own spec).
4. **#7/#8** only if the in-app label shows Swift-side time above the network time.

No code was changed for the audit except the new example `crates/core/examples/profile_parser.rs`.
