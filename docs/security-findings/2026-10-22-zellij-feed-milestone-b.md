# Security review: Zellij agent feed (feature 49, milestone B)

Date: 2026-10-22. Branch `rust-core-dev/zellij-feed` at `9e2fa87`. Reviewer: `hacker` pass of the dev-chain.

## 1. Scope

Reviewed: `crates/core/src/zellij_feed.rs`, `host_api/{transcript,approvals,session_io}.rs`, `pane_context.rs`, `multiplexer.rs` (name rules), `crates/host/src/{screenwatch,sessions}.rs`, `scripts/agent-hooks/ssh-app-hook.py`.

Applied: InputValidation / command injection (names into shell and argv), DoS (screen size, scan cost), MetadataLeak (Zellij error text, environment of other processes), spoofing of the registry and of the pane-process scan.
Ruled out: cryptography, authentication, replay, downgrade, key derivation, timing, randomness (nothing in the diff), MITM (no new network protocol; the SSH exec path is unchanged), path traversal (no path is built from input).

Live tests (own processes, private Zellij socket folders, no tmux, no real sessions touched):
- 11 hostile names (quotes, `$(...)`, backticks, newline, `-h`, `--help`, `--version`, `--config-dir`, `--data-dir`) fed straight into the capture script, bypassing the Rust validator: no command ran (marker file not created), every call failed closed.
- 200 000 lines flooded into a pane: capture was 842 KB in 0.11 s (Zellij caps the scrollback), far below the 3 MB bound.
- Process scan over 1 146 processes: 0.1 s.
- 13 hostile `ZELLIJ_SESSION_NAME` values through the real hook script: only valid names (letters, digits, `_`, `-`, up to 64) were registered; quotes, newlines, non-ASCII digits, `../`, JSON text, 65 characters were all dropped.
- Zellij's "not found" text (it lists other sessions) never reaches the app (loopback test `the_zellij_feed_reads_the_focused_pane_through_the_session_handle`).

## 2. Findings

1. **InputValidation, Low.** `scripts/agent-hooks/ssh-app-hook.py` (`zellij_session_name`), `crates/host/src/sessions.rs` (`zellij_name`) and `crates/host/src/screenwatch.rs` (`capture_zellij`) accept a name that starts with `-`; the core validator (`validate_session_name`) refuses it. Scenario: a pane whose `ZELLIJ_SESSION_NAME` is `-x` registers that name and the host later runs `zellij --session -x action ...`. Verified: clap rejects a dash value (`unexpected argument '-x'`), so nothing runs; defence in depth only. Fix direction: refuse a leading `-` in all three, as the core does.
2. **InputValidation, Low (inherent, same user).** On macOS the scan reads `ps -Eww`, where the environment follows the command line in one string, so a process whose *arguments* contain the tokens `ZELLIJ_SESSION_NAME=<n>` and `ZELLIJ_PANE_ID=<k>` is taken for a process of that pane. Effect: a user's own process can make the feed believe an agent runs in a pane. No privilege boundary is crossed (same user, same host); the registry stays first for the transcript. Fix direction: none cheap on macOS; the Linux `/proc/<pid>/environ` branch is exact.

## 3. Verdict

Findings, highest severity Low.
