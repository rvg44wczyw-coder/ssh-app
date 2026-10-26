# Universal agent feed, phase 6 (Grok): hacker review, 2026-10-26

## Scope
- `transcript/grok.rs` (chat_history.jsonl to turns), `screen/grok.rs` and `GrokDialect`, `GrokSource` locate arm,
  `prompt()` hook of `DialectParser`, registry row `grok`.
- Classes that apply: InputValidation (hostile history lines and pane text), DoS, dialect spoofing, command injection
  through the locate script (folder encoded with `sed`, always quoted), MetadataLeak. Not applicable: crypto,
  network protocol, replay, downgrade.
- Live (own instances only): real Grok Build 1.0.70 in a private tmux socket (owner's permission for this task);
  the real session parsed; mutation fuzz of the pane parser seeded with the ten Grok fixtures plus Grok markers:
  850,329 cases, 0 panics, worst 17 ms; 5 MB single line 0.02 s; 200k lines 0.05 s. Truncation sweep and hostile
  JSON lines in unit tests. `auth.json` was never opened.

## Findings
None new.

Notes:
1. A `type` of `system`/`user`/`assistant` with no Claude/agy/Codex field is now taken for Grok; Claude's `system`
   records carry `subtype` and were wrongly caught on the first try (found by the existing Claude test, fixed:
   Grok's system record must have a string `content` and no `subtype`).
2. The `[stop]` suffix makes any last pane line a status line; the status is only displayed.
3. Side effect of the live run: one keystroke chose "always allow" for the read-only MCP tool `tasks__list` in the
   owner's Grok permission file (noted in `docs/todo.md`); not a defect of the code under review.

## Verdict
Clean.
