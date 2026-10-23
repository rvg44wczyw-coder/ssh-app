# Feature 53: Agent skills manager

- **Status**: Draft 2026-10-23 (owner request). Not reviewed yet.
- **Design**: every new screen uses system components in Liquid Glass (`glassEffect`, `GlassEffectContainer`, system `List`/`Form`), see `prefer-system-ui-components`.
- **Agents**: Claude Code first (`SKILL.md` in `<project>/.claude/skills/<name>/` and `~/.claude/skills/<name>/`, or the profile's `CLAUDE_CONFIG_DIR`); Antigravity (`agy`) uses the project's `.agents/skills/<name>/SKILL.md`. Ollama has no skills.

## 1. Purpose

Skills decide how an agent works in a project, but today they can only be edited in a terminal. The owner wants a full management panel per project and per agent, and a periodic status of what the skills cost (Claude's `/context` and usage tell which skill or context part ate the most).

## 2. Behaviour

### 2.1 Management panel (session settings → Skills)

- **List** of skills visible to the session's agent, grouped by scope: *Project* (editable), *User* and *Built in / plugins* (read only). A row: name, one-line description (front-matter `description`), size in tokens (estimate), enabled state, scope badge.
- **Create**: name (validated `[a-z0-9-]{1,64}`), template (blank, "from existing", "from the project's role template"). Creates `<project>/.claude/skills/<name>/SKILL.md` (or `.agents/skills/` for agy) with front matter (`name`, `description`) and an empty body.
- **Edit**: a native editor (monospaced `TextEditor`, front-matter fields as form rows, body below, Markdown preview). Save writes the file atomically on the host (temp file + rename), keeps one `.bak`. Extra files in the skill folder are listed and can be opened, edited, added or deleted the same way.
- **Delete**: confirmation, moves the folder to `<scope>/skills/.trash/<name>-<timestamp>/` (restorable from the panel for 30 days); never `rm -r`.
- **Enable/disable**: renaming `SKILL.md` is not used; a disabled skill is moved to `.disabled/` next to the others (the agent does not see it). Reversible.
- Validation before save: front matter parses, `name` equals the folder name, `description` is not empty, total size ≤ 256 KB. A skill from a plugin or the repo's tracked files is shown read only with "Copy to project".

### 2.2 Status (periodic)

- **Per-skill cost**: estimated tokens of the front matter (always loaded) and of the body (loaded when used), computed on the host while listing.
- **Per-session usage**: for Claude Code the app captures `/context` output (a private read of the pane, like the usage line, at most every 90 s while the session screen is open and the agent is idle) and shows the categories (system prompt, tools, skills, memory files, messages) with the biggest one highlighted; the Skills row opens the panel sorted by cost. From the session file the app also counts which skills were actually invoked (`Skill` tool calls) per session: "used 3×".
- If the agent is busy or the capture does not parse, the last known value is kept and marked with its age; nothing is sent to the agent.

## 3. Interface

Host helper commands (`ssh-app-host skills …`, JSON, run over the existing SSH channel): `list --agent A --dir D`, `read --path P`, `write --path P` (content on stdin), `create`, `delete` (to trash), `restore`, `set-enabled`. UniFFI records: `SkillSummary { name, scope, description, path, tokens_front, tokens_body, enabled, read_only, uses }`, `SkillDoc { path, front_matter, body, files }`, errors typed (`SkillError::{Invalid, Exists, NotFound, TooLarge, ReadOnly}`).

## 4. Constraints and invariants

- Every path is resolved on the host against the allowed roots (project `.claude/skills`, `.agents/skills`, user skills, the profile's config dir); `..`, absolute paths in names and symlinks leaving the roots are rejected (canonicalise and prefix-check).
- No command from a skill file is ever executed by the app or helper; content is display and edit data.
- Writes are atomic; a failed write leaves the old file; the `.bak` is the previous version.
- Tests never touch the real `~/.claude` or `~/.agents`: they use temp dirs through the helper's `--root` override.
- Battery: status reads only while the session screen is open; no timers in the background.

## 5. Open questions (owner)

1. ~~Trash or real delete~~ — decided 2026-10-23: **trash, with a confirmation sheet** (as in §2.1).
2. ~~User-level skills~~ — decided 2026-10-23: **project skills only** are editable; user-level and plugin skills are listed read only ("Copy to project" to change them).
3. `/context` format differs between Claude Code versions: capture it first (like feature 52) before fixing the parser.

## 6. Plan (dev-chain)

`doc-dev` (this) → `rev` → `rust-core-dev` (helper commands, parser, path guard, tests in temp dirs) → `rev` → `swift-bridge-dev` → `rev` → `ios-ui-dev` (panel, editor, status card, Liquid Glass) → `rev` → `hacker` (path traversal, symlinks, trash, size limits) → merge.
