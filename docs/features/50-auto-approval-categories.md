# Feature 50: Auto-approval by command category (configurable, with presets)

Status: spec (2026-10-21), owner decisions on file edits, per-project policy and temporary trust applied (§10); not yet reviewed (`rev` next). Builds on `19-hooks-templates-and-host-guard.md` (host guard
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
6. **Projects with a hard boundary** (§4.3, §4.5): `Auto` is granted inside a project folder only; anything that
   reaches outside it is asked on the phone, never auto-allowed.
7. **Temporary trust** (§4.6): "Allow for N minutes" on the approval card, with a configurable duration, signed by the
   phone so the agent cannot create one.

Non-goals: not a sandbox (see §8); no AI judgement of commands (that is the agent's own auto mode); no
auto-approval from the phone side (the hook decides on the host, the phone is not involved when `Auto`);
no new rule syntax for the user to learn (the existing `Tool(spec)` rules stay the storage format).

## 3. Model

```
AutoCategory { id, title, description, risk, commands: [AutoCommand], tools: [String] }
AutoCommand  { rule: "Bash(cargo test:*)", risk, note }
Risk         = ReadOnly | LocalChange | RunsProjectCode | Network | Destructive
Level        = Ask | Auto | Block
AutoScope    = Project | Anywhere                 // where a category may be Auto
AutoPolicy   = { defaults: { category id -> Level },           // used inside any project below
                 per-agent overrides: { agent -> { category id -> Level } },
                 projects: [ AutoProject ],
                 inspect_anywhere: bool,                       // default false
                 preset_id: Option<String> }
AutoProject  = { root: String, label: String,
                 overrides: { category id -> Level },          // per project, wins over defaults
                 allow_file_edits: bool }                      // Edit/Write inside the root (see §3.3)
TempTrust    = { id, category ids, project root, agent, session id, expires_at, signature }
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
  can be set to `Auto` only inside a **project** (§4.3): outside every project they stay `Ask`.
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
| `rust` | Rust (cargo) | `cargo check`, `build`, `test`, `clippy`, `fmt`, `doc`; RunsProjectCode | Auto in projects |
| `python` | Python | `python3 -m pytest`, `pytest`, `ruff`, `mypy`, `python3 -m unittest`; RunsProjectCode | Auto in projects |
| `go` | Go | `go build`, `test`, `vet`, `fmt`, `mod tidy`; RunsProjectCode | Auto in projects |
| `node` | Node | `npm test`, `npm run lint`, `npm run build`, `pnpm test`, `yarn test`; RunsProjectCode | Auto in projects |
| `swift` | Swift / Xcode | `swift build`, `swift test`, `xcodebuild … build/test`; RunsProjectCode | Auto in projects |
| `make` | Make / CMake | `make`, `cmake --build`; RunsProjectCode (runs the repository's recipes) | Ask |
| `files` | Files: create, move, edit | `mkdir`, `touch`, `cp`, `mv`, and the `Edit`/`Write` tools, **inside the project only** (§3.3, §4.5); LocalChange | Ask (Auto in "Trusted project") |
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
| **Developer** | Cautious + language categories (`rust`, `python`, `go`, `node`, `swift`) Auto **inside projects**; `git-local` and `files` Ask. |
| **Trusted project** | Developer + `git-local`, `files` (including file edits), `make` Auto inside projects; `network`, `git-remote`, `install` Ask. |
| **Lockdown** | Everything Ask (removes all category rules), the floor stays. |

### 3.3 File edits (decision of 2026-10-21)

Edits and writes (`Edit`, `Write`, `MultiEdit`, `NotebookEdit`, and the agents' equivalents) may be `Auto` **inside the project
only**. The hook resolves the target with `realpath` (symlinks followed, `..` and `~` expanded) and allows only a path inside the
project root; a path outside, a symlink that leaves, or a path that cannot be resolved is **not** allowed automatically and goes to
the phone as a normal approval. Never auto-allowed even inside the project: `.git/**`, the agent's and ssh-app's settings and hook
files (`.claude/**`, `.gemini/**`, `~/.ssh-app/**`), `.env*`, key files, and files the floor already protects. The same boundary
applies to the shell forms (`cp`, `mv`, `mkdir`, `touch`): every path argument must resolve inside the project (§4.5).

A preset sets levels; the user can then change any category; the screen shows "Custom (based on Developer)" once it
differs. Presets are data (core), not UI code, so the macOS TUI (feature 47) and the watch can use them.

## 4. Enforcement (host guard)

### 4.1 Where the policy lives

`~/.ssh-app/guard.json` (feature 19) gets one new object, read by the app and **ignored by the hook**:

```json
{ "v": 1, "deny": [...], "ask": [...], "allow": [...],
  "auto": { "preset": "developer", "levels": {"rust": "auto", ...}, "agents": {"antigravity": {"go": "ask"}},
            "projects": [{"root": "/Users/me/rust/ssh-app", "label": "ssh-app",
                          "overrides": {"make": "auto"}, "allow_file_edits": true}],
            "inspect_anywhere": false, "trust_max_secs": 86400,
            "managed": {"allow": [...], "ask": [...], "deny": [...]} } }
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

### 4.3 Projects

A **project** is a folder on the host that the user added in the app (remote folder picker). The hook finds the project of a tool call
from the payload's `cwd` (both resolved with `realpath`; the longest matching root wins). A call whose `cwd` is in no project, or whose
`cwd` cannot be resolved, gets **no automatic allow** for any category with scope `Project` (all of them by default), so it is asked on
the phone as today. Rules reach the hook as `allow_in` entries (`{"root": ..., "rules": [...], "edits": true|false}`) next to the
plain `allow` list; the plain `allow` list keeps only what was typed by hand and, optionally, `inspect` when "Read-only anywhere" is on.

Per-project policy: each project has its own overrides on top of the defaults (a scratch project may allow `make`, a work project may
not allow `files`). The policy is **stored on the host and edited only in the app**. A file inside the repository never grants anything
(it is attacker-controlled); a future repo file could only *narrow* (add asks), and is out of scope here.

### 4.5 The project boundary (hard)

Every automatic allow under `allow_in` must pass the boundary check in the hook, in addition to the all-parts-covered check:

1. every path-like argument of the command and every tool path resolves (after `~`, `..`, symlinks, relative to `cwd`) **inside the
   project root**; an unresolvable argument (shell variable `$X`, `$(…)`, backticks, globs that expand outside, env-dependent
   values) counts as outside;
2. no `cd`/`pushd`/`git -C`/`--manifest-path`/`-C <dir>`/`--prefix`/`--directory` with a target outside the root (inside is fine);
3. no absolute path argument at all except under the root (`/tmp`, `/etc`, `/Users/…` elsewhere are outside);
4. tool calls with a path (`Edit`, `Write`, `Read`) follow the same rule with the tool's path.

Leaving the project is **only** possible through a manual phone approval: the call is not auto-allowed, it goes through the normal
signed approval flow; the approval card says "outside the project `<label>`" when that is the reason. The temporary trust of §4.6
**never extends a project boundary**: it is bound to the same project root and re-checked by the same boundary code.

### 4.6 Temporary trust ("Allow for 1 h")

On an approval card the user can tap **Approve, and allow `<category>` in `<project>` for `<duration>`**. The duration is configurable:

- Settings → Auto-approval → **Temporary trust**: default duration (15 min, 1 h, 4 h, until this agent session ends, custom
  minutes), the list of durations offered on the card, a switch to never offer it, and the host's hard cap
  (`trust_max_secs`, default 24 h, written to `guard.json`; the hook ignores longer requests).
- A trust covers the **categories of the approved command** (not the exact string), within the **same project**, for the **same agent
  session**. A command with no category, a `Destructive` command, anything on the floor, and anything outside the project boundary
  cannot be trusted this way (the button is not shown).

Forgery resistance: the agent runs as the same user and can write files, so a trust is a **signed fact**, not a file entry. The approval
decision the phone signs (SSHSIG, existing `approval.rs` canonical payload) is extended with a versioned trust block (`trust_secs`,
category ids, project root, session id). The hook verifies the signature with `ssh-keygen -Y verify` against `allowed_signers` before it
honours a trust and **stores the signed blob** under `~/.ssh-app/run/trust/` (private folder, bounded to 50 entries); on every use it
re-verifies the blob and checks expiry with the host clock. Writing a trust file by hand therefore gives nothing without the phone's
private key. Expired or invalid entries are deleted on read.

Visibility and control: the Auto-approval screen lists **Active trusts** (category, project, agent, time left) with Revoke and **Revoke
all**; each use is written to the audit log (§4.4) as `trusted` with the trust id; the host sends no push for it. A trust ends when the
agent session ends (the `SessionEnd` event) unless the chosen duration is "until session ends", which is the same event.

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
3. **Categories list**: icon, title, risk badge, **Ask / Auto / Block** control. Rows whose `Auto` depends on a project
   show "in projects". Tapping a row lists its exact commands with risk badges and the rule behind each.
4. **Projects**: list + "Add project…" (remote directory picker already in the app) with the folder as root; tapping a project
   opens its **overrides** (same Ask/Auto/Block control per category, "Allow file edits in this project" switch). A warning states
   the boundary rule in one sentence: "Anything outside the project folder is always asked on your phone."
   Also: switch "Read-only inspection anywhere" (default off).
5. **Temporary trust**: default duration, durations offered on the card, never-offer switch, host cap; **Active trusts** with Revoke.
6. **Floor**: read-only list "Always asked or blocked".
7. **Recently auto-approved** (from `auto.log`) and **Preview rules** (the compiled `allow`/`deny`/`allow_in`) before **Save**.
8. Warnings (existing `host_guard_warnings` plus new): a preset above Cautious with no project ("language
   categories do nothing until you add a project"); hooks not installed or without `PreToolUse`; the agent does not
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
list_trusts(session) -> Vec<TempTrust>; revoke_trust(session, id); revoke_all_trusts(session)
trust_options(category ids of a request, cwd) -> Option<TrustOffer>   // None: nothing can be trusted for this call
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
("Auto-approval works on the host for every agent and understands projects and their boundary; the agent's own rules stay in its
settings").

## 8. Security considerations (for `rev` and `hacker`)

1. **Allow needs every part covered** (the fixed semantics); tests keep the bypass list from the finding. Any new
   category rule must pass it.
2. **Narrow rules only.** A category may never contain a whole-program rule (`cargo:*`, `go:*`, `python3:*`, `make:*`
   is the one exception, kept `Ask` by default and flagged `RunsProjectCode`). A test asserts that no `Auto`-capable rule
   is a bare program name.
3. **Project code is attacker-controlled** under prompt injection (the agent can edit `build.rs`/`Makefile`/test
   files if `files` is `Auto`, then run `cargo build`). Therefore `files` Auto and `RunsProjectCode` Auto together in
   one project is the "Trusted project" preset only, with an explicit warning, and never the default.
4. **The boundary** (§4.5) is checked on resolved paths; anything unresolvable counts as outside; the check lives in the hook (script and port, same tests) and also covers the trust of §4.6.
5. **Managed rules are rewritten only by the app**, but the agent runs as the same user and can edit `guard.json`
   with a shell command: the existing mitigation (deny edits of `~/.ssh-app/**`, part of the floor) is on whenever any
   `Auto` is on; the limitation (a shell write cannot be seen by a hook) is stated in the screen.
6. **Input too large** fails closed (existing). **Unknown tool names** are never auto-allowed (only the listed ones).
7. The audit log is display-safe, bounded, and not a source of truth for anything.
8. The policy comes only from the app user; nothing in a transcript, file or command text becomes a rule, a category
   or a project.
9. **Temporary trust is signed** (§4.6): the hook verifies the phone's signature before storing it and on every use; the grace
   cannot outlive the host cap; revocation deletes the blob.
10. **File edits** (§3.3): realpath on the target, symlink-out and protected paths excluded, never `.git`/agent/hook settings.

## 9. Phases

| Phase | Content | Done when |
|---|---|---|
| 0 | Fix `allow` compound semantics in the hook script and the Rust port | **done 2026-10-21** (`68b9e82`) |
| 1 | Core: category/preset data, `compile_auto_policy`, `auto_policy_from_guard`, enums, warnings; per-agent lists in the hook (script + port) | unit tests incl. "no bare program", "no destructive Auto", bypass list; parity tests script ↔ port |
| 2 | Projects in the hook: `allow_in`, project lookup, **boundary check** (§4.5), file-edit rule (§3.3), audit log + `ctl auto-log` | loopback tests with a private tmux/sshd as in AGENTS.md; boundary test table (`..`, symlink out, `$X`, `-C`, absolute paths) in script and port |
| 3 | **Temporary trust** (§4.6): trust block in the signed decision (`approval.rs` canonical bytes, versioned), hook verify/store/expire, `ctl trusts/revoke`, host cap | forgery tests (hand-written trust file, expired, wrong key, wrong project/session) |
| 4 | Bridge regeneration, iOS screen (presets, categories, projects, temporary trust, active trusts), card button, raw-editor "managed" lines | simulator + Swift tests; device check |
| 5 | `rev` + `hacker` pass; docs: feature 19 cross-links | findings closed |

Workflow: `doc-dev → rev → rust-core-dev → rev → swift-bridge-dev → rev → ios-ui-dev → rev → hacker → merge`.

## 10. Decisions (owner, 2026-10-21)

1. **File edits**: yes, automatically, **inside the project only**; outside the project it is a normal ask, no auto-allow (§3.3, §4.5).
2. **Policy per project**: yes, strict. Stored on the host, edited in the app; **leaving the project folder only through a manual
   approval** (§4.3, §4.5). A repository file never grants anything.
3. **Temporary trust**: in the first version, with a **configurable duration** (§4.6).

## 11. Still open

1. Windows/WSL hosts (features 33/34): categories are POSIX-shell only in v1.
2. A repository file that can only narrow the policy (add asks): not in v1.
3. The exact set of path-taking flags the boundary check knows (`-C`, `--manifest-path`, `--prefix`, `--directory`, `--work-tree`,
   `--git-dir`, `-f` of make): kept as data in the hook, extended with the category tables; an unknown flag with a path-like value is
   treated as outside.

## 12. Review (`rev`, 2026-10-21) - applied

Findings of the review of §1-11, and what the implementation follows (these win over earlier text):

1. **No `managed` bookkeeping.** `HostGuard` keeps only what the user typed (`deny`/`ask`/`allow`) and gains `auto: AutoPolicy`.
   The hook JSON is **compiled on every write** (`host_guard_to_json`): nothing generated is ever read back as a user rule, so a
   stale rule from an older category table cannot survive. `guard.json` carries the policy (`"auto"`, for the app) and the compiled
   `auto_rules` (for the hook).
2. **One compiled shape for the hook** (`auto_rules`, evaluated in this order, deny before allow):
   `[{"agents": ["claude"]?, "root": "/p"?, "allow": [...], "deny": [...], "edits": bool}]`. An entry applies when the agent matches
   (absent = all agents) and, if `root` is present, the call's `cwd` resolves inside it. `allow` rules of an entry with a `root` pass the
   boundary check (§4.5); an entry without a `root` (only `inspect_anywhere` and global Block) is not boundary-checked.
3. **Precedence of levels**: project override > agent override > default. **Block** is per category and per agent, **not** per project
   (a project can only choose Ask or Auto), so a project cannot lift a Block.
4. **Floor** (§3.1) is emitted as global `deny` entries whenever any category is `Auto` or any project exists; it is never removable
   by a project or a trust.
5. **Scope of `Auto` is `Project`** for every category except when `inspect_anywhere` is on, which turns on only the `inspect`
   category (read-only commands and `Read`/`Glob`/`Grep`) without a project, still behind the floor.
6. **Trust message** (signed by the phone with the approval key, namespace `ssh-app-approval`, a distinct message prefix so it can
   never be mistaken for an approval): `SSH_APP_TRUST_V1:` followed by the fields `id, nonce, ts, expires, agent, session, root, edits,
   sha256(rules joined by "\n")`, each as `<utf8 length>:<bytes>\n`. The decision file carries `trust: {expires, agent, session, root,
   edits, rules[], signature}` next to the approval's own signature. The **rules travel signed** (the phone expands the categories), so
   the hook needs no category table. The hook rechecks `expires <= ts + trust_max_secs`, the agent/session of the live call, then the
   boundary and the all-parts check with the trusted rules.
7. **Where each rule is enforced twice** (script and Rust port): boundary, all-parts allow, edit rule, trust verify. One test table is
   shared (`crates/core/tests/hook_script.rs` for the script, `crates/host/src/guard.rs` tests for the port).
8. **Cost**: one `ssh-keygen -Y verify` per use of a trust (only when a trust matches the call's agent, session and project).


## 13. Implementation and security review (2026-10-21)

Phases 1-4 are implemented: `crates/core/src/auto_approval.rs` (categories, presets, policy, compile to `auto_rules`,
trust bytes, offers), `scripts/agent-hooks/ssh-app-hook.py` and `crates/host/src/{auto,trust}.rs` (evaluation, boundary,
audit log, signed trust), `HookTemplatesView.swift` (screen) and `ApprovalBannerView.swift` (card button). One integration
test per behaviour runs against both implementations (`SSH_APP_TEST_IMPL=rust`).

Findings of the `hacker` pass, all closed:

- `inspect_anywhere` and in-project `cat` could read `~/.ssh`, `.env`, cloud credentials through Bash (the floor only
  covers the Read tool). Fix: an automatic allow never applies to a call that names a credential, agent-settings or
  `.git` path anywhere in its text (`touches_sensitive`, script and port, also for trusts).
- Prefix rules `git branch -a`, `git branch -v`, `git remote -v` could be extended with `-D` / `add`. Replaced by
  `git branch --show-current` and `git remote get-url`.
- A trust is bound to the request the phone saw (id, nonce, agent, session), carries its own SSHSIG, is re-verified on
  every use, ends with `SessionEnd`, and only works while its root is still a project of the policy.

Known limits (documented, not fixed): `git commit` runs the repository's own git hooks and `cargo`/`make`/`npm` run
project code (the categories say so and the warnings repeat it); a command substitution inside an argument
(`git commit -m "$(…)"`) always asks.
