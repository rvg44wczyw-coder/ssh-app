# Feature 50: Auto-approval by command category (configurable, with presets)

Status: spec (2026-10-21), not reviewed (`rev` next). Builds on `19-hooks-templates-and-host-guard.md` (host guard
rules enforced by the hook), `25-host-hook.md` (approval flow) and `docs/security-findings/2026-10-21-guard-allow-compound-commands.md`
(the allow semantics this feature relies on; fixed first).

## 1. Problem

Every Bash call of Claude Code or Antigravity (agy) is a question on the phone unless a rule says otherwise.
Most of them are the same few kinds: look at files, `git status`, `cargo test`, `go build`, `pytest`. The owner
wants to say once "these categories need no question" for each agent, with sensible presets, without writing rule
strings by hand. Rules exist already (`HostGuard.allow/ask/deny`, enforced on the host by the hook), but they are
a flat list of strings in an editor: hard to reason about, easy to make too broad, no notion of risk.

## 2. Goals and non-goals

Goals:

1. **Categories** of commands (read-only inspection, git, Rust, Python, Go, Node, Swift/Xcode, make, Docker,
   network, file changes, package installs, …) each with a **level**: `Ask` (default), `Auto` (allowed
   without a question), `Block` (refused on the host).
2. **Presets** that set many levels at once (Cautious, Developer, Trusted project, Lockdown) and can be adjusted.
3. **Per agent**: Claude Code and agy (and the other hook-capable agents as far as `host_guard_agent_support` allows)
   may have different levels.
4. **Risk is explicit**: each command in a category carries a risk tag; commands that run project code or reach
   the network are never hidden inside a "safe" category.
5. One enforcement point: the host guard in the hook (works the same for every agent), no new protocol.

