# Review and adversarial pass: skills manager (feature 53), 2026-10-23

Scope: `crates/host/src/skills.rs`, `crates/host/tests/skills.rs`, `crates/core/src/skills.rs`, `crates/core/src/host_api/skills.rs`, `SkillsSheetView.swift`. Live tests: the debug `ssh-app-host skills` binary fed (a) a project whose `.claude` is a link to another folder, (b) a name `../../etc`, (c) non-JSON, (d) a 300 KB body; plus the temp-dir test suite (symlinked skill folders and SKILL.md, user-scope writes, month-old trash purge, relative project). Real `~/.claude` and `~/.agents` were never touched.

## Findings

1. **[PathTraversal] none open.** Names are `[a-z0-9-]{1,64}` (no dots or slashes, so `.trash`/`.disabled` cannot be addressed as skills); the skills folder is built under the canonical project root and every level (`.claude`, `skills`, a skill folder, `SKILL.md`) must be a real directory or file, never a link. Live (a) created nothing in the link target.
2. **[DoS] none open.** Request capped at 200 KB on stdin, content at 100 KB (checked in the core and again in the helper), 500 entries listed, 100 files per skill, 4 KB read for descriptions.
3. **[SandboxEscape / confused deputy, Low, accepted]** The project folder comes from the tab's working directory; an agent that `cd`s somewhere makes the app offer that folder's `.claude/skills`. A write still needs the person's tap, is limited to that folder's skills tree and never overwrites outside it. Mitigation in the UI: the folder is shown at the top of the screen.
4. **[InputValidation, Low, accepted]** Check-then-rename races (a local process swapping a folder for a link between the check and the rename) are possible for another process of the same account; that process could edit the same files directly.
5. **Injection:** the request reaches the helper as one shell-quoted argument of `printf %s`; a unit test runs a real shell and gets the hostile request back byte for byte.
6. **Delete is reversible:** moves to `.trash/<name>-<time>` after a confirmation dialog; only entries older than 30 days are removed, only by exact `<valid name>-<digits>` pattern and only real directories.

## Verdict

Clean for Critical/High/Medium; two Low items accepted with the reasons above.
