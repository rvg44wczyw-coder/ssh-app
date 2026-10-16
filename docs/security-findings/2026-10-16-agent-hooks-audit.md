# Security Audit — M15 Agent Approval Hooks & Launch Profiles (2026-10-16)

## 1. Scope
- `crates/core/src/agents.rs`, `session.rs` (`install/uninstall_agent_hooks`, `agent_hooks_status`, `write_remote_config`), `scripts/agent-hooks/ssh-app-hook.py`, iOS profile storage (`AgentProfile.swift`).
- Applicable: input validation / injection (profile → shell command, hook dir → agent shell), file writes (symlinks, temp files, data fidelity), DoS (hook stalls the agent), secret leakage, IPC spoofing (socket). Ruled out: crypto and replay (no signing here; the broker/signature path is Feature 03 and the broker does not exist yet), downgrade (no negotiation).
- Live tests (own temp dirs, own tmux server, scratch cargo project outside the repo): hostile/typed-wrong JSON into the real hook script, slow-drip fake broker, 3000-case quoting fuzz executing the generated command through `sh`, temp-file symlink clobber, JSON merge fidelity, argv exposure via `ps`. Code-analysis only: iOS storage.

## 2. Findings (all fixed and re-verified unless noted)
1. **MetadataLeak — Medium**: profile `env` values are embedded in the command line, so `ps` on the host shows them (verified: `ps -axo args` printed `env ANTHROPIC_API_KEY=sk-TESTSECRET ...` for tmux and its shell), and they are stored unencrypted in iOS `UserDefaults`. *Fix*: env keys containing `KEY/TOKEN/SECRET/PASSWORD/PASSWD/CREDENTIAL` are rejected; secrets must come from the host's own shell profile. Re-verified: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GITHUB_TOKEN`, `DB_PASSWORD` rejected, `CLAUDE_CONFIG_DIR` accepted.
2. **PathTraversal — Low**: `write_remote_config` / `build_write_file_command` wrote `<file>.ssh-app.tmp` with `>`, which follows a pre-planted symlink (verified: planted `settings.json.ssh-app.tmp -> victim.txt`, victim overwritten with our JSON). Needs write access to the config dir (same user), hence Low. *Fix*: `rm -f` + `set -C` (noclobber). Re-verified by a loopback-sshd test (victim unchanged).
3. **DoS — Low**: hook socket read used a per-`recv` timeout; a broker dripping 1 byte / 0.5 s kept the hook alive 12.3 s with `SSH_APP_HOOK_TIMEOUT=2`, up to the agent's own 130 s timeout. *Fix*: overall monotonic deadline. Re-verified (regression test: < 4 s).
4. **InputValidation — Low**: wrongly typed hook JSON (`[]`, `"str"`, `{"toolCall":"x"}`) crashed the script with an `AttributeError` traceback and exit 1 and no decision; Antigravity's exit-code semantics are undocumented. *Fix*: type guards plus a catch-all that yields "no opinion", exit 0. Re-verified (rc=0, empty stderr; deep nesting 200k and a 50 MB payload also handled, ~115 MB RSS).
5. **InputValidation — Low**: merging into a config rewrote numbers outside f64 range or precision (`18446744073709551616` → `1.8446744073709552e19`, `0.1000000000000000055…` → `0.1`), silently altering user settings. *Fix*: `serde_json` `arbitrary_precision`. Re-verified.

## 3. Tested and held
- 3000-case fuzz of env values/args with shell metacharacters (`'"\$`;&|<>(){}*?!#~`, tabs): 0 injections, sentinel file never created; values containing control chars rejected; only deliberate expansion is a leading `~`.
- Hook dir cannot inject into the agent's shell (strict charset); `python3 -I` isolates from `PYTHON*`/user site.
- Socket spoofing: world-accessible socket, group-readable dir, and symlinked socket are all ignored (no opinion), garbage/non-boolean/short replies never approve.
- Invalid JSON, BOM, 5000-level nesting, `hooks` of the wrong type: config refused, file untouched. Symlinked `settings.json` is written through, original mode and one-time `.bak` preserved.

## 4. Residual / accepted
- Hooks, script and socket live in the same account as the agent; a prompt-injected agent can edit its own hook config (stated trust boundary in the doc).
- Profile env still allows `PATH`, `SHELL`, `NODE_OPTIONS`, `GIT_SSH_COMMAND`: the profile author is the device owner.
- Duplicate JSON keys collapse to the last value (JSON semantics).
- Not reproduced: a single full-suite run showed 2 loopback tests failing once (exec at `list`/`status`) right after a heavy build; 0 failures in the next 15 runs (10 isolated, 5 full-suite). Cause unknown.
- Claude permission modes that skip prompts bypass `PermissionRequest` (documented).

## 5. Verdict
Findings (highest severity: Medium), all remediated and re-verified; no open issues.
