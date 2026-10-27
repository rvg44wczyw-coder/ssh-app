# Background tasks (M48): review and security pass (2026-10-27)

## Scope

`crates/core/src/screen/background.rs`, the overlay hooks in `dialect.rs`, `AgentKeys` task keys in `agent_registry.rs`, and the iOS
chip, view-model rules and Stop button. Pane text is untrusted display data; the app sends fixed keys only after the core's rules allow.
Live tests: local, on fixtures and 1,500 mutated screens (deletions, truncations, injected escape sequences, fake footers, huge counts); nothing else.

## Findings

1. **Logic, Low (fixed)** - agy's `● [HH:MM:SS] cmd running` status line sits between the two rules of the footer and was returned as the
   typed draft. It would have blocked the open rule and shown a phantom draft. The draft reader now skips it. Test: `stop_and_open_are_allowed_only_on_the_matching_screens`.
2. **InputValidation, none found** - counts are capped at 999, commands at 20 x 200 characters, output rows at 200, notes are field-limited;
   the mutation test asserts the bounds and no panic over 1,500 variants; truncations of every fixture do not panic.
3. **Key injection, none found** - Stop sends one byte (`x` or `k`) and only when `background_stop_allowed` holds (a Tasks dialog with a
   `Command:` field, no question on screen); Open sends fixed steps only when the screen is that agent's, shows tasks, has no dialog,
   question or draft. A hostile output can print a fake footer or `Shell details` text, but the rule reads the last non-empty line of
   the pane and the dialect; the worst outcome is a counter shown, or one `x` typed into an empty input box.
4. **Controversial (accepted, named)** - the count is read from footer wording (`1 shell`, `1 task(s)`); an agent update that changes it makes
   the chip disappear, never send a wrong key.

## Verdict

Clean after the one fix (highest severity: Low).
