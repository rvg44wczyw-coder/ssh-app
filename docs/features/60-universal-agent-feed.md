# Feature 60: Universal agent feed (one pipeline, one interface per agent)

- **Slug**: `universal-agent-feed`
- **Target Release**: Milestone 49
- **Roles**: `doc-dev`, `rev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `hacker`
- **Hacker Gate**: Yes (host adapters read session files and parse untrusted agent output; new agents add parsers of untrusted text)
- **Status**: Plan (2026-10-25); phases 0-3 done and phase 4 done in code 2026-10-25 (device run pending), see "Progress" at the end. Not reviewed by the owner. Builds on `docs/architecture/screen-parser.md`, `docs/architecture/codex-grok-study-2026-10-25.md`, `docs/features/59-background-tasks.md`, `docs/features/11-agent-hooks-and-launch-profiles.md`.

## 1. Purpose

The feed (chat view of an agent's work) works today for Claude Code and Antigravity. Codex and Grok are next, and further agents
(OpenCode, Qwen Code, Crush, Cline, Aider, Goose, ...) after them. Each new agent must be **one adapter plus one descriptor row**,
not a new branch in a dozen places. This feature refactors what exists into small interfaces, proves with golden tests that nothing
changes for Claude and agy, and then connects Codex and Grok through the same interfaces.

Out of scope: the visual redesign of the feed, background tasks (spec 59, built on top of this), the macOS app.

## 2. What exists (study of the code, 2026-10-25)

```text
host (laptop)                         core (Rust, on the phone)                         iOS
─────────────                         ─────────────────────────                         ───
find agent in pane  ── ps/awk script  crates/core/src/host_api/mod.rs                   TerminalSessionViewModel
 (claude | agy)        SESSION_TRANSCRIPT_BODY                                           (isAgyAgent, detectedAgy ...)
read session file   ── claude: jsonl  feed_read.rs (delta read), feed_stream.rs         FeedTextView  (agent-agnostic)
                     └ agy: SQLite →  crates/host/src/agyfeed.rs  → Claude-shaped jsonl  AgentTraits (keys per agent)
screen capture      ── tmux pane      screen/{mod,claude,agy,dialog,...}.rs             AgentType enum (UI choice)
                                       ScreenDialect {Claude, Agy, Unknown}              68 per-agent branches in 16 files
hooks               ── ~/.claude, …   agent_hooks/ (per-agent plan, already table-like) HookPreview, Skills, Profiles views
```

Findings:

1. **The back half is already generic.** `TranscriptParser` / `FeedState` / `feed_read` / `feed_stream` and `FeedTextView` work on one
   normalized record shape (`TranscriptTurn`). Antigravity already proves the pattern: its SQLite database is converted by the host
   helper (`agyfeed.rs: step_record`) into Claude-shaped JSONL records, and the client parses them with the same parser.
2. **The front half is per-agent code spread out**: the host detection script (`if b == "claude" … if b == "agy"`), the file locator
   (a `case "$kind"` block per agent), `detect()` and `claude()`/`agy()` in `screen/mod.rs`, `ScreenDialect` as a closed enum, the
   `AgentType` enum and its 68 uses in Swift, and the `AgentTraits` table that lives in Swift while the dialects live in Rust.
3. **`AgentKind` already lists** Claude Code, Antigravity, Codex, Grok, OpenCode, Qwen Code, Crush, Cline, Aider, Goose
   (`agents.rs`, hooks and launch profiles). The feed ignores everything but the first two.
4. **Hooks are table-like already** (`agent_hooks/plan.rs`): the model to copy for the feed.

## 3. Target design: four seams, one registry

One registry of **agent descriptors**, owned by the Rust core, drives everything; Swift reads it instead of branching.

```text
AgentRegistry (core)  ──►  AgentDescriptor { kind, display_name, process_names, config_dirs,
                                             SessionSource, ScreenDialect, traits, commands, hooks }