Non-goals: not a sandbox (see §8); no AI judgement of commands (that is the agent's own auto mode); no
auto-approval from the phone side (the hook decides on the host, the phone is not involved when `Auto`);
no new rule syntax for the user to learn (the existing `Tool(spec)` rules stay the storage format).

## 3. Model

```
AutoCategory { id, title, description, risk, commands: [AutoCommand], tools: [String] }
AutoCommand  { rule: "Bash(cargo test:*)", risk, note }
Risk         = ReadOnly | LocalChange | RunsProjectCode | Network | Destructive
Level        = Ask | Auto | Block
AutoPolicy   = { agent-independent defaults: { category id -> Level },
                 per-agent overrides: { agent -> { category id -> Level } },
                 trusted_dirs: [String], preset_id: Option<String> }
```

Rules of the model:

- `Destructive` commands (`rm`, `git reset --hard`, `git push --force`, `dd`, `mkfs`, `sudo`, writes to agent/hook
  settings and key material) are **not selectable as `Auto` at all**: they live only in the "Always ask" and "Block"
  floor. A category can contain no destructive command.
- Commands are listed as **narrow subcommands**, never as the whole program: `cargo test`, `cargo build`,
  `cargo clippy`, `cargo fmt`, not `cargo:*` (that would allow `cargo run` and `cargo install`). `go build`, `go test`,
  `go vet`, not `go:*` (`go run`, `go install`). `python3 -m pytest`, not `python3:*` (`python3 -c` runs anything).
- `RunsProjectCode` commands (`cargo build`/`test`/`run`, `make`, `npm run`/`test`, `go test`, `pytest`, `python3 x.py`,
  `xcodebuild`) execute code from the repository (`build.rs`, proc macros, Makefile, package scripts, test code). They
  can be set to `Auto` only together with **trusted folders** (§5): outside a trusted folder they stay `Ask`.
- `Network` commands (`curl`, `wget`, `git fetch/pull/push`, `npm install`, `pip install`, `cargo install`, `docker
  pull`) are never in a "read-only" or "build" category; they have their own category and default `Ask`.
- The level of a category applies to its commands **as a whole**; a user who wants one command out edits the raw rules
  (the existing guard editor stays, §6).

### 3.1 Category list (first cut; the definitive table is data in the core, versioned)

| id | Title | Contents (rules), risk | Default level in "Developer" |
|---|---|---|---|
| `inspect` | Read-only inspection | `Read`, `Glob`, `Grep`, `Bash(ls)`, `pwd`, `cat`, `head`, `tail`, `wc`, `which`, `file`, `stat`, `echo`, `Bash(git status/diff/log/show/branch)`; ReadOnly | Auto |
| `git-local` | Git: local changes | `git add`, `git commit`, `git checkout`, `git switch`, `git stash`, `git restore`; LocalChange | Ask |
| `git-remote` | Git: remote | `git fetch`, `pull`, `push`; Network | Ask |
| `rust` | Rust (cargo) | `cargo check`, `build`, `test`, `clippy`, `fmt`, `doc`; RunsProjectCode | Auto in trusted folders |
| `python` | Python | `python3 -m pytest`, `pytest`, `ruff`, `mypy`, `python3 -m unittest`; RunsProjectCode | Auto in trusted folders |
| `go` | Go | `go build`, `test`, `vet`, `fmt`, `mod tidy`; RunsProjectCode | Auto in trusted folders |
| `node` | Node | `npm test`, `npm run lint`, `npm run build`, `pnpm test`, `yarn test`; RunsProjectCode | Auto in trusted folders |
| `swift` | Swift / Xcode | `swift build`, `swift test`, `xcodebuild … build/test`; RunsProjectCode | Auto in trusted folders |
| `make` | Make / CMake | `make`, `cmake --build`; RunsProjectCode (runs the repository's recipes) | Ask |
| `files` | Files: create and move | `mkdir`, `touch`, `cp`, `mv` inside the project, `Edit`/`Write` in the project; LocalChange | Ask |
| `docker` | Docker (read-only) | `docker ps`, `logs`, `images`, `inspect`; ReadOnly | Auto |
| `network` | Network | `curl`, `wget`, `WebFetch`, `WebSearch`; Network | Ask |
| `install` | Package installs | `npm install`, `pip install`, `cargo install`, `brew install`, `go install`; Network + RunsProjectCode | Ask |

"Always ask / Block floor" (not a category, fixed and shown read-only): `rm`, `sudo`, `dd`, `mkfs`, `git reset --hard`,
`git clean`, `git push --force`, `chmod -R`, `chown`, `curl … | sh`, edits to `~/.ssh`, `.env*`, agent settings and
`~/.ssh-app`. These are the existing "no-destructive", "no-secrets" and "protect agent config" presets, applied as `deny`
whenever any `Auto` category is on (§4.2).

### 3.2 Presets

| Preset | Effect |
|---|---|
| **Cautious** | `inspect` and `docker` Auto; everything else Ask. |
| **Developer** | Cautious + language categories (`rust`, `python`, `go`, `node`, `swift`) Auto **in trusted folders**; `git-local` Ask. |
| **Trusted project** | Developer + `git-local`, `files`, `make` Auto in trusted folders; `network`, `git-remote`, `install` Ask. |
| **Lockdown** | Everything Ask (removes all category rules), the floor stays. |

A preset sets levels; the user can then change any category; the screen shows "Custom (based on Developer)" once it
differs. Presets are data (core), not UI code, so the macOS TUI (feature 47) and the watch can use them.

## 4. Enforcement (host guard)

### 4.1 Where the policy lives

`~/.ssh-app/guard.json` (feature 19) gets one new object, read by the app and **ignored by the hook**:

```json
{ "v": 1, "deny": [...], "ask": [...], "allow": [...],
  "auto": { "preset": "developer", "levels": {"rust": "auto", ...}, "agents": {"antigravity": {"go": "ask"}},
            "trusted_dirs": ["/Users/me/rust/ssh-app"], "managed": {"allow": [...], "ask": [...], "deny": [...]} } }
```

The hook keeps reading `deny`/`ask`/`allow` only. The app **compiles** the policy into those lists when it saves:
the rules of `Auto` categories go to `allow`, of `Block` to `deny`, the floor to `deny`; user-written rules stay as
they are. `managed` records the exact strings the app generated last time, so the next compile removes exactly those
and nothing the user typed, even if the category definitions changed between app versions.

Per-agent levels: the hook already knows the agent; guard.json gets per-agent sections of the three lists
(`"agents": {"antigravity": {"allow": [...]}}`) read by the hook in addition to the common lists (small hook change
in `ssh-app-hook.py` and the Rust port `crates/host/src/guard.rs`, same tests in both).

### 4.2 Order of decision (unchanged from feature 19, restated)

`deny` (any part of the command matches, after peeling wrappers) → `ask` (same) → `allow` (**every** part covered,
words as typed, no `sudo`/`env`/`sh -c` borrowing, no file redirection, no process substitution — the fixed
semantics) → otherwise the phone is asked. A category set to `Auto` therefore only removes questions for commands
that are covered whole; `cargo test && rm x` still goes to the deny floor.

### 4.3 Trusted folders

`RunsProjectCode` rules are emitted into a separate list `allow_in` with the folders: the hook allows them only when
the payload's `cwd` is inside a trusted folder (both resolved with `realpath`; a path outside, a symlink out, or a
`cwd` that cannot be resolved means "not trusted"). `cd` inside a command changes the effective directory, so a line that
contains `cd`, `pushd`, `git -C` or `--manifest-path`/`-C <dir>` pointing outside the trusted folders is **not**
auto-allowed under `allow_in` (falls to the phone). Trusted folders are typed by the user in the app (folder picker
of the connected host); nothing a repository or an agent writes can add one.

### 4.4 Audit

The reason string of an auto-allow names the category and command (`Auto-approved: Rust · cargo test`), the hook
appends one line per auto-decision to `~/.ssh-app/run/auto.log` (time, agent, category, first 120 characters of the
command, display-safe; bounded ring of 500 lines). The app shows the last entries in the Auto-approval screen ("Recently
auto-approved") so a surprise is visible and traceable. No command text goes to push payloads.

## 5. UI (iOS first; the same data feeds the TUI)

Settings → **Auto-approval** (per connected host, like the guard; "Connect to a host first" when offline):

1. **Preset** row (segmented / picker: Cautious, Developer, Trusted project, Lockdown, Custom) with one line of
   explanation each.
2. **Agent** switch: "All agents" or one agent (Claude Code, Antigravity, others listed as supported); per-agent
   overrides are shown with a dot and can be reset.
3. **Categories list**: icon, title, risk badge, **Ask / Auto / Block** control. Rows whose `Auto` depends on trusted folders
   show "in trusted folders". Tapping a row lists its exact commands with risk badges and the rule behind each.
4. **Trusted folders**: list + "Add folder…" (remote directory picker already in the app) + warning text.
5. **Floor**: read-only list "Always asked or blocked".
6. **Recently auto-approved** (from `auto.log`) and **Preview rules** (the compiled `allow`/`deny`/`allow_in`) before **Save**.
7. Warnings (existing `host_guard_warnings` plus new): a preset above Cautious with no trusted folders ("language
   categories do nothing until you add a trusted folder"); hooks not installed or without `PreToolUse`; the agent does not
   support `allow`; a category's commands are partly already allowed by the agent's own settings.

The raw guard editor (feature 19) stays for exceptions and shows managed rules as locked lines with a "managed by
Auto-approval" caption.

## 6. Core API (UniFFI)

```
auto_categories() -> Vec<AutoCategory>
auto_presets() -> Vec<AutoPreset>
compile_auto_policy(policy: AutoPolicy, current: HostGuard) -> Result<HostGuard, SshCoreError>
auto_policy_from_guard(guard: HostGuard) -> AutoPolicy        // reads the "auto" object; unknown/invalid -> Lockdown
auto_policy_warnings(policy: AutoPolicy, agent_support: ...) -> Vec<String>
read_auto_log(session) -> Vec<AutoDecision>                     // via ctl on the host
```

Enums, not strings, for `Risk`, `Level` and preset ids (spec 36/UniFFI cleanup applies: `AutoLevel`, `AutoRisk`).
`compile_auto_policy` is pure and deterministic; it validates through the existing `validate_host_guard` (so rule
syntax and limits are the same) and refuses a policy that would make a `Destructive` command `Auto`.

## 7. Agent support

| Agent | Auto (allow) | Ask | Block | Notes |
|---|---|---|---|---|
| Claude Code | yes (PreToolUse / PermissionRequest) | yes | yes | needs the PreToolUse (or PermissionRequest) hook installed |
| Antigravity | yes | yes | yes | its own `permissions.allow` is honoured too (plain commands only) |
| Codex, Qwen Code, Crush, OpenCode | yes | no | yes | no "ask" dialect: unmatched goes to the phone as today |
| Grok | no | no | yes | can only block |
| Cline, Aider, Goose, Ollama chat | no | no | no | no hooks; the screen shows "not supported" |

Claude's own `permissions.allow` (feature 21) remains available and independent; the UI explains the difference
("Auto-approval works on the host for every agent and understands trusted folders; the agent's own rules stay in its
settings").

## 8. Security considerations (for `rev` and `hacker`)

1. **Allow needs every part covered** (the fixed semantics); tests keep the bypass list from the finding. Any new
   category rule must pass it.
2. **Narrow rules only.** A category may never contain a whole-program rule (`cargo:*`, `go:*`, `python3:*`, `make:*`
   is the one exception, kept `Ask` by default and flagged `RunsProjectCode`). A test asserts that no `Auto`-capable rule
   is a bare program name.
3. **Project code is attacker-controlled** under prompt injection (the agent can edit `build.rs`/`Makefile`/test
   files if `files` is `Auto`, then run `cargo build`). Therefore `files` Auto and `RunsProjectCode` Auto together in
   one trusted folder is the "Trusted project" preset only, with an explicit warning, and never the default.
4. **`cd`/`-C`/`--manifest-path` escaping the trusted folder** (§4.3) is handled by not auto-allowing such lines.
5. **Managed rules are rewritten only by the app**, but the agent runs as the same user and can edit `guard.json`
   with a shell command: the existing mitigation (deny edits of `~/.ssh-app/**`, part of the floor) is on whenever any
   `Auto` is on; the limitation (a shell write cannot be seen by a hook) is stated in the screen.
6. **Input too large** fails closed (existing). **Unknown tool names** are never auto-allowed (only the listed ones).
7. The audit log is display-safe, bounded, and not a source of truth for anything.
8. The policy comes only from the app user; nothing in a transcript, file or command text becomes a rule, a category
   or a trusted folder.

## 9. Phases

| Phase | Content | Done when |
|---|---|---|
| 0 | Fix `allow` compound semantics in the hook script and the Rust port | **done 2026-10-21** (`68b9e82`) |
| 1 | Core: category/preset data, `compile_auto_policy`, `auto_policy_from_guard`, enums, warnings; per-agent lists in the hook (script + port) | unit tests incl. "no bare program", "no destructive Auto", bypass list; parity tests script ↔ port |
| 2 | `allow_in` + trusted folders in the hook (realpath, `cd`/`-C` rule), audit log + `ctl auto-log` | loopback tests with a private tmux/sshd as in AGENTS.md |
| 3 | Bridge regeneration, iOS screen, raw-editor "managed" lines | simulator + Swift tests; device check |
| 4 | `rev` + `hacker` pass; docs: roadmap row, feature 19 cross-links | findings closed |

Workflow: `doc-dev → rev → rust-core-dev → rev → swift-bridge-dev → rev → ios-ui-dev → rev → hacker → merge`.

## 10. Open questions

1. Per-**project** policy (a policy file in the repository the user trusts) vs only per host: a repo-provided file is
   attacker-controlled, so it must never grant anything; at most it could *narrow* (add asks). Out of scope for v1.
2. Should `Auto` also cover `Edit`/`Write` of files inside a trusted folder (it is `files`, `LocalChange`)? Proposed: yes
   only in "Trusted project", only paths inside the folder, never dotfiles, `.git`, hook or agent settings.
3. Timeouts: should an `Auto` decision expire (e.g. "for 1 hour")? Proposed for v2: a "Trust this session for 1 h"
   button on the approval card that adds a temporary managed rule.
4. Windows/WSL hosts (feature 33/34): commands differ (`cargo.exe`, PowerShell); categories are POSIX-shell only in v1.
