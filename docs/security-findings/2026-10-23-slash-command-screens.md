# Review and adversarial pass: /btw aside and slash-command screens (feature 52), 2026-10-23

Scope: `crates/core/src/screen/dialog.rs` and its hook in `parse_screen_feed`, `TerminalSessionViewModel` (`runAgentCommand`, `sendDialogKeys`, `closeOverlay`), `CommandScreens.swift`. Live tests: the parser run on the eight real Claude Code 2.1.308 captures (btw pending/answered, model, permissions, config, mcp, status, idle) with the right result for each; unit tests for a permission question (stays a prompt), an unknown screen with an `Esc to cancel` footer (not a dialog), the idle input box, and 50 rows of 5000-character labels (capped). No real tmux was used by the new tests.

## Findings

1. **[InputValidation] none open.** Everything read from the pane is display text: lengths capped (row texts 200, answer 20 000, rows 200, dialog looked for only in the last 60 lines).
2. **[Injection] none open.** The app sends only fixed key sequences (arrows, Enter, `s`, PageDown, Esc); a row's text is never sent. `runAgentCommand` accepts only `/` plus letters and hyphens and is called with fixed strings.
3. **[Spoofing, Low, accepted]** Program output that imitates a dialog (a title such as "Select model" under a rule and an `Esc to cancel` footer) would be drawn as a native screen; its taps send arrows and Enter to the pane the person is already looking at. Nothing is executed by the app; the effect equals the person pressing those keys in the terminal.
4. **[Quality, fixed in this pass]** The `/btw` answer is model output and was rendered as Markdown with tappable links; links are now stripped.
5. **Design note:** a known dialog is recognised before the prompt parser runs, so the `/mcp` footer ("Enter to confirm") no longer produces a generic prompt card; a real permission question is unaffected (verified by test).

## Verdict

Clean for Critical/High/Medium; one Low accepted.
