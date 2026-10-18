# Feature 24: `ssh-app-host` — typed host helper (phase P0 of the host service)

Status: spec (2026-10-18). Part of `docs/features/16-host-service-analysis.md` (phases P0–P3, owner
decisions in its section 11). This document covers **P0 only**: the helper binary, the usage probe
ported to it, the installer and the fallback. P1 (hook), P2 (launchd agent while a phone session
exists) and P3 (push) get their own specs.

## 1. Goal

Today the host side is ~600 lines of Python and shell embedded in Rust strings and run through
`python3` (`usage_probe.py`) or `sh`. Parsing is by text markers, nothing is versioned, and the
usage probe re-reads every transcript on each call. P0 introduces one small Rust binary,
`ssh-app-host`, that the app installs on the host and calls instead:

- no `python3` dependency for usage;
- a versioned, typed JSON answer (`hello` reports version and capabilities, so the app can tell
  "helper present / older / absent");
- unit and loopback tests of the host logic itself;
- the base the later phases (hook, daemon, push) build on.

Not in P0: the hook, any resident process, push, telemetry/file-browser/git ports (they stay shell
or Python and keep working), Linux (macOS only, owner decision).

## 2. Design

### 2.1 Binary

New workspace crate `crates/host`, binary `ssh-app-host`, std plus `serde`/`serde_json`/`sha2`/`hex`
only (no async runtime, no network). Built for `aarch64-apple-darwin` (the owner's laptop);
`x86_64-apple-darwin` is added when its target is installed. Release profile: `opt-level = "s"`,
LTO, `panic = "abort"`, stripped.

Subcommands (stdout is one JSON object; diagnostics go to stderr; exit 0 on a handled result):

| Command | Purpose |
|---|---|
| `hello` | `{"v":1,"name":"ssh-app-host","version":"0.1.0","caps":["usage"]}` |
| `usage [CONFIG_DIR...]` | what `usage_probe.py` prints today, field for field (sessions with agent, model, context, limit note, plan limits; Ollama load; 5 h and 7 d token totals; `partial`) |

Arguments are the same validated config directories the app already passes; the helper expands `~`
itself and refuses anything with control characters. There is **no** subcommand that executes
anything, writes files or takes a free-form path other than those config directories.

### 2.2 Usage logic

A direct port of `crates/core/src/usage_probe.py` (the reference; its behaviour is the spec):
process tree and tmux panes via `ps`/`tmux` (run with a private `PATH` addition for Homebrew),
agent detection (claude, codex, agy, grok, `ollama run`), Claude transcript state (newest
transcript for the pane's folder, last assistant usage, limit notice in the last 6 h), plan limits
recorded by the status-line wrapper (`~/.ssh-app/run/limits`), Codex limits from its rollout
files, Antigravity state from its local transcript, Ollama `/api/ps` on `127.0.0.1:11434` (plain
HTTP over a `TcpStream`, 2 s timeout), rolling 5 h / 7 d totals with a 15 s budget and the same
de-duplication key. The tmux calls honour `$TMUX_TMPDIR`/`$TMUX` exactly as the Python did (the
tests pass a private socket through the environment, see section 5).

### 2.3 Installation and trust

- The app bundles the helper binary (a resource built by `make host-helper`); the core receives its
  bytes and its SHA-256 from the app. The hash is therefore pinned by the app build, not read from
  the host.
- `install_host_helper(bytes, sha256)` writes it to `~/.ssh-app/bin/ssh-app-host` with the same
  guarded writer as the hook script (temp file, `chmod 700`, atomic `mv`, never through a symlink),
  then runs `shasum -a 256` on the host and refuses to keep it when the digest differs.
- Every use calls `host_helper_status()`: it runs `sha256` of the installed file and compares with
  the pinned hash; only an exact match is ever executed. A different file (old version, tampered)
  reports `outdated`/`mismatch` and the app falls back to the Python probe.
- The helper never updates itself and never touches the network except `127.0.0.1:11434`.

### 2.4 Core and app

- Core: `fetch_host_usage` tries `~/.ssh-app/bin/ssh-app-host usage ...` when the status is `ok`
  (hash matches), else runs the embedded Python exactly as now. The result type is unchanged.
- API (UniFFI): `host_helper_status(expected_sha256) -> HostHelperStatus { Absent, Mismatch,
  Ok { version } }`, `install_host_helper(bytes, sha256)`.
- iOS: a "Host service" row in the server editor / Settings shows `not installed / ok / needs
  update` with an Install/Update button (preview of path and hash first). Nothing is installed
  automatically.

## 3. Security

- The agent runs as the same user and can replace the file: the pinned-hash check on every use
  means a replaced helper is **not executed** by the app (fallback to Python). It cannot be made to
  run by the app, which is the property that matters; it could still be run by the agent itself,
  which is no new capability.
- Arguments: only validated config directories; the helper rejects control characters and caps
  count (8) and length (300).
- Output is parsed by the existing strict parser with the existing caps; unknown fields ignored.
- The binary opens no listening socket, starts no process other than `ps` and `tmux` (fixed argv,
  no shell), reads only transcript/limit/rollout files under the agents' directories and the pane's
  folder (never writes).
- Reading files: regular files only (`metadata().is_file()` after open), size caps, a time budget,
  as in the audit fixes of 2026-10-18 (a FIFO must not hang the helper).

## 4. Failure behaviour

Helper missing, wrong hash, crashes or prints invalid JSON: the app uses the Python probe, so the
usage strip never depends on the helper. A helper that exceeds the host-side watchdog (30 s, as
today) is killed and the fallback runs.

## 5. Tests

- Host crate: unit tests per module (agent detection regexes, transcript parsing, window totals,
  limits recording/rollover, Codex and agy readers, JSON shape equals the Python's on the same
  fixtures).
- Parity test: run `usage_probe.py` and `ssh-app-host usage` on the same fixture tree (temp
  `HOME`, config dirs, no tmux) and compare the JSON (allowing order differences).
- Loopback: install over the throwaway sshd (hash verified, tampered file refused, fallback),
  `hello`, and a private-socket tmux session running a fake `claude` process (per `AGENTS.md`
  test isolation: `tmux -S`, `TMUX` removed, real tmux pid compared before and after).
- Swift: status mapping and the Install row compile-tested.

## 6. Exit criteria

Parity test green; loopback install/fallback green; the sessions/usage path works with the helper
absent; no change to the wire types. Deleting the Python probe waits until the helper has run on
the owner's device (live checks are postponed), so P0 keeps both.

## 7. Review notes (rev, 2026-10-18)

Named trade-offs, not defects:
- Porting ~330 lines of working Python is a risk with no new user-visible feature in P0; it is
  accepted because the later phases (hook, daemon) need the typed binary and the install/trust
  path exists first, and the parity test pins the behaviour. Python stays as the fallback.
- The hash is checked and the file then executed in one host command; a same-user attacker could
  swap the file between the two. That attacker can already run anything as the user, so the check
  protects against stale or accidentally replaced helpers, not against a same-user adversary
  (stated in section 3).
- `x86_64` is not built until its Rust target is installed; the owner's host is Apple silicon.
- `regex` is a dependency of the helper (already used by the core) to keep the agent-detection
  patterns identical to the Python ones.

## 8. Implementation notes (2026-10-18)

Done: `crates/host` (`hello`, `usage`; `timefmt`, `usage` modules; lib + bin), parity test against
`usage_probe.py` (`crates/host/tests/parity.rs`), core: `build_usage_command(dirs, pin)` with Python
fallback, `host_helper_status`, `install_host_helper`, `install_bundled_host_helper`,
`bundled_host_helper_sha256/size`; the helper is **embedded in the core** (`build.rs` reads
`crates/core/embedded/ssh-app-host`, produced by `scripts/build-xcframework.sh`; an app resource
would be an unsigned Mach-O in the bundle) so the pin travels with the library; app: "Host service"
card in the host monitor sheet; loopback test (`usage_report_...`) installs the helper over a
throwaway sshd, gets the same report as Python, and shows a replaced helper is not run.
Security pass: `docs/security-findings/2026-10-18-host-helper-p0.md`.

Deviation: telemetry stays a shell script (its parser lives in the core); only usage moved in P0.
Python is kept as the fallback; deleting it waits for a device run (live checks are postponed).