```

### 3.1 Host seam: `SessionSource` (find the agent, locate its session, read it incrementally)

A trait in the host helper (`crates/host`), one implementation per agent, selected by `AgentKind`:

```text
trait SessionSource {
    fn kind(&self) -> AgentKind;
    fn matches(&self, args: &[String]) -> bool;                       // process detection, replaces the awk branches
    fn locate(&self, ctx: &PaneContext) -> Option<SessionRef>;        // pane pid, cwd, env, hook-noted session id
    fn read(&self, s: &SessionRef, cursor: Cursor) -> ReadResult;     // normalized records + new cursor
}
```

- **Output is always the normalized record stream** (§4). Claude: the file as it is (already normalized). Antigravity: today's
  `agyfeed`. Codex: `rollout-*.jsonl` mapped to records (`item_completed` → turns, `CommandExecution` → tool calls). Grok:
  `updates.jsonl` / `chat_history.jsonl` mapped (formats to be read in phase 5/6).
- The detection script of `host_api/mod.rs` is generated from the registry's `process_names` instead of hand-edited per agent.
  The helper (`ssh-app-host`) gets one subcommand `feed locate|read --agent <kind>`; the shell fallback keeps the current Claude
  and agy scripts only (no new shell code for new agents: they require the helper, like the other typed commands).
- Everything read from an agent's files is untrusted: bounded sizes, no execution, control characters stripped by the existing
  `strip_ansi_codes` path.

### 3.2 Screen seam: `DialectParser` (the pane as a feed when there is no file, and dialogs, status, draft)

`ScreenDialect` stays a UniFFI enum (Swift needs it) and gains `Codex`, `Grok`; the code behind it becomes a trait with a registry:

```text
trait DialectParser: Sync {
    fn dialect(&self) -> ScreenDialect;
    fn score(&self, lines: &[String]) -> u32;                         // replaces detect()'s counters
    fn turns(&self, lines: &[String], b: &mut Builder);               // today claude() / agy()
    fn footer(&self, lines: &[String]) -> Footer;                     // draft, hints, background count (spec 59)
    fn status(&self, lines: &mut Vec<String>) -> Option<String>;      // the live "working" line
    fn dialog(&self, lines: &[String]) -> Option<ScreenDialog>;       // slash-command screens
}
static DIALECTS: &[&dyn DialectParser] = &[&ClaudeDialect, &AgyDialect, &CodexDialect, &GrokDialect];
```

`parse_screen_feed` becomes: strip, `detect` = highest `score`, then call the winner's methods. Shared helpers (`Builder`,
`split_cols`, `numbered`, dialog row types, `restyle`) stay common. A dialect that is not written yet is simply absent: its
screens fall back to `Unknown` ("Terminal output"), as now.

### 3.3 Descriptor seam: `AgentTraits` and commands move to the core

`AgentTraits` (send-now keys, pull-queue keys, clear-line, suggestion key, image prefix; spec 59 adds open/stop/close-tasks keys)
and the per-agent **command catalog** (slash commands for the paperclip menu, today `AgentCommandItems` in Swift) become one
exported record `AgentDescriptor` built from a static table in the core. Swift gets `agent_descriptor(kind)` and reads fields; it
has **no** `if agy` / `switch agentType` left in the feed code. Unknown or unchecked agents get the `generic` descriptor (only keys
every terminal editor has).

### 3.4 iOS seam: resolve once, query traits

`TerminalSessionViewModel` resolves `AgentKind` once per tab (from the tab's creation type or from detection, as `resolve` does
now) and holds `descriptor`. The 68 branches become queries (`descriptor.traits.sendNowKeys`, `descriptor.commands`,
`descriptor.supportsDraftMirror`). `AgentType` (the "what to create" choice in the UI) stays as UI vocabulary and maps to
`AgentKind`; it adds `codex` and `grok` cases. Generics in Swift: no generic views; a feed view is one type that takes a
descriptor value (a generic over a protocol would only multiply specialisations and does not cross the UniFFI boundary).

### 3.6 Adding an agent changes no architecture

The acceptance rule of the whole design (owner's requirement, 2026-10-25): **a new agent, local or remote, is added without touching
the feed pipeline.** What a new agent costs:

| Step | Where | Size |
|---|---|---|
| 1. A `SessionSource` implementation (find the session, read it into normalized records) | `crates/host/src/sessions/<agent>.rs` | one file |
| 2. A `DialectParser` implementation (screen markers, status, footer, dialogs) | `crates/core/src/screen/<agent>.rs` | one file |
| 3. One row in the registry (process names, config dirs, traits, command list) | `crates/core/src/agent_registry.rs` | one table row |
| 4. Fixtures (session file, screens) and a registry line in the golden tests | `tests/` | data |

Nothing else changes: `TranscriptParser`, `FeedState`, `feed_stream`, `FeedTextView`, the bridge records and the Swift views stay as
they are, and no `if agent == …` is added anywhere. If a new agent seems to need one, the seam is wrong and is fixed first.

- **Local or remote is not the agent's business.** `SessionSource::locate/read` work through a small host-I/O interface
  (`HostIo`: run a command, read a file range, list a folder), not directly through SSH. The SSH transport implements it today;
  the local transport of the macOS app (spec 58) implements it later. An adapter written once serves an agent on the laptop
  and on a remote machine.
- **One conformance suite for every adapter.** A generic test harness (`fn conformance<S: SessionSource>(source: S, fixtures)`)
  checks the same properties for each: output parses with `parse_transcript_jsonl`; reading in any chunking equals reading the
  whole; no panic on any prefix of a fixture; oversized and malformed lines are bounded; control characters never reach the
  turns; paths come from the registry, never from file contents. A new adapter is done when it passes the suite. The same for
  `DialectParser`: a screen suite (every fixture parses; a screen of another agent scores lower; truncation sweep).
- **A scaffold** (`scripts/new-agent.sh <id>`) writes the two files, the registry row and the fixture folders with `todo!()`
  free stubs that return "not recognised", so a half-written agent degrades to `generic` instead of breaking the build.
- **Capture helper**: `scripts/capture-agent.sh <command>` starts the agent in a private tmux socket (AGENTS.md rules) inside the app
  folder, records the screens named in a list (idle, working, dialog, background task) as fixtures with the account lines
  removed, and copies the newest session file. It runs only on the owner's permission for that one task.
- **Not planned**: loading agent definitions from files or URLs at run time. A declarative-definition engine is a separate,
  larger design with its own security surface (regexes and paths from untrusted files); it is deferred until the adapter route
  proves too slow for the long tail of simple agents.

### 3.5 Where generics and interfaces pay, and where they do not

| Place | Choice | Reason |
|---|---|---|
| Host `SessionSource`, core `DialectParser` | traits + static registry | real polymorphism: N agents, one call site |
| `AgentDescriptor`, `AgentTraits` | plain data (records + table) | crosses UniFFI; data, not behaviour |
| Swift feed code | one type + descriptor value | UniFFI cannot export generics or trait objects |
| Normalized record | one versioned JSON schema (§4) | the only contract between host adapters and the client parser |

## 4. The normalized record (the contract)

**Correction found in phase 0:** `TranscriptParser` accepts **two** record shapes today, not one: Claude's (`type`, `message`, content blocks) and the step shape of Antigravity (`step_index`, `source`, `type`, `content`, `tool_calls`), which the host converts from its database. Phase 0 freezes both; the contract below is the union, and a new adapter may emit either. Documented from what `TranscriptParser` reads today. Adapters emit exactly this, one JSON object per line:
user text, assistant text blocks, `thinking`, `tool_use` (id, name, input) and `tool_result` (id, content, is_error), system notes
with a label, turn duration, and a `version: 1` marker on the first record. New fields are additive; the parser ignores unknown
ones. This schema is the acceptance test for every adapter: *feeding an adapter's output to `parse_transcript_jsonl` gives the
turns the agent's own screen shows*.

## 5. Per-agent matrix (from the study)

| | Claude | Antigravity | Codex | Grok |
|---|---|---|---|---|
| Process | `claude` | `agy`, `antigravity` | `codex` | `grok`, `agent` |
| Session source | `~/.claude*/projects/<slug>/*.jsonl` | SQLite via helper | `~/.codex/sessions/Y/M/D/rollout-*.jsonl` | `~/.grok/sessions/<project>/<uuid>/` |
| Mapping | as is | `step_record` (exists) | `item_completed` + `CommandExecution` | `updates.jsonl` (to read) |
| Screen | `⏺ ❯ ✻ ⏵⏵` | `○ ● Keyboard:` | banner `>_ OpenAI Codex`, `›`, `•`, `Worked for` | boxed TUI, header row, boxed prompt |
| Dialogs | `/model /config …` (done) | done | `/permissions /model /status` (to capture) | `/settings`, `/model`, `/session-info`, `Ctrl+x` (captured) |
| Hooks | done | done | done; user must trust in Codex's own TUI | done; folder trust gate |
| Background tasks | footer `N shell`, detail screen | `/tasks` | `/ps`, `/stop` all | feed rows, dashboard |

## 6. Plan (phases, roles, gates)

Rule for every phase: **behaviour for Claude and Antigravity does not change**; each phase ends green on `cargo fmt`, `clippy -D
warnings`, `cargo test`, the Swift suite and a simulator build, then `rev`.

| Phase | Content | Role | Gate / done when |
|---|---|---|---|
| 0 | **Golden tests first.** Freeze today's behaviour: `parse_screen_feed` on every fixture in `screen/fixtures` (turns, status, draft, prompt, dialog serialised to a snapshot), `parse_transcript_jsonl` and `FeedState` chunk-split equality on real Claude and agy JSONL fixtures, `AgentTraits.resolve` table, the detection script on recorded `ps` outputs | `rust-core-dev` | snapshots committed; they must stay byte-identical through phases 1-4 |
| 1 | `AgentRegistry` + `AgentDescriptor` in the core (data only: kinds, process names, config dirs, traits, commands); `agent_descriptor(kind)` exported; bindings regenerated | `rust-core-dev`, `swift-bridge-dev` | descriptor of Claude/agy equals today's Swift `AgentTraits` (test) |
| 2 | Host `SessionSource` trait; move Claude and agy behind it; detection script generated from the registry; `ssh-app-host feed locate|read` | `rust-core-dev` | golden tests green; loopback tests on a private tmux/sshd (AGENTS.md) |
| 3 | `DialectParser` trait + registry; `claude()`/`agy()`/dialog code moved behind it; `detect()` by score | `rust-core-dev` | screen snapshots byte-identical |
| 4 | iOS: resolve `AgentKind` once, hold `descriptor`, replace the 68 branches by queries, delete the Swift `AgentTraits` and `AgentCommandItems` tables, add `codex`/`grok` to `AgentType` | `ios-ui-dev` | Swift suite + simulator build; screenshots of the feed for Claude and agy unchanged |
| 5 | **Codex**: `SessionSource` (rollout mapping), `CodexDialect`, descriptor row (keys, commands from live capture), hook trust note in the UI | `rust-core-dev`, `ios-ui-dev`, `hacker` | live check with the owner's permission for that task; findings fixed |
| 6 | **Grok**: read the two session formats first (permission), `SessionSource`, `GrokDialect`, settings/model/session-info screens, descriptor row, hook trust gate in the UI | same | same |
| 7 | Background tasks (spec 59) on the new seams | same | per spec 59 |
| 8 | **Add-an-agent kit**: `HostIo` interface under `SessionSource`, the conformance suites (§3.6), `scripts/new-agent.sh`, `scripts/capture-agent.sh`; prove it by adding one simple agent (OpenCode or Aider) with only the four steps of §3.6 | `rust-core-dev`, `ios-ui-dev` | the sample agent is added with no change outside its two files, one registry row and its fixtures; the suites pass |

Each of 5 and 6 is "one adapter, one dialect, one table row, fixtures": if a phase needs a new `if agent == …` anywhere else, the
design has failed and the seam is fixed first.

## 7. Tests

- **Golden/characterisation** (phase 0) are the safety net for the refactor; they are not removed afterwards.
- **Per adapter**: fixtures cut from real sessions (sanitised: `/tmp/project`, `user@example.com`; no account ids), a truncation
  sweep (no panic on any prefix), oversized and malformed lines, a record with escape sequences and a fake prompt.
- **Registry**: every `AgentKind` has a descriptor; `generic` is used for agents without a dialect; no two kinds share a process
  name; detection of nested processes (`node … codex`, wrapper scripts) on recorded `ps` outputs.
- **Live checks** (one task = one permission, agent started with its folder in the app's directory, private tmux socket): the
  Codex queue and approval screens, Grok's task list, the Grok session formats.

## 8. Security

Adapters read files the agent wrote: treat as untrusted (size caps per line and per read, no path built from file contents, the
existing `validate_transcript_path` for every path). A new dialect parser never executes or sends anything it parses; the keys
the app sends come only from the descriptor table, not from screen text. The detection script is generated from fixed registry
strings (no agent-controlled text in the script).

## 9. Review (`rev`, 2026-10-25), applied

- Documentation checklist: purpose, interfaces, constraints, phases and tests present; the normalized record is the single contract
  and is stated as such (§4). Gap closed: phase 0 makes "no behaviour change" testable instead of promised.
- `[controversial]` A trait + static registry for ~4 agents is more machinery than four `match` arms. Argued for it anyway: the list
  is meant to grow to ten, the arms today are already in 16 Swift files and 8+ Rust files, and the registry's cost is one table.
  If after phase 3 it reads heavier than the code it replaced, collapse the trait to a plain function table.
- `[controversial]` Moving `AgentTraits` from Swift to the core adds a bridge call for data that changes rarely. Accepted: one
  source of truth for keys, commands and dialect beats two tables that must agree; the descriptor is fetched once per tab.
- `[controversial]` A `HostIo` layer between adapters and SSH is one more interface for a need (the macOS local transport) that is
  not built yet. Argued for it: adapters written against SSH directly would have to be rewritten for the local transport, and
  the layer is three small methods that the existing `execute_command_on_active_session` already provides.
- `[controversial]` "No `if agent == …` anywhere" is a rule that a deadline will tempt us to break once. The golden tests include a
  grep-style check (a test that fails if an agent id literal appears in the feed code outside the registry and the adapters).
- `[controversial]` Normalizing Codex and Grok into the Claude record shape loses native detail (Codex's JS `exec` wrapper,
  Grok's subagent events). Accepted as a named tradeoff: the feed shows what Claude's feed shows; native extras can become
  optional record fields later without breaking adapters.

## 10. Open questions

- Grok's `updates.jsonl` and `chat_history.jsonl` formats (phase 6 starts by reading them; needs one live-run permission).
- Where Codex and Grok keep hook trust (to show "hooks need your trust" correctly).
- Agents without a dialect (OpenCode, Qwen Code, Crush, Cline, Aider, Goose): ship as `generic` (launch and hooks only) until
  captured; decide per agent after phase 6.
- Windows hosts (`crates/winhost`): the registry's process names and config dirs need a Windows column when that work resumes.

## Revision notes

- 2026-10-25: first version from the code survey and the Codex/Grok study; review applied (§9).
- 2026-10-25: owner's requirement "add a new agent type conveniently and fast, local or remote, without architectural changes": §3.6 rewritten as the add-an-agent rule (two files, one registry row, fixtures; `HostIo`; conformance suites; scaffold and capture scripts), phase 8 = the kit; run-time loaded definitions removed from the plan. §4 corrected: two accepted record shapes.

## Progress (2026-10-25)

| Phase | Result |
|---|---|
| 0 | Golden tests: screen snapshots (20 fixtures), transcript snapshots (both record shapes), chunked-feed equality, host detection on recorded `ps`, the `AgentTraits` table and resolve matrix. |
| 1 | `crates/core/src/agent_registry.rs`: `AgentDescriptor` (id, kind, display name, process names, dialect, `SessionStore`, keys, commands) for Claude, Antigravity and generic; exported lookups by id, kind and program. |
| 2 | `crates/core/src/session_source.rs`: trait `SessionSource` (id, hook name, argument markers, locate arm) for Claude and Antigravity; the host locate script is built from the registry and the sources. Kept behaviour proved by end-to-end runs of the script with a fake `ps`/`lsof` and a temporary `HOME` (session id noted by the hooks, newest file, second account directory, agy by id and newest, nothing found). The text of the script differs from the old one in two lines only (the hook name is chosen with a `case`). The separate `ssh-app-host feed locate|read` subcommand of §3.1 is not built: the generated script serves both agents; it comes with the first agent that needs the helper (Codex or Grok). |
| 3 | `crates/core/src/screen/dialect.rs`: trait `DialectParser` (line score, overlay, turns, code colour, bold-heading rule), registry `DIALECTS`, `detect` by total score. The old `detect` is kept in the tests as the reference and compared on every fixture, every prefix of its lines and every single line; all snapshots are byte-identical. `split_footer` and `take_status` stay shared: they know the markers of every dialect at once. |
| 4 | Swift: `AgentTraits` reads the registry; `AgentResolver` is the one place that maps a tab to a row (the screen wins over the tab's type); `isAgyAgent`, `screenIsAgy`, `screenIsClaude` replaced by one `screenDialect` and the descriptor; the command menu is built from `descriptor.commands` and hidden when a tab's agent has none. |
| 5 (part 1, no live run) | Codex: `transcript/codex.rs` reads the rollout (`event_msg` `UserMessage`, `AgentMessage`, `Reasoning` summary, `CommandExecution` as one tool turn with its result, `task_complete` as "Worked for …"); the model's side (`response_item`, instructions, encrypted reasoning), `world_state` and token records are skipped; account ids in `session_meta` are never read. `CodexSource` (locate arm: noted session id, else the newest rollout whose first line names the folder; `CODEX_HOME` honoured; detection also behind `@openai/codex`), registry row `codex` (dialect `Unknown`, no send-now key, no commands). Fixture `tests/golden/transcript/codex_rollout.jsonl` (sanitised, `/tmp/project`) with snapshot and chunking test; end-to-end locate tests. **Waits for a live capture (owner's permission):** `CodexDialect` and a `ScreenDialect::Codex` value, the keys (queue, send now), the command list, approval screens, `/ps`. |
| 5 (part 2, live capture) | Live run of Codex 0.164.0 in a private tmux (owner's permission for this task; only `sleep`, a denied `touch` and a blocked `curl`, nothing edited). `screen/codex.rs` + `CodexDialect` + `ScreenDialect::Codex`: banner, `›` your input, `•` replies, `• Ran/Running/Exited (status N) <cmd>` with its `└` output as one `Bash` tool turn, `Worked for …`, `✗`/`■` notices; the footer is cut from the hint line (`← for agents · ? for shortcuts` or `tab to queue message`) up to the `›` composer and the `Queued follow-up inputs` block above it; the draft is read, the placeholder `Ask Codex to do anything` is not; `• Working (…)` is the status; the approval (`Would you like to run the following command?`, `› 1. Yes, proceed (y)` …) is a prompt (the `›` cursor added to the option pattern, the question skips the `$ cmd`/`Reason:` lines). Rollout: `turn_aborted` → "Interrupted after Ns". Keys verified live: **Enter during a turn sends at once (steers the turn), Tab queues, Shift+Left (`ESC [ 1 ; 2 D`) brings the last queued message back into the box, Up recalls history** (so `send_now_keys` is none and `pull_queue_keys` is Shift+Left). Command menu (18 entries) from `/`. Fixtures `screen/fixtures/codex_*.txt` (sanitised: account, session id, user paths removed). `/stop` ends the background terminals (`Exited (status -1)`), `/ps` lists them. Hook trust: a start screen `Hooks need review` (1 Review hooks / 2 Trust all and continue / 3 Continue without trusting) appears when hooks are new or changed. |
| 6 (Grok, live capture) | Grok Build 1.0.70 in a private tmux (owner's permission for this task; list_dir, one `echo` (our PreToolUse hook held it 2m7s until its 130 s timeout, then it ran), a denied `write`; nothing edited). **History:** `transcript/grok.rs` reads `~/.grok/sessions/<folder with / as %2F>/<uuid>/chat_history.jsonl` (`user` with `<user_query>`, `reasoning` summary, `assistant` with `tool_calls`, `tool_result` by `tool_call_id`; the system prompt, `synthetic_reason` messages and `<user_info>` are skipped; tools renamed to the common names). **Locate:** `GrokSource` (noted session id, else the newest session of the folder; `GROK_HOME` honoured). **Screen:** `screen/grok.rs` + `GrokDialect` + `ScreenDialect::Grok`: `❯` input with clock, `◆ Thought for`, `◈ Ran/Explored …` tools, `Worked for`, boxed composer (draft, placeholder `Build anything`), the data-retention notice cut with the footer, `… [stop]` working line as status, `#1 text` queue lines, the `[Send now][edit][cancel]` queue panel, the permission dialog (`┃ › Allow Edit to …?`, `1 (○) …`, hint `1/4:select`) as a prompt. **Keys verified live:** Enter queues, Ctrl+Enter (`ESC [ 13 ; 5 u`) sends now, Up then `e` brings the queued entry back ("editing queued #1"). 22-entry menu from `/` (built-in commands). Fixtures `grok_*.txt` and `grok_chat_history.jsonl` sanitised. Side effect on the owner's Grok: a keystroke of mine chose "always allow" for `tasks__list` (read-only MCP tool) in `~/.grok/sessions/<folder>/permission_grok-pager.toml`. |

Review of phases 0–4 (`rev`, 2026-10-25), all findings fixed:
- `[security: low]` program names from the registry went into the awk part of the locate script without a charset check; now only `[a-z0-9._-]` names are written (`is_safe_program_name`, tested with quotes, `$`, `;`, newline).
- `[quality]` the generic row was found as "the row of kind Aider", which breaks when Aider gets a row: `generic_agent_descriptor()` is exported and used.
- `[quality]` the registry was rebuilt over FFI on every `traits` access: read once (`AgentResolver.all`).
- `[quality]` `AgentTraits.resolve` was left only for tests: removed, tests use `AgentResolver`.

Scope note on phase 4: of the 68 per-agent branches found, those on the feed path are gone. The rest are per-agent **settings
screens** (hooks, launch profiles, permissions, skills, themes in `AgentProfile`, `SessionPermissionsSheetView`,
`HookPreviewSheetView`, `SkillsSheetView`, `AppAppearance`, `CommandPaletteSheetView`): they differ by agent by nature (what
hooks an agent supports, where its config lives) and belong to spec 11; they move onto the registry when Codex and Grok get
their hook screens (phases 5-6). The "no agent id literal in the feed code" guard test is added then, scoped to the feed files.
