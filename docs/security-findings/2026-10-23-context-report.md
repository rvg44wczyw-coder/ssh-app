# Review and security pass: `/context` report parser and "Context cost" section (2026-10-23)

Scope: `crates/core/src/screen/context.rs`, `TerminalSessionViewModel.measureContext`, `SkillsSheetView.contextSection`.
Format captured live from Claude Code 2.1.308 in a private tmux socket (`crates/core/tests/data/claude_context.txt`).

## rev
- Pure parser, no I/O, no `unwrap()`; numbers parsed strictly (digits and one dot, optional k/m), capped at 100 M tokens; at most 300 skills and 20 categories; the last report on the pane wins.
- The app types only the fixed command `/context` (validated by `runAgentCommand`), and only when no status line, prompt or dialog is on the screen (the agent is idle). It is a local command: no model tokens are spent.
- Tests: real capture (model, window, categories, per-skill costs, `< 20 tokens`, memory/MCP trees ignored), absent report, hostile numbers, 1000-skill flood.

## hacker (loopback/local only, no real tmux touched)
- Hostile pane text (huge numbers, `1e9`, percent 99999) gives no report or bounded values: covered by the test above.
- Skill/category names are display text only; nothing from the report is executed, sent as keys or written to a file.
- Verdict: clean.

## Controversial
- The report stays in the pane (and the feed) after the measurement, because Claude prints it as a normal turn. Accepted: it is also what the owner would see when typing `/context`.
