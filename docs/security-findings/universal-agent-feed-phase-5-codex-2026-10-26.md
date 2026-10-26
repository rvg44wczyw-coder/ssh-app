# Universal agent feed, phase 5 (Codex): hacker review, 2026-10-26

## Scope
- `crates/core/src/transcript/codex.rs` (rollout to turns), `screen/codex.rs` and `CodexDialect` (screen to turns,
  footer, draft, status), `session_source.rs` `CodexSource` locate arm, `prompt.rs` (`›` cursor, question lookup),
  registry row `codex`.
- Attack classes that apply: InputValidation (hostile rollout lines and pane text), DoS (huge lines, many lines),
  spoofing of the dialect detection, command injection through the locate script, MetadataLeak (account data).
  Not applicable: crypto, network protocol, replay, downgrade, key derivation.
- Live tests (own instances only): the real Codex 0.164.0 in a private tmux socket (owner's permission for this task)
  to capture screens and a rollout; the parser run on that real rollout; mutation fuzz of the pane parser seeded with
  the nine Codex fixtures plus Codex markers (`› `, `• `, `└ `, `← for agents`, `Queued follow-up`, `Yes, proceed (y)`):
  1,097,613 cases, 0 panics, worst case 1 ms; 5 MB single line 0.02 s; 200k lines 0.05 s.
- Code-analysis only: the locate arm (same quoting rules as the Claude and agy arms; `CODEX_HOME` is read from the
  process environment and used as a path prefix inside double quotes, the folder is matched with `grep -qF`).

## Findings
None new.

Notes (no action):
1. Text the agent prints in the bottom 10 lines of the pane still counts in full for the dialect score (the known
   limitation of the phases 1-4 report). Printed `← for agents` lines above the footer cannot flip another agent to
   Codex or Codex to another agent (test `printed_markers_do_not_flip_the_dialect` now includes two Codex screens).
2. A forged `› text` line near the bottom of a Codex pane with a hint line can show as the draft: cosmetic, the draft is
   only displayed.
3. Privacy: Codex's `/status` card prints the account e-mail and the session id. They stay in the pane and, if the owner
   opens `/status`, in the feed on the owner's own phone. The fixtures were scrubbed (account, session id, user paths)
   and checked with grep before being added. The rollout parser never reads `session_meta` account ids.

## Verdict
Clean.
