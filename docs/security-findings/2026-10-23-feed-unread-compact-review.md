# Review and adversarial pass: feed window, hook passthrough, unread counter, compact layout (2026-10-23)

Scope: commits `2b218af`, `f32aa16`, `efac98d` and the follow-up edits: `host_api/transcript.rs` remote read command, `transcript/claude.rs` permission-mode turns, `hook.rs` / `ssh-app-hook.py` AskUserQuestion passthrough, unread counter, `feed_mode_*` defaults, `FloatingQuickMenu`. Live tests: the read command run locally on a crafted JSONL (5000 noise lines, NUL and invalid UTF-8 inside a record, file name `a'b"; touch PWNED; echo.jsonl`); hook tests in temp homes. Not touched: real tmux, real Docker.

## Findings

1. **[DoS/efficiency, Medium] Fixed.** The read command ran `grep` over the whole session file (192 MB seen) on every poll (15 s per open tab, 30 s per background tab). Now `tail -c 32000000` first; malformed first line is skipped by the parser (`jsonl.rs`).
2. **[InputValidation, Low] Fixed.** Without `-a`/`LC_ALL=C`, `grep` may answer "Binary file matches" for a record with NUL or invalid UTF-8 and drop the whole transcript. Verified live: with `LC_ALL=C grep -a` the record passes, noise is removed, the odd file name executed nothing (no `PWNED` file created).
3. **[Quality, Low] Fixed.** `feed_mode_<id>` and `chat_last_opened_<id>` stayed in the defaults after a tab was closed; now removed in `closeSession`.
4. **[Quality, Low] Fixed.** The Python hook had no test for the AskUserQuestion passthrough (only the Rust one); added `a_question_of_claude_gets_no_phone_popup_in_the_python_hook_either`.
5. **[Info] Passthrough is fail-open by design:** no output means Claude asks in its own terminal dialog; the tool name comes from the agent, which cannot gain anything (nothing is approved). Verified by both tests (queue stays empty). A push for the question still comes from the passive Notification event.
6. **[Controversial, kept]** The terminal reports one column less to the host. It is a workaround kept next to the real fix (layout width); the owner verified the two together. Comment in `TerminalContainerView.swift` states this.
7. **Ruled out:** crypto, key handling, MITM, replay, downgrade (no protocol change); `FloatingQuickMenu` sends only fixed key bytes, never row text; its position is stored as a fraction and clamped (cannot end up off screen after rotation); permission-mode value is length- and charset-validated.

## Verdict

Findings (highest: Medium), all fixed; re-verified: `cargo clippy -D warnings`, `ssh-core --lib` 415 pass, `hook_script` 43 pass.
