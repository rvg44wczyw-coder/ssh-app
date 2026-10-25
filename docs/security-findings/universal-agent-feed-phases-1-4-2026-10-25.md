# Universal agent feed, phases 1–4 (registry, session sources, dialects, Swift resolver): adversarial review

Date: 2026-10-25. Mode: standalone (working tree on `main`, uncommitted phases 1–4).

## 1. Scope

Reviewed: `crates/core/src/agent_registry.rs`, `session_source.rs` (the host locate script), `screen/dialect.rs`
(+ `screen/mod.rs`, `style.rs`), `host_api/{mod,transcript}.rs` (consumer of the script), and the Swift resolver
(`AgentTraits.swift`, VM descriptor, command menu).

Attack capabilities considered: a hostile text in the agent's pane (prompt injection, `cat` of a hostile file, a
hostile repository), a hostile process list entry or environment on the host, a hostile file in a project folder,
another tab's tmux name.

Live tests (all against local instances: `/bin/sh` running the real generated script with a fake `ps`/`lsof`, a
temporary `HOME` and a private PATH; a scratch crate outside the repo linking `ssh-core`; no tmux, sshd, agent or
network was started; tmux pid 35350 before and after):

| # | Test | Result |
|---|---|---|
| L1 | Command-injection payloads (`$(touch F)`, backticks, `"; touch F; "`, `'; touch F; '`) in process arguments, in the cwd reported by `lsof`, and in the session id of a hook note | no file created, no error; payloads only ever appear as data (control run proves the script reaches the Claude arm) |
| L2 | Session id with `..`, `/`, shell metacharacters in a hook note | rejected by the `case` filter, falls back to the newest file of the folder |
| L3 | 30,000 process rows; a ppid cycle; a 500 KB command line | 0.49 s / 0.37 s, depth capped at 40, no hang |
| L4 | Tab name with regex characters vs. another tab's hook note | **finding 1** |
| L5 | Agent's open files include a path with `/brain/` inside a hostile repo | **finding 2** |
| L6 | 1,042,790 mutated screens (15 fixtures mutated by insert/delete/truncate/repeat of ANSI, box drawing, markers, control and bidi characters, 90 s) through `parse_screen_feed`; plus 5 MB single line, 200k lines, 500k `ESC[` | 0 panics, worst case 1 ms, large inputs ≤ 0.05 s |
| L7 | Lines of one agent's markers placed at column 0 above a real screen of the other | **finding 3** |

Ruled out: cryptography, authentication, key derivation, replay, downgrade, timing, randomness (this change touches
none). Registry strings are compile-time constants; they reach the shell only through `is_safe_program_name` (added
by the review; id and hook names are checked by a unit test).

## 2. Findings

1. **[InputValidation / Low]** `session_source.rs` `MID_B`: `grep -q "\"tmux\":\"$1\""` takes the tab name as a
   regular expression, and the names the app allows include `.` (`validate_session_name`: `[a-zA-Z0-9_.-]`).
   Verified: a tab named `a.c` picked the hook note of the tab `abc` and located that tab's session file (a feed of
   another tab). Needs two tabs of the owner's own; effect is a wrong feed, not a leak across users. Fix direction:
   `grep -F` (and the same for the `agent` match), or compare the parsed field.
2. **[PathTraversal / Low]** `AGY_ARM`: with no hook id and no `/proc` (macOS), the session directory is the first
   `.../brain/<x>` found in the open files of the agent (`lsof`). A repository the agent opened that contains
   `brain/<x>/.system_generated/logs/transcript.jsonl` (verified) is returned as the session, so the feed renders
   text chosen by the repository's author as the agent's conversation. It only appears when the hook note is
   missing. Fix direction: accept only a directory under `$HOME/.gemini/antigravity-cli/brain/`; same prefix check in the
   `/proc` branch.
3. **[Spoofing / Low]** `screen/dialect.rs`: dialect detection is a sum of per-line scores over the whole pane, and
   the strongest rule (`Keyboard: ↑/↓`, 3 points) or a few `● ` lines at column 0 outvote the real footer. Verified:
   4 such lines above a Claude screen turn it into `Agy`; 8 `● ` lines too. Since phase 4 the descriptor (keys, command
   menu, `SessionStore`) follows the dialect, so a pane that prints such lines (a `cat` of a hostile file, prompt
   injection) switches a Claude tab to the Antigravity keys, menu and database feed source for as long as it is on
   screen. Behaviour is identical to before phase 4 (the differential test compares with the old detector), so this is
   not a regression. Keys are only sent on a tap of the user, and no key is destructive in either agent. Fix direction:
   score only the lines of the footer area, or give the tab's own type a weight and require the detected dialect to hold
   for several polls before it overrides the tab.

Informational (not findings): the transcript path reader accepts any path outside a denylist, not only the paths the
script prints (`transcript/path.rs`); consider also requiring the `.jsonl` suffix there. The output of the script is
re-validated (no newline, no control characters) before use.

## 3. Fixes and re-test (2026-10-25)

1. `grep -qF` for the tab name and the agent name in the hook notes. Re-test: tab `a.c` finds nothing, tab `abc` finds its file. Unit test `a_tab_name_is_not_a_pattern`.
2. The agy arm accepts a folder only under `$HOME/.gemini/antigravity-cli/brain/` (`awk` with a literal prefix, in the `/proc` and the `lsof` branches). Re-test: the repository's transcript is no longer offered, the agent's own is. Unit test `agy_ignores_a_brain_folder_outside_its_own`.
3. Two layers. Core: `detect` counts the bottom 10 lines in full and the lines above at most 2 per dialect; 250 printed marker lines above the fixtures of both agents no longer change the dialect (`printed_markers_do_not_flip_the_dialect`, 4 fixtures × 5 spoof kinds); all fixtures keep their dialect. App: `DialectTracker` needs two looks in a row to go from one recognised agent to another (unit test). Re-test with the live spoof harness: 12 of 12 screens keep their dialect; a second fuzz run (1.04 M cases) again 0 panics.

Left on purpose: text printed in the bottom 10 lines still counts in full; an attacker who controls the last lines of the pane
also controls what the user sees there, and the real footer is in that area. The frozen-script snapshot was regenerated.

## 4. Verdict

Clean after fixes (all three Low findings fixed and re-tested). No injection, no hang, no panic.
