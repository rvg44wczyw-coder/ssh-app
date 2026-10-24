# Hook extra commands (M46): rev and hacker pass, 2026-10-24

## Scope

`scripts/agent-hooks/ssh-app-hook-extra.py` (wrapper), `crates/core/src/agent_hooks/{config,plan,preset}.rs`
(command assembly, validation, preview summary), `host_api/hooks.rs` (install of the wrapper).
Live tests: the wrapper run as a local subprocess against a stand-in hook, in a temp dir; no network, no real
agent, no tmux. Ruled out: MITM, replay, downgrade, key confusion, timing (no crypto or network here).

## Findings (all fixed)

1. **[InputValidation / guard bypass] Medium-High.** The wrapper read at most 1 MB of the event and handed
   that cut-off text to the ssh-app hook. A tool call with a >1 MB input made the hook fail to parse its input
   and have no opinion, which lets the call through under Auto/bypass: a host-guard deny rule was skipped.
   Without the wrapper the hook reads everything. Verified live (2 MB event: the hook received 1,048,576
   bytes, invalid JSON). Fix: the hook gets the whole event; only the extra command's copy is capped at 1 MB.
   Tests: `a_missing_hook_and_a_huge_input_do_not_crash_the_wrapper`, `the_extra_command_sees_at_most_one_megabyte`.
2. **[DoS] Medium.** A command that left a background child holding its stderr (`sleep 4 &`) held the wrapper
   until the child ended (4.1 s measured), and with a timeout that counted as a failure (deny when "stop on
   failure" is set). Fix: stderr drained by a thread, the wrapper waits only for the command itself.
   Test: `a_child_left_in_the_background_does_not_hold_the_wrapper`.
3. **[DoS] Low-Medium.** The wrapper buffered all of the command's stderr: 300 MB of output gave 716 MB RSS.
   Fix: only the first 300 bytes are kept. Test: `a_flood_on_stderr_is_not_kept_and_escapes_do_not_reach_the_log`.
4. **[MetadataLeak / terminal escape] Low.** Escape sequences from stderr went into `extra.log` unchanged
   (`cat` of the log could rewrite the terminal). Fix: non-printable characters become spaces.
5. **[Availability] Low.** The agent's hook time limit (130 s) did not cover an extra command running before
   the hook's own wait (up to 120 s): the agent could cut the wrapper off. Fix: the written limit is 130 s plus
   the extra command's timeout (`handler_timeout`, test `the_agents_time_limit_covers_...`).

Checked and fine: base64 URL-safe command (no quoting escape, `hostile_text_in_the_command_stays_data`);
arguments `nan`, `inf`, `-5`, bad base64 and missing `--` mean "no opinion"; the inner hook is run without a
shell; the log is opened `O_NOFOLLOW`, mode 0600.

## Accepted

- A malformed wrapper command line (only possible if someone edits the agent's settings) makes the ssh-app hook
  not run at all; this equals an agent that can edit its own hook settings, covered by the "Protect agent and
  hook settings" templates.
- The extra command is the user's own code and runs with the user's rights by design; the preview shows it
  decoded before install.

## Verdict

Findings (highest severity Medium-High), all fixed in the same pass.
