# Project-scoped hooks, hooks overview, agy and Grok quota probes (2026-10-26)

## 1. Scope

Reviewed: `crates/core/src/agent_hooks/{config,overview}.rs` (`HookScope`, `agent_project_hooks_path`, `check_project_install`, the overview command and parser), `crates/core/src/host_api/hooks.rs` (`*_in` calls), `crates/host/src/hooksview.rs` (`hooks-overview`), `crates/host/src/{agyquota,grokquota}.rs` (screen parsers, private-tmux probe, cache), the `usage.rs` hooks into them.

Categories that applied: input validation, path traversal / injection, subprocess execution (probe launches agy/grok), file-system access driven by untrusted input (folders from agent histories), metadata leakage. Ruled out: crypto, replay, downgrade, MITM (no new protocol, no keys).

Live tests actually run (own loopback / local instances only):
- loopback sshd: project install into a folder named `a$(touch PWNED)b\`touch PWNED2\`'; touch PWNED3 #`: file written inside it, no `PWNED*` anywhere (regression test `project_hooks_touch_only_the_project_and_refuse_to_double_the_user_level_ones`).
- loopback sshd: user-level install then project install refused; project removal leaves foreign settings and the user level intact.
- fuzz: 300,000 random inputs built from hostile fragments (huge durations, negative and exponent percents, impossible dates, NUL, box drawing) into `agyquota::parse`, `limits_for` and `grokquota::parse`: 0 panics, every Grok percent within 0..=100.
- `hooks-overview` run against the real `$HOME` of the dev laptop (read-only): reported agents and projects, no writes.
- Probes run for real into private tmux sockets under `~/.ssh-app/run/{agyq,grokq}`; the owner's tmux pid (35350) unchanged; no leftover servers.
- A neutral empty folder as the probe's cwd was tried: both agents stop at their trust prompt and the probe fails (safe).

## 2. Findings

1. **Low, SandboxEscape** (accepted, documented): the quota probes start agy/grok in the folder of the pane where that agent already runs. A hostile repository there could carry project hooks or MCP servers that run when the probe's agent starts. Exposure is not new (the user's own agent runs in that folder), the probe only starts while such a pane exists and sends nothing but `/usage`. Fix direction if wanted: skip the probe for folders that hold project-level agent config the user has not trusted.
2. **Low, MetadataLeak**: `hooks-overview` lists user-owned directories found in agent histories (Claude/Codex/Grok/agy) to the phone of the same user. Paths only; Codex account fields are never read; folders not owned by the current user or not directories are dropped (checked by test `hostile_folders_are_dropped`).
3. **Low, InputValidation** (open, not a vulnerability): agy has no local project hooks file; its `.agents/hooks.json` may be committed by accident. The spec's offer to add it to `.git/info/exclude` is not implemented yet.

No Critical, High or Medium findings. The probe cache files are written atomically (tmp + rename) in the 0700 `run` folder; a parser given another user's cache cannot do more than show wrong numbers (day-old caches are ignored).

## 3. Verdict

Findings (highest severity: Low).
